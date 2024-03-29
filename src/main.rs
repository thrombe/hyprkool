use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

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
    /// remember what workspace was last focused on an activity
    pub remember_activity_focus: bool,

    /// how long to wait for ipc responses before executing the command in ms
    pub ipc_timeout: u32,

    pub mouse: MouseConfig,
}
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            remember_activity_focus: true,
            ipc_timeout: 300,
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
    Command(Command),
}
impl Message {
    fn msg(&self) -> Vec<u8> {
        serde_json::to_string(self).unwrap().into_bytes()
    }
}

#[derive(Subcommand, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Command {
    Info {
        #[command(subcommand)]
        command: InfoCommand,

        #[arg(long, short, default_value_t = false)]
        monitor: bool,
    },
    MouseLoop,
    IpcListen,
    IpcQuit,
    RememberActivityWorkspace,
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
    async fn execute(self, state: Arc<tokio::sync::Mutex<State>>, stateful: bool) -> Result<()> {
        let mut state = state.lock().await;

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

impl InfoCommand {
    async fn execute(self, state: State, monitor: bool) -> Result<()> {
        let mut ael = EventListener::new();

        match self {
            InfoCommand::WaybarActivityStatus => {
                fn print_state(state: &State, name: &str) {
                    state
                        .get_activity_status_repr(name)
                        .into_iter()
                        .for_each(|a| {
                            println!(
                                "{}",
                                serde_json::to_string(&WaybarText { text: a })
                                    .expect("it will work")
                            );
                        });
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(&state, &workspace.name);

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        print_state(&state, &name);
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            InfoCommand::WaybarActiveWindow => {
                let windows = Arc::new(Mutex::new(Clients::get_async().await?));

                let ws = windows.clone();
                let print_status = move |addr: Option<Address>| {
                    let mut ws = ws.lock().expect("could not read windows");
                    let Some(addr) = addr else {
                        let w = WaybarText {
                            text: "Hyprland".to_owned(),
                        };
                        println!("{}", serde_json::to_string(&w).unwrap());
                        return;
                    };

                    let mut w = ws.iter().find(|w| w.address == addr).cloned();
                    if w.is_none() {
                        *ws = Clients::get().expect("could not get windows");
                        w = ws.iter().find(|w| w.address == addr).cloned();
                    }

                    println!(
                        "{}",
                        serde_json::to_string(&WaybarText {
                            text: w.map(|w| w.initial_title).unwrap()
                        })
                        .unwrap()
                    );
                };

                let addr = Client::get_active_async().await?.map(|w| w.address);
                print_status(addr);

                ael.add_active_window_change_handler(move |e| {
                    print_status(e.map(|e| e.window_address));
                });
            }
            InfoCommand::Submap => {
                if !monitor {
                    println!("'info submap' not supported without --monitor");
                    return Ok(());
                }
                ael.add_sub_map_change_handler(|submap| {
                    println!("{{\"submap\":\"{submap}\"}}");
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

                let ws = window_states.clone();
                let print_state = move |e: Option<WindowEventData>| {
                    let workspace =
                        Workspace::get_active().expect("could not get active workspace");
                    let Some(e) = e else {
                        let w = WindowStatus {
                            title: "Hyprland".to_owned(),
                            initial_title: "Hyprland".to_owned(),
                            class: "Hyprland".to_owned(),
                            address: "0x0".to_string(),
                            workspace: workspace.name,
                            icon: PathBuf::new(),
                        };
                        println!("{}", serde_json::to_string(&w).unwrap());
                        return;
                    };
                    let mut ws = ws.lock().expect("could not read windows");
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
                    println!("{}", serde_json::to_string(&w).unwrap());
                };

                let w = Client::get_active_async().await?.map(|w| WindowEventData {
                    window_class: w.class,
                    window_title: w.title,
                    window_address: w.address,
                });
                print_state(w);

                ael.add_active_window_change_handler(move |e| {
                    print_state(e);
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

                let ws = window_states.clone();
                let print_status = move |name: &str, except: Option<Address>| {
                    let mut ws = ws.lock().expect("could not get lock");
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

                    println!("{}", serde_json::to_string(&wds).unwrap());
                };

                let w = Workspace::get_active()?;
                print_status(&w.name, None);

                let ws = window_states.clone();
                let ps = print_status.clone();
                ael.add_window_open_handler(move |_| {
                    let mut ws = ws.lock().expect("could not get lock");
                    ws.windows = Clients::get().unwrap().to_vec();
                    drop(ws);

                    let w = Workspace::get_active().expect("could not get active workspace");
                    ps(&w.name, None);
                });
                let ws = window_states.clone();
                let ps = print_status.clone();
                ael.add_window_moved_handler(move |_| {
                    let mut ws = ws.lock().expect("could not get lock");
                    ws.windows = Clients::get().unwrap().to_vec();
                    drop(ws);

                    let w = Workspace::get_active().expect("could not get active workspace");
                    ps(&w.name, None);
                });
                let ps = print_status.clone();
                let ws = window_states.clone();
                ael.add_window_close_handler(move |addr| {
                    let mut ws = ws.lock().expect("could not get lock");
                    ws.windows.retain(|w| w.address != addr);
                    drop(ws);

                    let w = Workspace::get_active().expect("could not get active workspace");
                    ps(&w.name, Some(addr));
                });

                let ps = print_status.clone();
                ael.add_workspace_change_handler(move |e| {
                    let name = match &e {
                        WorkspaceType::Regular(name) => name.as_str(),
                        WorkspaceType::Special(name) => {
                            name.as_ref().map(|s| s.as_str()).unwrap_or("special")
                        }
                    };
                    ps(name, None);
                });
            }
            InfoCommand::Workspaces => {
                fn print_state(state: &State, name: &str) {
                    let Some((activity_index, Some(workspace_index))) = state.get_indices(name)
                    else {
                        return;
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

                    println!("{}", serde_json::to_string(&activity).unwrap());
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(&state, &workspace.name);

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        print_state(&state, &name);
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            // TODO: maybe this can make InfoCommand::Workspace obsolete.
            // need to add more fields tho. (currectly focused activity)
            InfoCommand::AllWorkspaces => {
                fn print_state(state: &State, name: &str) {
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
                            if w == name {
                                ws.focus = true;
                            }
                            wss.push(ws);
                        }
                        activity.push(wss);
                        activities.push(activity);
                    }

                    println!("{}", serde_json::to_string(&activities).unwrap());
                }

                let workspace = Workspace::get_active_async().await?;
                print_state(&state, &workspace.name);

                ael.add_workspace_change_handler(move |e| match e {
                    WorkspaceType::Regular(name) => {
                        print_state(&state, &name);
                    }
                    WorkspaceType::Special(..) => {}
                });
            }
            InfoCommand::Activities => {
                let ws = Workspace::get_active_async().await?;
                let Some(w) = ws.name.split(':').next() else {
                    return Ok(());
                };

                let print_state = move |w: &str| {
                    let acs = state
                        .activities
                        .iter()
                        .map(|name| ActivityStatus {
                            name: name.into(),
                            focus: w == name,
                        })
                        .collect::<Vec<_>>();
                    println!("{}", serde_json::to_string(&acs).unwrap());
                };
                print_state(w);

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
                    print_state(w);
                });
            }
        }

        if monitor {
            ael.start_listener_async().await?;
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.clone() {
        Command::MouseLoop => {
            let state = State::new(cli.config()?);

            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&state.workspaces[0][0]),
            ))
            .await?;

            // TODO: multi monitor setup yaaaaaaaaaaaaaaaaa
            let monitor = Monitor::get_active_async().await?;
            let w = state.config.daemon.mouse.edge_width as i64;
            let m = state.config.daemon.mouse.edge_margin as i64;

            loop {
                tokio::time::sleep(std::time::Duration::from_millis(
                    state.config.daemon.mouse.polling_rate,
                ))
                .await;
                let nx = state.config.workspaces.0 as usize;
                let ny = state.config.workspaces.1 as usize;
                let mut c = CursorPosition::get_async().await?;
                let mut y = 0;
                let mut x = 0;
                if c.x <= w {
                    x += nx - 1;
                    c.x = monitor.width as i64 - m;
                } else if c.x >= monitor.width as i64 - 1 - w {
                    x += 1;
                    c.x = m;
                }
                if c.y <= w {
                    y += ny - 1;
                    c.y = monitor.height as i64 - m;
                } else if c.y >= monitor.height as i64 - 1 - w {
                    y += 1;
                    c.y = m;
                }

                if x + y == 0 {
                    continue;
                }

                let workspace = Workspace::get_active_async().await?;
                let Some((current_activity_index, Some(current_workspace_index))) =
                    state.get_indices(&workspace.name)
                else {
                    println!("unknown workspace {}", workspace.name);
                    continue;
                };

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
        Command::Info { command, monitor } => {
            command.execute(State::new(cli.config()?), monitor).await?;
        }
        Command::RememberActivityWorkspace => {
            // - [Unix sockets, the basics in Rust - Emmanuel Bosquet](https://emmanuelbosquet.com/2022/whatsaunixsocket/)
            let sock_path = "/tmp/hyprkool.sock";

            if std::fs::metadata(sock_path).is_ok() {
                println!("A socket is already present. Deleting...");
                std::fs::remove_file(sock_path).with_context(|| {
                    format!("could not delete previous socket at {:?}", sock_path)
                })?;
            }

            let sock = UnixListener::bind(sock_path)?;
            let state = Arc::new(tokio::sync::Mutex::new(State::new(cli.config()?)));
            loop {
                match sock.accept().await {
                    Ok((stream, _addr)) => {
                        let mut sock = BufReader::new(stream);
                        let mut line = String::new();
                        sock.read_line(&mut line).await?;
                        let message = serde_json::from_str::<Message>(&line)?;
                        match message {
                            Message::Command(Command::IpcQuit) => {
                                break;
                            }
                            Message::Command(command) => {
                                match command.execute(state.clone(), true).await {
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
        comm => {
            if let Ok(sock) = UnixStream::connect("/tmp/hyprkool.sock").await {
                let mut sock = BufWriter::new(sock);
                sock.write_all(&Message::Command(comm.clone()).msg())
                    .await?;
                sock.flush().await?;
                sock.shutdown().await?;

                let sleep = tokio::time::sleep(Duration::from_secs_f32(0.3));
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
                                println!("falling back to stateless commands");
                            }
                            _ => {
                                unreachable!();
                            }
                        }
                    }
                    _ = sleep => {
                        println!("timeout. could not connect to hyprkool. falling back to stateless commands");
                    }
                }
            }

            let state = State::new(cli.config()?);
            comm.execute(Arc::new(tokio::sync::Mutex::new(state)), false)
                .await?;
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
