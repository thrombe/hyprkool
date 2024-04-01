use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Context, Result};
use clap::{arg, Subcommand};
use hyprland::{
    data::{Client, Clients, Workspace},
    event_listener::{EventListener, WindowEventData},
    shared::{
        Address, HyprData, HyprDataActive, HyprDataActiveOptional, HyprDataVec, WorkspaceType,
    },
};
use linicon::IconPath;
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
};

use crate::{Message, State};

#[derive(Subcommand, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum InfoCommand {
    WaybarActivityStatus,
    WaybarActiveWindow,

    Submap,
    Activities,
    Workspaces,
    AllWorkspaces,
    ActiveWindow {
        /// try to find smallest icon bigger/equal to this size in px
        /// default is 0
        /// returns the biggest found size if none is bigger than/equal to the specified size
        #[arg(long, short = 's', default_value_t = 0)]
        try_min_size: u16,

        /// default value is the current icon theme
        /// will use fallback theme is this is not found
        #[arg(long, short)]
        theme: Option<String>,
    },
    ActiveWorkspaceWindows {
        /// try to find smallest icon bigger/equal to this size in px
        /// default is 0
        /// returns the biggest found size if none is bigger than/equal to the specified size
        #[arg(long, short = 's', default_value_t = 0)]
        try_min_size: u16,

        /// default value is the current icon theme
        /// will use fallback theme is this is not found
        #[arg(long, short)]
        theme: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct InfoOutput {
    stream: InfoOutputStream,
    tx: Sender<()>,
}
impl InfoOutput {
    fn new(stream: InfoOutputStream) -> (Self, Receiver<()>) {
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        (Self { stream, tx }, rx)
    }
    async fn send_mesg(&self, mesg: String) -> Result<()> {
        self.stream.send_mesg(mesg, self.tx.clone()).await
    }
}

#[derive(Clone, Debug)]
pub enum InfoOutputStream {
    Stream(Arc<Mutex<UnixStream>>),
    Stdout,
}
impl InfoOutputStream {
    async fn _send_mesg(stream: &Arc<Mutex<UnixStream>>, mesg: String) -> Result<()> {
        let mut stream = stream.lock().await;
        stream.write_all(&Message::IpcMessage(mesg).msg()).await?;
        stream.write_all("\n".as_bytes()).await?;
        stream.flush().await?;
        Ok(())
    }

    async fn send_mesg(&self, mesg: String, tx: Sender<()>) -> Result<()> {
        match self {
            InfoOutputStream::Stream(s) => {
                if Self::_send_mesg(s, mesg).await.is_err() {
                    tx.send(()).await?;
                }
            }
            InfoOutputStream::Stdout => {
                println!("{}", mesg);
            }
        }
        Ok(())
    }
}

impl InfoCommand {
    pub async fn execute(
        self,
        stream: InfoOutputStream,
        state: Arc<Mutex<State>>,
        monitor: bool,
    ) -> Result<()> {
        let mut ael = EventListener::new();
        let (stream, mut exit) = InfoOutput::new(stream);

        match self {
            InfoCommand::WaybarActivityStatus => {
                async fn print_state(
                    state: Arc<Mutex<State>>,
                    name: String,
                    stream: InfoOutput,
                ) -> Result<()> {
                    let state = state.lock().await;
                    for a in state.get_activity_status_repr(&name).into_iter() {
                        let msg = serde_json::to_string(&WaybarText { text: a })?;
                        stream.send_mesg(msg).await?;
                    }
                    Ok(())
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(state.clone(), workspace.name, stream.clone()).await?;

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        tokio::spawn(print_state(state.clone(), name, stream.clone()));
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            InfoCommand::WaybarActiveWindow => {
                let windows = Arc::new(Mutex::new(Clients::get_async().await?));

                async fn print_status(
                    stream: InfoOutput,
                    addr: Option<Address>,
                    ws: Arc<Mutex<Clients>>,
                ) -> Result<()> {
                    let mut ws = ws.lock().await;
                    let Some(addr) = addr else {
                        let w = WaybarText {
                            text: "Hyprland".to_owned(),
                        };
                        let msg = serde_json::to_string(&w)?;
                        stream.send_mesg(msg).await?;
                        return Ok(());
                    };

                    let mut w = ws.iter().find(|w| w.address == addr).cloned();
                    if w.is_none() {
                        *ws = Clients::get_async().await?;
                        w = ws.iter().find(|w| w.address == addr).cloned();
                    }

                    let msg = serde_json::to_string(&WaybarText {
                        text: w.map(|w| w.initial_title).unwrap(),
                    })?;

                    stream.send_mesg(msg).await?;
                    Ok(())
                }

                let addr = Client::get_active_async().await?.map(|w| w.address);
                print_status(stream.clone(), addr, windows.clone()).await?;

                ael.add_active_window_change_handler(move |e| {
                    tokio::spawn(print_status(
                        stream.clone(),
                        e.map(|e| e.window_address),
                        windows.clone(),
                    ));
                });
            }
            InfoCommand::Submap => {
                if !monitor {
                    println!("'info submap' not supported without --monitor");
                    return Ok(());
                }
                let stream = stream.clone();
                ael.add_sub_map_change_handler(move |submap| {
                    let msg = format!("{{\"submap\":\"{}\"}}", submap);
                    let stream = stream.clone();
                    tokio::spawn(async move {
                        let stream = stream.clone();
                        stream.send_mesg(msg).await
                    });
                });
            }
            InfoCommand::ActiveWindow {
                theme,
                try_min_size,
            } => {
                let window_states = Arc::new(Mutex::new(WindowStates::new(
                    Clients::get_async().await?.to_vec(),
                    theme,
                    try_min_size,
                )?));

                async fn print_state(
                    stream: InfoOutput,
                    e: Option<WindowEventData>,
                    ws: Arc<Mutex<WindowStates>>,
                ) -> Result<()> {
                    let workspace = Workspace::get_active_async().await?;
                    let Some(e) = e else {
                        let w = WindowStatus {
                            title: "Hyprland".to_owned(),
                            initial_title: "Hyprland".to_owned(),
                            class: "Hyprland".to_owned(),
                            address: "0x0".to_string(),
                            workspace: workspace.name,
                            icon: PathBuf::new(),
                        };
                        let msg = serde_json::to_string(&w)?;
                        stream.send_mesg(msg).await?;
                        return Ok(());
                    };
                    let mut ws = ws.lock().await;
                    let w = ws
                        .get_window(e.window_address.clone())
                        .ok()
                        .unwrap_or_else(|| WindowStatus {
                            title: e.window_title.clone(),
                            initial_title: e.window_title,
                            class: e.window_class.clone(),
                            address: e.window_address.to_string(),
                            workspace: workspace.name,
                            icon: ws.get_default_app_icon().unwrap_or_default(),
                        });
                    let mesg = serde_json::to_string(&w)?;
                    stream.send_mesg(mesg).await?;
                    Ok(())
                }

                let w = Client::get_active_async().await?.map(|w| WindowEventData {
                    window_class: w.class,
                    window_title: w.title,
                    window_address: w.address,
                });
                print_state(stream.clone(), w, window_states.clone()).await?;

                ael.add_active_window_change_handler(move |e| {
                    tokio::spawn(print_state(stream.clone(), e, window_states.clone()));
                });
            }
            InfoCommand::ActiveWorkspaceWindows {
                theme,
                try_min_size,
            } => {
                let window_states = Arc::new(Mutex::new(WindowStates::new(
                    Clients::get_async().await?.to_vec(),
                    theme,
                    try_min_size,
                )?));

                async fn print_status(
                    stream: InfoOutput,
                    name: String,
                    except: Option<Address>,
                    ws: Arc<Mutex<WindowStates>>,
                ) -> Result<()> {
                    let mut ws = ws.lock().await;
                    let wds = ws
                        .windows
                        .iter()
                        .filter(|w| w.workspace.name == name)
                        .map(|w| w.address.clone())
                        .filter(|w| except.as_ref().map(|e| w != e).unwrap_or(true))
                        .collect::<Vec<_>>()
                        .into_iter()
                        .filter_map(|w| ws.get_window(w).ok())
                        .collect::<Vec<_>>();

                    let msg = serde_json::to_string(&wds)?;
                    stream.send_mesg(msg).await?;
                    Ok(())
                }

                let w = Workspace::get_active_async().await?;
                print_status(stream.clone(), w.name, None, window_states.clone()).await?;

                let ws = window_states.clone();
                let s = stream.clone();
                ael.add_window_open_handler(move |_| {
                    let ws = ws.clone();
                    let s = s.clone();
                    tokio::spawn(async move {
                        let ws = ws.clone();
                        {
                            let mut ws = ws.lock().await;
                            ws.windows = Clients::get_async().await?.to_vec();
                        }
                        let w = Workspace::get_active_async().await?;
                        print_status(s, w.name, None, ws.clone()).await?;
                        Result::<()>::Ok(())
                    });
                });
                let ws = window_states.clone();
                let s = stream.clone();
                ael.add_window_moved_handler(move |_| {
                    let ws = ws.clone();
                    let s = s.clone();
                    tokio::spawn(async move {
                        {
                            let mut ws = ws.lock().await;
                            ws.windows = Clients::get_async().await?.to_vec();
                        }
                        let w = Workspace::get_active_async().await?;
                        print_status(s, w.name, None, ws).await?;
                        Result::<()>::Ok(())
                    });
                });
                let ws = window_states.clone();
                let s = stream.clone();
                ael.add_window_close_handler(move |addr| {
                    let ws = ws.clone();
                    let s = s.clone();
                    tokio::spawn(async move {
                        {
                            let mut ws = ws.lock().await;
                            ws.windows.retain(|w| w.address != addr);
                        }
                        let w = Workspace::get_active_async().await?;
                        print_status(s.clone(), w.name, Some(addr), ws).await?;
                        Result::<()>::Ok(())
                    });
                });

                let ws = window_states.clone();
                ael.add_workspace_change_handler(move |e| {
                    let name = match e {
                        WorkspaceType::Regular(name) => name,
                        WorkspaceType::Special(name) => name.unwrap_or("special".to_owned()),
                    };
                    tokio::spawn(print_status(stream.clone(), name, None, ws.clone()));
                });
            }
            InfoCommand::Workspaces => {
                async fn print_state(
                    stream: InfoOutput,
                    state: Arc<Mutex<State>>,
                    name: String,
                ) -> Result<()> {
                    let state = state.lock().await;
                    let Some((activity_index, Some(workspace_index))) = state.get_indices(name)
                    else {
                        return Ok(());
                    };

                    let mut activity = Vec::new();
                    let nx = state.config.workspaces.0 as usize;
                    let mut wss = Vec::new();
                    let mut focii = HashMap::<String, Vec<String>>::new();
                    state.named_focii.iter().for_each(|(k, v)| {
                        if let Some(fl) = focii.get_mut(v) {
                            fl.push(k.clone());
                        } else {
                            focii.insert(
                                v.clone(),
                                vec![k.clone()],
                            );
                        }
                    });
                    for (i, w) in state.workspaces[activity_index].iter().enumerate() {
                        if i % nx == 0 && i > 0 {
                            activity.push(wss);
                            wss = Vec::new();
                        }
                        let mut ws = WorkspaceStatus {
                            name: w.to_owned(),
                            focused: false,
                            named_focus: focii.get(w).cloned().unwrap_or_default(),
                        };
                        if i == workspace_index {
                            ws.focused = true;
                        }
                        wss.push(ws);
                    }
                    activity.push(wss);

                    let mesg = serde_json::to_string(&activity)?;
                    stream.send_mesg(mesg).await?;
                    Ok(())
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(stream.clone(), state.clone(), workspace.name).await?;

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        tokio::spawn(print_state(stream.clone(), state.clone(), name));
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            // TODO: maybe this can make InfoCommand::Workspace obsolete.
            // need to add more fields tho. (currectly focused activity)
            InfoCommand::AllWorkspaces => {
                async fn print_state(
                    stream: InfoOutput,
                    state: Arc<Mutex<State>>,
                    name: String,
                ) -> Result<()> {
                    let state = state.lock().await;
                    let mut activities = Vec::new();
                    let mut focii = HashMap::<String, Vec<String>>::new();
                    state.named_focii.iter().for_each(|(k, v)| {
                        if let Some(fl) = focii.get_mut(v) {
                            fl.push(k.clone());
                        } else {
                            focii.insert(
                                v.clone(),
                                vec![k.clone()],
                            );
                        }
                    });
                    for i in 0..state.activities.len() {
                        let mut activity = Vec::new();
                        let nx = state.config.workspaces.0 as usize;
                        let mut wss = Vec::new();
                        for (i, w) in state.workspaces[i].iter().enumerate() {
                            if i % nx == 0 && i > 0 {
                                activity.push(wss);
                                wss = Vec::new();
                            }
                            let mut ws = WorkspaceStatus {
                                name: w.to_owned(),
                                focused: false,
                                named_focus: focii.get(w).cloned().unwrap_or_default(),
                            };
                            if w == &name {
                                ws.focused = true;
                            }
                            wss.push(ws);
                        }
                        activity.push(wss);
                        activities.push(activity);
                    }

                    let mesg = serde_json::to_string(&activities)?;
                    stream.send_mesg(mesg).await?;
                    Ok(())
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(stream.clone(), state.clone(), workspace.name).await?;

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        tokio::spawn(print_state(stream.clone(), state.clone(), name));
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            InfoCommand::Activities => {
                let ws = Workspace::get_active_async().await?;
                let Some(w) = ws.name.split(':').next() else {
                    return Ok(());
                };

                async fn print_state(
                    stream: InfoOutput,
                    w: String,
                    state: Arc<Mutex<State>>,
                ) -> Result<()> {
                    let state = state.lock().await;
                    let acs = state
                        .activities
                        .iter()
                        .map(|name| ActivityStatus {
                            name: name.into(),
                            focused: &w == name,
                        })
                        .collect::<Vec<_>>();
                    let mesg = serde_json::to_string(&acs)?;
                    stream.send_mesg(mesg).await?;
                    Ok(())
                }
                print_state(stream.clone(), w.to_owned(), state.clone()).await?;

                ael.add_workspace_change_handler(move |e| {
                    let name = match &e {
                        WorkspaceType::Regular(name) => name.as_str(),
                        WorkspaceType::Special(..) => {
                            return;
                        }
                    };

                    let Some(w) = name.split(':').next() else {
                        return;
                    };
                    tokio::spawn(print_state(stream.clone(), w.to_owned(), state.clone()));
                });
            }
        }

        if monitor {
            tokio::select! {
                r = ael.start_listener_async() => {
                    r?;
                }
                _ = exit.recv() => {}
            }
        }

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug)]
struct WaybarText {
    text: String,
}

#[derive(Serialize, Debug)]
struct ActivityStatus {
    name: String,
    focused: bool,
}

#[derive(Serialize, Debug)]
struct WorkspaceStatus {
    name: String,
    focused: bool,
    named_focus: Vec<String>,
}

#[derive(Deserialize, Serialize, Debug)]
struct WindowStatus {
    title: String,
    class: String,
    initial_title: String,
    icon: PathBuf,
    address: String,
    workspace: String,
}

#[derive(Debug)]
struct WindowStates {
    /// windows returned by hyprland
    windows: Vec<Client>,
    /// icons for every searched (app, size) pair
    icons: HashMap<String, IconPath>,
    theme: String,
    try_min_size: u16,
}
impl WindowStates {
    fn new(windows: Vec<Client>, theme: Option<String>, try_min_size: u16) -> Result<Self> {
        let s = Self {
            windows,
            icons: Default::default(),
            theme: theme
                .or_else(linicon::get_system_theme)
                .context("could not get current theme")?,
            try_min_size,
        };
        Ok(s)
    }

    fn get_default_app_icon(&mut self) -> Result<PathBuf> {
        self.get_icon_path("wayland")
    }

    fn get_icon_path(&mut self, class: &str) -> Result<PathBuf> {
        if let Some(icon) = self.icons.get(class) {
            return Ok(icon.path.clone());
        }

        let icons = linicon::lookup_icon(class).from_theme(&self.theme);
        let mut icon = None;
        let mut alt = None;
        for next in icons {
            let next = next?;
            if next.min_size >= self.try_min_size
                && next.min_size
                    < icon
                        .as_ref()
                        .map(|i: &IconPath| i.min_size)
                        .unwrap_or(u16::MAX)
            {
                icon = Some(next);
            } else if next.min_size
                > alt
                    .as_ref()
                    .map(|i: &IconPath| i.min_size)
                    .unwrap_or(u16::MIN)
            {
                alt = Some(next);
            }
        }
        let icon = icon.or(alt).context("could not find an icon")?;
        let path = icon.path.clone();
        self.icons.insert(class.to_owned(), icon);
        Ok(path)
    }

    fn get_window(&mut self, address: Address) -> Result<WindowStatus> {
        let mut w = self.windows.iter().find(|w| w.address == address).cloned();
        if w.is_none() {
            self.windows = Clients::get()?.to_vec();
            w = self.windows.iter().find(|w| w.address == address).cloned();
        }
        let Some(w) = w else {
            return Err(anyhow!("could not find window"));
        };
        if let Some(icon) = self.icons.get(&w.initial_class) {
            return Ok(WindowStatus {
                title: w.title,
                class: w.class,
                initial_title: w.initial_title,
                address: w.address.to_string(),
                workspace: w.workspace.name,
                icon: icon.path.clone(),
            });
        }

        let default_icon = self.get_default_app_icon()?;
        let path = self.get_icon_path(&w.initial_class).unwrap_or(default_icon);

        Ok(WindowStatus {
            title: w.title,
            initial_title: w.initial_title,
            class: w.class,
            address: w.address.to_string(),
            workspace: w.workspace.name,
            icon: path,
        })
    }
}
