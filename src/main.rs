use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, Context, Result};
use clap::{arg, command, Parser, Subcommand};
use hyprland::{
    data::{Client, Clients, CursorPosition, Monitor, Workspace},
    dispatch::{Dispatch, DispatchType, WindowIdentifier, WorkspaceIdentifierWithSpecial},
    event_listener::{EventListener, WindowEventData},
    shared::{
        Address, HyprData, HyprDataActive, HyprDataActiveOptional, HyprDataVec, WorkspaceType,
    },
};
use linicon::IconPath;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{UnixListener, UnixStream},
    select,
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
};

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    fn config(&self) -> Result<Config> {
        let config = self
            .config_dir
            .clone()
            .map(PathBuf::from)
            .or(dirs::config_dir().map(|pb| pb.join("hypr")))
            .map(|pb| pb.join("hyprkool.toml"))
            .map(std::fs::read_to_string)
            .transpose()?
            .map(|s| toml::from_str::<Config>(&s))
            .transpose()?
            .unwrap_or(Config::default());
        match config.workspaces {
            (0, _) | (_, 0) => {
                return Err(anyhow!("Use non zero workspace grid dimentions in config"));
            }
            _ => (),
        }
        Ok(config)
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    // TODO: maybe
    // pub enable: bool,
    /// how long to wait for ipc responses before executing the command in ms
    // pub ipc_timeout: u64,
    pub fallback_commands: bool,

    /// remember what workspace was last focused on an activity
    pub remember_activity_focus: bool,

    pub mouse: MouseConfig,
}
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            remember_activity_focus: true,
            fallback_commands: true,
            mouse: Default::default(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct MouseConfig {
    pub switch_workspace_on_edge: bool,
    /// mouse polling rate in ms
    pub polling_rate: u64,
    /// number of pixels to consider as edge
    pub edge_width: u64,
    /// push cursor inside margin when it loops
    pub edge_margin: u64,
}
impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            switch_workspace_on_edge: true,
            polling_rate: 300,
            edge_width: 0,
            edge_margin: 2,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub activities: Vec<String>,
    /// number of workspaces in x and y dimensions
    pub workspaces: (u32, u32),
    pub daemon: DaemonConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            activities: vec!["default".into()],
            workspaces: (2, 2),
            daemon: Default::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Message {
    IpcOk,
    IpcErr(String),
    IpcMessage(String),
    Command(Command),
}
impl Message {
    fn msg(&self) -> Vec<u8> {
        serde_json::to_string(self).unwrap().into_bytes()
    }
}

#[derive(Subcommand, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Command {
    Daemon {
        #[arg(long, short, default_value_t = false)]
        move_to_hyprkool_activity: bool,
    },
    DaemonQuit,
    Info {
        #[command(subcommand)]
        command: InfoCommand,

        #[arg(long, short, default_value_t = false)]
        monitor: bool,
    },
    FocusWindow {
        #[arg(long, short)]
        address: String,
    },
    MoveRight {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    MoveLeft {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    MoveUp {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    MoveDown {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    NextActivity {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    PrevActivity {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SwitchToActivity {
        /// <activity name>
        #[arg(short, long)]
        name: String,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SwitchToWorkspaceInActivity {
        /// <workspace name>
        #[arg(short, long)]
        name: String,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SwitchToWorkspace {
        /// <activity name>:<workspace name>
        #[arg(short, long)]
        name: String,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    ToggleSpecialWorkspace {
        #[arg(short, long)]
        name: String,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,

        #[arg(short, long, requires("move_window"))]
        silent: bool,
    },
}

impl Command {
    async fn execute(self, state: Arc<Mutex<State>>, stateful: bool) -> Result<()> {
        let mut state = state.lock().await;
        let stateful = state.config.daemon.remember_activity_focus && stateful;

        if stateful {
            let workspace = Workspace::get_active_async().await?;
            let a = match &self {
                Command::SwitchToActivity { name, move_window } => {
                    Some((name.clone(), *move_window))
                }
                Command::NextActivity { cycle, move_window } => {
                    let i = state
                        .activities
                        .iter()
                        .position(|a| workspace.name.starts_with(a))
                        .map(|i| {
                            let mut i = i;
                            let n = state.activities.len();
                            if *cycle {
                                i = (i + 1) % n;
                            } else {
                                i = (i + 1).min(n);
                            }
                            i
                        })
                        .unwrap_or(0);
                    let a = state.activities[i].clone();
                    state.remember_workspace(&workspace);
                    Some((a, *move_window))
                }
                Command::PrevActivity { cycle, move_window } => {
                    let i = state
                        .activities
                        .iter()
                        .position(|a| workspace.name.starts_with(a))
                        .map(|i| {
                            let mut i = i as isize;
                            let n = state.activities.len();
                            if *cycle {
                                i = (n as isize + i - 1) % n as isize;
                            } else {
                                i = (i - 1).max(0);
                            }
                            i as usize
                        })
                        .unwrap_or(0);
                    let a = state.activities[i].clone();
                    Some((a, *move_window))
                }
                _ => None,
            };

            if let Some((a, move_window)) = a {
                if let Some(w) = state.focused.get(&a).cloned() {
                    state.move_to_workspace(&w, move_window).await?;
                    return Ok(());
                }
            }
        }

        match self {
            Command::SwitchToWorkspace { name, move_window } => {
                let (activity_index, workspace_index) =
                    state.get_indices(&name).context("activity not found")?;
                let workspace_index = workspace_index.context("workspace not found")?;
                let new_workspace = &state.workspaces[activity_index][workspace_index];
                state.move_to_workspace(new_workspace, move_window).await?;
            }
            Command::SwitchToWorkspaceInActivity { name, move_window } => {
                let workspace = Workspace::get_active_async().await?;
                let activity_index = state
                    .get_activity_index(&workspace.name)
                    .context("could not get current activity")?;
                let activity = &state.activities[activity_index];
                let new_workspace = format!("{activity}:{name}");
                state.move_to_workspace(&new_workspace, move_window).await?;
            }
            Command::SwitchToActivity {
                mut name,
                move_window,
            } => {
                let workspace = Workspace::get_active_async().await?;
                if let Some(activity_index) = state.get_activity_index(&workspace.name) {
                    let activity = &state.activities[activity_index];
                    let id = workspace
                        .name
                        .strip_prefix(activity)
                        .expect("just checked this");
                    name.push_str(id);
                } else {
                    name.push_str("(1 1)");
                };
                state.move_to_workspace(&name, move_window).await?;
            }
            Command::NextActivity { cycle, move_window } => {
                let workspace = Workspace::get_active_async().await?;
                let activity_index = state.get_activity_index(&workspace.name);
                let new_activity_index = activity_index
                    .map(|i| {
                        let mut i = i;
                        if cycle {
                            i += 1;
                            i %= state.activities.len();
                        } else {
                            i = i.min(state.activities.len() - 1);
                        }
                        i
                    })
                    .unwrap_or(0);
                let id =
                    activity_index.and_then(|i| workspace.name.strip_prefix(&state.activities[i]));
                let mut name = state.activities[new_activity_index].clone();
                if let Some(id) = id {
                    name.push_str(id);
                } else {
                    name = state.workspaces[new_activity_index][0].clone();
                };
                state.remember_workspace(&workspace);
                state.move_to_workspace(&name, move_window).await?;
            }
            Command::PrevActivity { cycle, move_window } => {
                let workspace = Workspace::get_active_async().await?;
                let activity_index = state.get_activity_index(&workspace.name);
                let new_activity_index = activity_index
                    .map(|i| {
                        let mut i = i as isize;
                        if cycle {
                            i += state.activities.len() as isize - 1;
                            i %= state.activities.len() as isize;
                        } else {
                            i = i.max(0);
                        }
                        i as usize
                    })
                    .unwrap_or(0);
                let id =
                    activity_index.and_then(|i| workspace.name.strip_prefix(&state.activities[i]));
                let activity_index = new_activity_index;
                let mut name = state.activities[activity_index].clone();
                if let Some(id) = id {
                    name.push_str(id);
                } else {
                    name = state.workspaces[activity_index][0].clone();
                };
                state.remember_workspace(&workspace);
                state.move_to_workspace(&name, move_window).await?;
            }
            Command::MoveRight { cycle, move_window } => {
                let workspace = state.moved_workspace(1, 0, cycle).await?;
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveLeft { cycle, move_window } => {
                let workspace = state.moved_workspace(-1, 0, cycle).await?;
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveUp { cycle, move_window } => {
                let workspace = state.moved_workspace(0, -1, cycle).await?;
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveDown { cycle, move_window } => {
                let workspace = state.moved_workspace(0, 1, cycle).await?;
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::ToggleSpecialWorkspace {
                name,
                move_window,
                silent,
            } => {
                if !move_window {
                    state.toggle_special_workspace(name).await?;
                    return Ok(());
                }
                let window = Client::get_active_async()
                    .await?
                    .context("No active window")?;
                let workspace = Workspace::get_active_async().await?;

                let special_workspace = format!("special:{}", &name);
                let active_workspace = &workspace.name;

                if window.workspace.name == special_workspace {
                    if silent {
                        let windows = Clients::get_async().await?;
                        let c = windows
                            .iter()
                            .filter(|w| w.workspace.id == window.workspace.id)
                            .count();
                        if c == 1 {
                            // keep focus if moving the last window from special to active workspace
                            state.move_to_workspace(active_workspace, true).await?;
                        } else {
                            state.move_window_to_workspace(active_workspace).await?;
                        }
                    } else {
                        state.move_to_workspace(active_workspace, true).await?;
                    }
                } else {
                    state.move_window_to_special_workspace(name.clone()).await?;
                    if !silent {
                        state.toggle_special_workspace(name).await?;
                    }
                };
            }
            Command::FocusWindow { address } => {
                let windows = Clients::get_async().await?;
                let cursor = CursorPosition::get_async().await?;
                for w in windows {
                    if w.address.to_string() == address {
                        Dispatch::call_async(DispatchType::FocusWindow(WindowIdentifier::Address(
                            w.address,
                        )))
                        .await?;
                        Dispatch::call_async(DispatchType::MoveCursor(cursor.x, cursor.y)).await?;
                        break;
                    }
                }
            }
            _ => {
                return Err(anyhow!("cannot ececute these commands here"));
            }
        }

        Ok(())
    }
}

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
enum InfoOutputStream {
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
    async fn execute(
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
                    for (i, w) in state.workspaces[activity_index].iter().enumerate() {
                        if i % nx == 0 && i > 0 {
                            activity.push(wss);
                            wss = Vec::new();
                        }
                        let mut ws = WorkspaceStatus {
                            name: w.to_owned(),
                            focus: false,
                        };
                        if i == workspace_index {
                            ws.focus = true;
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
                                focus: false,
                            };
                            if w == &name {
                                ws.focus = true;
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
                            focus: &w == name,
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

#[derive(Debug)]
pub struct State {
    pub focused: HashMap<String, String>,
    pub activities: Vec<String>,
    pub workspaces: Vec<Vec<String>>,
    pub config: Config,
}

impl State {
    fn new(config: Config) -> Self {
        let (x, y) = config.workspaces;
        let raw_workspaces = (1..=y).flat_map(|y| (1..=x).map(move |x| (x, y)));
        let mut activities = config.activities.clone();
        if activities.is_empty() {
            activities.push("default".into());
        }
        let cooked_workspaces = activities
            .iter()
            .map(|name| {
                raw_workspaces
                    .clone()
                    .map(|(x, y)| format!("{name}:({x} {y})"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        Self {
            focused: HashMap::new(),
            activities,
            workspaces: cooked_workspaces,
            config,
        }
    }

    fn get_activity_index(&self, name: impl AsRef<str>) -> Option<usize> {
        let name = name.as_ref();
        let activity_index = self.activities.iter().position(|a| name.starts_with(a))?;
        Some(activity_index)
    }

    /// (activity index, workspace index)
    fn get_indices(&self, name: impl AsRef<str>) -> Option<(usize, Option<usize>)> {
        let name = name.as_ref();
        let activity_index = self.get_activity_index(name)?;
        let workspace_index = self.workspaces[activity_index]
            .iter()
            .position(|w| w == name);
        Some((activity_index, workspace_index))
    }

    async fn moved_workspace(&self, x: i64, y: i64, cycle: bool) -> Result<&str> {
        let workspace = Workspace::get_active_async().await?;
        let Some((activity_index, Some(workspace_index))) = self.get_indices(workspace.name) else {
            return Err(anyhow!("Error: not in a valid activity workspace"));
        };
        let nx = self.config.workspaces.0 as i64;
        let ny = self.config.workspaces.1 as i64;
        let mut iy = workspace_index as i64 / nx;
        let mut ix = workspace_index as i64 % nx;
        if cycle {
            ix += x + nx;
            ix %= nx;
            iy += y + ny;
            iy %= ny;
        } else {
            ix += x;
            ix = ix.max(0).min(nx - 1);
            iy += y;
            iy = iy.max(0).min(ny - 1);
        }
        Ok(&self.workspaces[activity_index][(iy * nx + ix) as usize])
    }

    async fn move_to_workspace(&self, name: impl AsRef<str>, move_window: bool) -> Result<()> {
        let name = name.as_ref();
        if move_window {
            Dispatch::call_async(DispatchType::MoveToWorkspace(
                WorkspaceIdentifierWithSpecial::Name(name),
                None,
            ))
            .await?;
        } else {
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(name),
            ))
            .await?;
        }
        Ok(())
    }

    async fn move_window_to_workspace(&self, name: impl AsRef<str>) -> Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Name(name),
            None,
        ))
        .await?;
        Ok(())
    }

    async fn move_window_to_special_workspace(&self, name: impl AsRef<str>) -> Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Special(Some(name)),
            None,
        ))
        .await?;
        Ok(())
    }

    async fn toggle_special_workspace(&self, name: String) -> Result<()> {
        Dispatch::call_async(DispatchType::ToggleSpecialWorkspace(Some(name))).await?;
        Ok(())
    }

    fn get_activity_status_repr(&self, workspace_name: &str) -> Option<String> {
        let Some((activity_index, Some(workspace_index))) = self.get_indices(workspace_name) else {
            return None;
        };

        let mut activity = String::new();
        let nx = self.config.workspaces.0 as usize;
        let n = self.workspaces[activity_index].len();
        for (i, _) in self.workspaces[activity_index].iter().enumerate() {
            if i == 0 {
            } else if i % nx == 0 && i > 0 && i < n {
                activity += "\n";
            } else {
                activity += " ";
            }
            if i == workspace_index {
                activity += "   ";
            } else {
                activity += "███";
            }
        }

        Some(activity)
    }

    fn remember_workspace(&mut self, w: &Workspace) {
        let a = w
            .name
            .split_once(':')
            .and_then(|(w, _)| self.activities.iter().find(|&a| a == w))
            .cloned();
        if let Some(a) = a {
            self.focused.insert(a, w.name.clone());
        }
    }
}

struct MouseDaemon {
    state: Arc<Mutex<State>>,

    // TODO: multi monitor setup yaaaaaaaaaaaaaaaaa
    monitor: Monitor,

    config: Config,
}
impl MouseDaemon {
    async fn new(state: Arc<Mutex<State>>) -> Result<Self> {
        let s = state.lock().await;
        let monitor = Monitor::get_active_async().await?;
        let config = s.config.clone();
        drop(s);

        Ok(Self {
            config,
            monitor,
            state,
        })
    }

    async fn run(&mut self, move_to_hyprkool_activity: bool) -> Result<()> {
        let workspace = Workspace::get_active_async().await?;

        {
            let state = self.state.lock().await;

            if state.get_indices(&workspace.name).is_none() && move_to_hyprkool_activity {
                Dispatch::call_async(DispatchType::Workspace(
                    WorkspaceIdentifierWithSpecial::Name(&state.workspaces[0][0]),
                ))
                .await?;
            };
        }

        let w = self.config.daemon.mouse.edge_width as i64;
        let m = self.config.daemon.mouse.edge_margin as i64;
        let enabled = self.config.daemon.mouse.switch_workspace_on_edge;

        let mut sleep_duration =
            std::time::Duration::from_millis(self.config.daemon.mouse.polling_rate);

        if !enabled {
            sleep_duration = std::time::Duration::from_secs(10000000);
        }

        loop {
            tokio::time::sleep(sleep_duration).await;
            if !enabled {
                continue;
            }

            let nx = self.config.workspaces.0 as usize;
            let ny = self.config.workspaces.1 as usize;
            let mut c = CursorPosition::get_async().await?;
            let mut y = 0;
            let mut x = 0;
            if c.x <= w {
                x += nx - 1;
                c.x = self.monitor.width as i64 - m;
            } else if c.x >= self.monitor.width as i64 - 1 - w {
                x += 1;
                c.x = m;
            }
            if c.y <= w {
                y += ny - 1;
                c.y = self.monitor.height as i64 - m;
            } else if c.y >= self.monitor.height as i64 - 1 - w {
                y += 1;
                c.y = m;
            }

            if x + y == 0 {
                continue;
            }

            let workspace = Workspace::get_active_async().await?;

            let state = self.state.lock().await;

            let Some((current_activity_index, Some(current_workspace_index))) =
                state.get_indices(&workspace.name)
            else {
                println!("unknown workspace {}", workspace.name);
                continue;
            };

            if let Some(window) = Client::get_active_async().await? {
                if window.fullscreen && window.fullscreen_mode == 0 {
                    continue;
                }
            }

            y += current_workspace_index / nx;
            y %= ny;
            x += current_workspace_index % nx;
            x %= nx;

            let new_workspace = &state.workspaces[current_activity_index][y * nx + x];
            if new_workspace != &workspace.name {
                state.move_to_workspace(new_workspace, false).await?;
                Dispatch::call_async(DispatchType::MoveCursor(c.x, c.y)).await?;
            }
        }
    }
}

struct IpcDaemon {
    state: Arc<Mutex<State>>,
    _config: Config,
    sock: UnixListener,
}
impl IpcDaemon {
    async fn new(state: Arc<Mutex<State>>) -> Result<Self> {
        let s = state.lock().await;
        let config = s.config.clone();
        drop(s);

        // - [Unix sockets, the basics in Rust - Emmanuel Bosquet](https://emmanuelbosquet.com/2022/whatsaunixsocket/)
        let sock_path = "/tmp/hyprkool.sock";
        if std::fs::metadata(sock_path).is_ok() {
            println!("A socket is already present. Deleting...");
            std::fs::remove_file(sock_path)
                .with_context(|| format!("could not delete previous socket at {:?}", sock_path))?;
        }

        let sock = UnixListener::bind(sock_path)?;
        Ok(Self {
            sock,
            _config: config,
            state,
        })
    }
    async fn run(&mut self) -> Result<()> {
        loop {
            match self.sock.accept().await {
                Ok((stream, _addr)) => {
                    let mut sock = BufReader::new(stream);
                    let mut line = String::new();
                    sock.read_line(&mut line).await?;
                    let message = serde_json::from_str::<Message>(&line)?;
                    match message {
                        Message::Command(Command::DaemonQuit) => {
                            sock.write_all(&Message::IpcOk.msg()).await?;
                            return Ok(());
                        }
                        Message::Command(Command::Info { command, monitor }) => {
                            tokio::spawn(command.execute(
                                InfoOutputStream::Stream(Arc::new(Mutex::new(sock.into_inner()))),
                                self.state.clone(),
                                monitor,
                            ));
                            // return Ok(());
                            continue;
                        }
                        Message::Command(command) => {
                            match command.execute(self.state.clone(), true).await {
                                Ok(_) => {
                                    sock.write_all(&Message::IpcOk.msg()).await?;
                                }
                                Err(e) => {
                                    sock.write_all(
                                        &Message::IpcErr(format!("error: {:?}", e)).msg(),
                                    )
                                    .await?;
                                }
                            }
                        }
                        _ => {
                            unreachable!();
                        }
                    }
                    sock.flush().await?;
                }
                Err(e) => println!("{:?}", e),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.clone() {
        Command::Daemon {
            move_to_hyprkool_activity,
        } => {
            let state = State::new(cli.config()?);
            let state = Arc::new(Mutex::new(state));
            let mut md = MouseDaemon::new(state.clone()).await?;
            let mut id = IpcDaemon::new(state).await?;

            tokio::select! {
                mouse = md.run(move_to_hyprkool_activity) => {
                    return mouse;
                }
                ipc = id.run() => {
                    ipc?;
                    println!("exiting daemon");
                }
            }
        }
        Command::Info { command, monitor } => {
            if let Ok(sock) = UnixStream::connect("/tmp/hyprkool.sock").await {
                let mut sock = BufWriter::new(sock);
                sock.write_all(
                    &Message::Command(Command::Info {
                        command: command.clone(),
                        monitor,
                    })
                    .msg(),
                )
                .await?;
                sock.flush().await?;
                sock.shutdown().await?;

                let mut sock = BufReader::new(sock);
                loop {
                    let mut line = String::new();
                    let _ = sock.read_line(&mut line).await?;

                    let command = serde_json::from_str(&line)?;
                    match command {
                        Message::IpcMessage(message) => {
                            println!("{}", message);
                        }
                        Message::IpcErr(message) => {
                            println!("{}", message);
                        }
                        _ => {
                            unreachable!();
                        }
                    }
                }
            }

            let config = cli.config()?;
            if !config.daemon.fallback_commands {
                return Ok(());
            }
            dbg!("falling back to stateless commands");

            command
                .execute(
                    InfoOutputStream::Stdout,
                    Arc::new(Mutex::new(State::new(cli.config()?))),
                    monitor,
                )
                .await?;
        }
        comm => {
            if let Ok(sock) = UnixStream::connect("/tmp/hyprkool.sock").await {
                let mut sock = BufWriter::new(sock);
                sock.write_all(&Message::Command(comm.clone()).msg())
                    .await?;
                sock.flush().await?;
                sock.shutdown().await?;

                let sleep = tokio::time::sleep(Duration::from_millis(300));
                let mut sock = BufReader::new(sock);
                let mut line = String::new();
                select! {
                    res = sock.read_line(&mut line) => {
                        res?;
                        let command = serde_json::from_str(&line)?;
                        match command {
                            Message::IpcOk => {
                                println!("Ok");
                                return Ok(());
                            }
                            Message::IpcErr(message) => {
                                println!("{}", message);
                            }
                            _ => {
                                unreachable!();
                            }
                        }
                    }
                    _ = sleep => {
                        println!("timeout. could not connect to hyprkool");
                    }
                }
            }

            let config = cli.config()?;
            if !config.daemon.fallback_commands {
                return Ok(());
            }
            println!("falling back to stateless commands");
            let state = State::new(cli.config()?);
            comm.execute(Arc::new(Mutex::new(state)), false).await?;
        }
    }

    Ok(())
}

#[derive(Deserialize, Serialize, Debug)]
struct WaybarText {
    text: String,
}

#[derive(Serialize, Debug)]
struct ActivityStatus {
    name: String,
    focus: bool,
}

#[derive(Deserialize, Serialize, Debug)]
struct WorkspaceStatus {
    name: String,
    focus: bool,
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
