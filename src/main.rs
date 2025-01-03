use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{arg, command, Parser, Subcommand};
use hyprland::data::Monitor;
use hyprland::data::Monitors;
use hyprland::dispatch::WorkspaceIdentifierWithSpecial;
use hyprland::{
    data::{Client, Clients, CursorPosition, Workspace},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use serde::{Deserialize, Serialize};
use tokio::io::BufWriter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;

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
pub enum MultiMonitorStrategy {
    // all monitors share a common hyprkool workspace (same x y) acitvity:(x y w)
    SeparateWorkspaces,

    // activity:(x y)
    SharedWorkspacesSyncActivities, // m1:a1w1 m2:a2w2 -> m1:a2w1 m2:a2w2 when switching activities
    SharedWorkspacesUnsyncActivities,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub activities: Vec<String>,
    /// number of workspaces in x and y dimensions
    pub workspaces: (i32, i32),
    pub multi_monitor_strategy: MultiMonitorStrategy,
    pub named_focii: HashMap<String, String>,
    pub daemon: DaemonConfig,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            activities: vec!["default".into()],
            workspaces: (2, 2),
            multi_monitor_strategy: MultiMonitorStrategy::SharedWorkspacesUnsyncActivities,
            named_focii: Default::default(),
            daemon: Default::default(),
        }
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
    SwitchNamedFocus {
        /// set current named focus to none if name not provided
        #[arg(short, long)]
        name: String,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SetNamedFocus {
        /// lock current named focus if none
        #[arg(short, long)]
        name: String,
    },
    ToggleOverview,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,

    /// don't use daemon for this command even if one is active (mainly useful for debugging)
    #[arg(long)]
    pub force_no_daemon: bool,
}

impl Cli {
    fn config(&self) -> Result<Config> {
        let config = self
            .config_dir
            .clone()
            .map(PathBuf::from)
            .or(dirs::config_dir().map(|pb| pb.join("hypr")))
            .map(|pb| pb.join("hyprkool.toml"))
            .filter(|p| p.exists())
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

pub fn get_socket_dir() -> Result<PathBuf> {
    let hypr_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("could not get HYPRLAND_INSTANCE_SIGNATURE")?;
    let mut sock_path = PathBuf::from("/tmp/hyprkool");
    sock_path.push(&hypr_signature);
    if std::fs::metadata(&sock_path).is_err() {
        std::fs::create_dir_all(&sock_path)?;
    }
    Ok(sock_path)
}

pub fn get_socket_path() -> Result<PathBuf> {
    let mut sock_path = get_socket_dir()?;
    sock_path.push("kool.sock");
    Ok(sock_path)
}
pub fn get_plugin_socket_path() -> Result<PathBuf> {
    let mut sock_path = get_socket_dir()?;
    sock_path.push("plugin.sock");
    Ok(sock_path)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Animation {
    None = 0,
    Left = 1,
    Right = 2,
    Up = 3,
    Down = 4,
    Fade = 5,
}

// TODO: do all this plugin ipc properly
pub async fn is_plugin_running() -> Result<bool> {
    _send_plugin_event(Animation::None as _).await
}

pub async fn set_workspace_anim(anim: Animation) -> Result<()> {
    _send_plugin_event(anim as _).await?;
    Ok(())
}

async fn _send_plugin_event(e: usize) -> Result<bool> {
    let sock_path = get_plugin_socket_path()?;

    if let Ok(sock) = UnixStream::connect(&sock_path).await {
        let mut sock = BufWriter::new(sock);
        sock.write_all(format!("{}", e).as_bytes()).await?;
        sock.flush().await?;
        sock.shutdown().await?;

        let sleep = tokio::time::sleep(Duration::from_millis(300));
        let mut sock = BufReader::new(sock);
        let mut line = String::new();
        tokio::select! {
            res = sock.read_line(&mut line) => {
                res?;
                let command = match serde_json::from_str(&line) {
                    Ok(c) => c,
                    Err(e) => {
                        println!("{}", e);
                        return Ok(false);
                    }
                };
                match command {
                    Message::IpcOk => {
                        println!("Ok");
                        return Ok(true);
                    }
                    Message::IpcErr(message) => {
                        println!("{}", message);
                        return Ok(false);
                    }
                    _ => {
                        unreachable!();
                    }
                }
            }
            _ = sleep => {
                println!("timeout. could not connect to hyprkool plugin");
            }
        }
    }

    Ok(false)
}

struct State {
    config: Config,
    monitors: Vec<KMonitor>,
}

impl State {
    async fn new(config: Config) -> Result<Self> {
        let m = Monitors::get_async().await?;
        let monitors = m
            .into_iter()
            .map(|m| KMonitor::new(m, &config.activities))
            .collect();

        Ok(Self { config, monitors })
    }

    fn moved_ws(&self, ws: KWorkspace, wrap: bool, x: i32, y: i32) -> KWorkspace {
        if wrap {
            KWorkspace {
                x: ((ws.x - 1 + x + self.config.workspaces.0).max(0) % self.config.workspaces.0)
                    + 1,
                y: ((ws.y - 1 + y + self.config.workspaces.1).max(0) % self.config.workspaces.1)
                    + 1,
            }
        } else {
            KWorkspace {
                x: self.config.workspaces.0.min(ws.x + x).max(1),
                y: self.config.workspaces.1.min(ws.y + y).max(1),
            }
        }
    }

    fn focused_monitor_mut(&mut self) -> &mut KMonitor {
        self.monitors
            .iter_mut()
            .find(|m| m.monitor.focused)
            .expect("no monitor focused")
    }

    async fn move_focused_window_to(&mut self, activity: &str, ws: KWorkspace) -> Result<()> {
        if let Some(_window) = Client::get_active_async().await? {
            Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                WorkspaceIdentifierWithSpecial::Name(&ws.name(activity, false)),
                None,
            ))
            .await?;
        }

        Ok(())
    }

    async fn move_focused_window_to_raw(&mut self, ws: &str) -> Result<()> {
        if let Some(_window) = Client::get_active_async().await? {
            Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                WorkspaceIdentifierWithSpecial::Name(ws),
                None,
            ))
            .await?;
        }

        Ok(())
    }

    async fn move_towards(&mut self, x: i32, y: i32, cycle: bool, move_window: bool) -> Result<()> {
        let (a, ws) = self
            .focused_monitor_mut()
            .current()
            .context("not in a hyprkool workspace")?;
        let ws = self.moved_ws(ws, cycle, x, y);
        if move_window {
            self.move_focused_window_to(&a, ws).await?;
        }
        self.focused_monitor_mut().move_to(a, ws).await?;
        Ok(())
    }

    async fn cycle_activity(&mut self, z: i32, cycle: bool, move_window: bool) -> Result<()> {
        let m = self.focused_monitor_mut();
        let (a, ws) = if let Some((a, ws)) = m.current() {
            let mut ai = m.get_activity_index(&a).context("unknown activity name")? as isize;
            ai += z as isize;
            if cycle {
                ai += self.config.activities.len() as isize;
                ai %= self.config.activities.len() as isize;
            } else {
                ai = ai.min(self.config.activities.len() as isize - 1).max(0);
            }
            let a = self.config.activities[ai as usize].clone();
            (a, ws)
        } else {
            let a = self.config.activities[0].clone();
            let ws = KWorkspace { x: 1, y: 1 };
            (a, ws)
        };
        if move_window {
            self.move_focused_window_to(&a, ws).await?;
        }
        self.focused_monitor_mut().move_to(a, ws).await?;
        Ok(())
    }

    async fn execute(&mut self, command: Command) -> Result<()> {
        match command {
            Command::MoveRight { cycle, move_window } => {
                self.move_towards(1, 0, cycle, move_window).await?;
            }
            Command::MoveLeft { cycle, move_window } => {
                self.move_towards(-1, 0, cycle, move_window).await?;
            }
            Command::MoveUp { cycle, move_window } => {
                self.move_towards(0, -1, cycle, move_window).await?;
            }
            Command::MoveDown { cycle, move_window } => {
                self.move_towards(0, 1, cycle, move_window).await?;
            }
            Command::NextActivity { cycle, move_window } => {
                self.cycle_activity(1, cycle, move_window).await?;
            }
            Command::PrevActivity { cycle, move_window } => {
                self.cycle_activity(-1, cycle, move_window).await?;
            }
            Command::ToggleSpecialWorkspace {
                name,
                move_window,
                silent,
            } => {
                if !move_window {
                    Dispatch::call_async(DispatchType::ToggleSpecialWorkspace(Some(name))).await?;
                    return Ok(());
                }
                let window = Client::get_active_async()
                    .await?
                    .context("No active window")?;

                let special_workspace = format!("special:{}", &name);
                let active_workspace = self
                    .focused_monitor_mut()
                    .monitor
                    .active_workspace
                    .name
                    .clone();

                _ = set_workspace_anim(Animation::Fade).await;
                if window.workspace.name == special_workspace {
                    if silent {
                        let windows = Clients::get_async().await?;
                        let c = windows
                            .iter()
                            .filter(|w| w.workspace.id == window.workspace.id)
                            .count();
                        if c == 1 {
                            // keep focus if moving the last window from special to active workspace
                            self.move_focused_window_to_raw(&active_workspace).await?;
                            Dispatch::call_async(DispatchType::Custom(
                                "focusworkspaceoncurrentmonitor",
                                &active_workspace,
                            ))
                            .await?;
                            Dispatch::call_async(DispatchType::FocusWindow(
                                WindowIdentifier::Address(window.address),
                            ))
                            .await?;
                        } else {
                            Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                                WorkspaceIdentifierWithSpecial::Name(&active_workspace),
                                None,
                            ))
                            .await?;
                        }
                    } else {
                        self.move_focused_window_to_raw(&active_workspace).await?;
                        Dispatch::call_async(DispatchType::Custom(
                            "focusworkspaceoncurrentmonitor",
                            &active_workspace,
                        ))
                        .await?;
                        Dispatch::call_async(DispatchType::FocusWindow(WindowIdentifier::Address(
                            window.address,
                        )))
                        .await?;
                    }
                } else {
                    Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                        WorkspaceIdentifierWithSpecial::Special(Some(&name)),
                        None,
                    ))
                    .await?;
                    if !silent {
                        Dispatch::call_async(DispatchType::ToggleSpecialWorkspace(Some(name)))
                            .await?;
                    }
                };
            }
            Command::ToggleOverview => {
                self.focused_monitor_mut().toggle_overview().await?;
            }
            Command::SwitchToActivity { name, move_window } => {
                if move_window {
                    if let Some((a, ws)) = self.focused_monitor_mut().current() {
                        self.move_focused_window_to(&a, ws).await?;
                    }
                }
                self.focused_monitor_mut().move_to_activity(name).await?;
            }
            Command::FocusWindow { address } => {
                let windows = Clients::get_async().await?;
                for w in windows {
                    if w.address.to_string() == address {
                        Dispatch::call_async(DispatchType::FocusWindow(WindowIdentifier::Address(
                            w.address,
                        )))
                        .await?;
                    }
                }
            }
            Command::SwitchToWorkspaceInActivity { name, move_window } => {
                let (a, _ws) = self
                    .focused_monitor_mut()
                    .current()
                    .context("not in hyprkool activity")?;
                let ws =
                    KWorkspace::from_ws_part_of_name(&name).context("invalid workspace name")?;
                if move_window {
                    self.move_focused_window_to(&a, ws).await?;
                }
                self.focused_monitor_mut().move_to(a, ws).await?;
            }
            Command::SwitchToWorkspace { name, move_window } => {
                let a = KActivity::from_ws_name(&name).context("activity not found")?;
                let ws = KWorkspace::from_ws_name(&name).context("workspace not found")?;
                if move_window {
                    self.move_focused_window_to(&a.name, ws).await?;
                }
                self.focused_monitor_mut().move_to(a.name, ws).await?;
            }
            Command::Daemon {
                move_to_hyprkool_activity,
            } => todo!(),
            Command::DaemonQuit => todo!(),
            Command::Info { command, monitor } => todo!(),
            Command::SwitchNamedFocus { name, move_window } => todo!(),
            Command::SetNamedFocus { name } => todo!(),
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct KMonitor {
    monitor: Monitor,
    activities: Vec<KActivity>,
}

impl KMonitor {
    fn new(m: Monitor, activities: &[String]) -> Self {
        KMonitor {
            activities: activities
                .iter()
                .map(|a| KActivity {
                    name: a.into(),
                    last_workspace: None,
                })
                .collect(),
            monitor: m,
        }
    }

    fn get_activity_index(&self, name: &str) -> Option<usize> {
        self.activities.iter().position(|a| a.name == name)
    }
}

// assuming self is updated with most recent info
impl KMonitor {
    fn current(&self) -> Option<(String, KWorkspace)> {
        let a = KActivity::from_ws_name(&self.monitor.active_workspace.name)?;
        let w = KWorkspace::from_ws_name(&self.monitor.active_workspace.name)?;
        _ = self.activities.iter().find(|ac| ac.name == a.name)?;
        Some((a.name, w))
    }

    async fn move_to_activity(&mut self, activity: String) -> Result<()> {
        if let Some((a, ws)) = self.current() {
            if let Some(ai) = self.get_activity_index(&a) {
                self.activities[ai].last_workspace = Some(ws);
            }

            self.move_to(activity, ws).await?;
        } else {
            self.move_to(activity, KWorkspace { x: 1, y: 1 }).await?;
        }

        Ok(())
    }

    async fn toggle_overview(&mut self) -> Result<()> {
        let (a, ws) = self.current().context("not in a hyprkool activity")?;

        _ = set_workspace_anim(Animation::Fade).await;

        if !self.monitor.focused {
            Dispatch::call_async(DispatchType::Custom(
                "focusmonitor",
                &format!("{}", self.monitor.id),
            ))
            .await?;
        }

        if self.monitor.active_workspace.name.ends_with(":overview") {
            Dispatch::call_async(DispatchType::Custom(
                "focusworkspaceoncurrentmonitor",
                &format!("name:{}", ws.name(&a, false)),
            ))
            .await?;
        } else {
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&ws.name(&a, true)),
            ))
            .await?;
        }

        Ok(())
    }

    async fn move_to(&mut self, activity: String, new_ws: KWorkspace) -> Result<()> {
        let res = if let Some((a, ws)) = self.current() {
            if let Some(ai) = self.get_activity_index(&a) {
                self.activities[ai].last_workspace = Some(ws);
            }

            set_workspace_anim(ws.get_anim(new_ws)).await
        } else {
            set_workspace_anim(Animation::Fade).await
        };

        if !self.monitor.focused {
            Dispatch::call_async(DispatchType::Custom(
                "focusmonitor",
                &format!("{}", self.monitor.id),
            ))
            .await?;
        }
        Dispatch::call_async(DispatchType::Custom(
            "focusworkspaceoncurrentmonitor",
            &format!("name:{}", new_ws.name(&activity, false)),
        ))
        .await?;

        res
    }
}

#[derive(Clone, Debug)]
struct KActivity {
    name: String,
    last_workspace: Option<KWorkspace>,
}

impl KActivity {
    fn from_ws_name(name: &str) -> Option<Self> {
        let (a, _ws) = name.split_once(':')?;
        Some(KActivity {
            name: a.to_owned(),
            last_workspace: None,
        })
    }
}

#[derive(Copy, Clone, Debug)]
struct KWorkspace {
    x: i32,
    y: i32,
}

impl KWorkspace {
    fn from_ws_name(name: &str) -> Option<Self> {
        let (_a, ws) = name.split_once(':')?;
        let ws = ws.split(':').next().unwrap_or(ws);
        Self::from_ws_part_of_name(ws)
    }

    fn from_ws_part_of_name(name: &str) -> Option<Self> {
        let ws = name.strip_prefix("(")?.strip_suffix(")")?;
        let (x, y) = ws.split_once(' ')?;
        let x: i32 = x.parse().ok()?;
        let y: i32 = y.parse().ok()?;
        Some(KWorkspace { x, y })
    }

    fn name(&self, activity: &str, overview: bool) -> String {
        if overview {
            format!("{}:({} {}):overview", activity, self.x, self.y)
        } else {
            format!("{}:({} {})", activity, self.x, self.y)
        }
    }

    fn get_anim(self, other: Self) -> Animation {
        let x = other.x - self.x;
        let y = other.y - self.y;

        if x > 0 && y == 0 {
            return Animation::Right;
        }
        if x < 0 && y == 0 {
            return Animation::Left;
        }
        if x == 0 && y > 0 {
            return Animation::Down;
        }
        if x == 0 && y < 0 {
            return Animation::Up;
        }

        Animation::Fade
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = cli.config()?;

    let mut s = State::new(config).await?;
    s.execute(cli.command).await?;
    Ok(())
}
