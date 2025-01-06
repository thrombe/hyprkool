use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{arg, command, Parser, Subcommand};
use hyprland::data::FullscreenMode;
use hyprland::data::Monitor;
use hyprland::data::Monitors;
use hyprland::dispatch::WorkspaceIdentifierWithSpecial;
use hyprland::event_listener::AsyncEventListener;
use hyprland::shared::Address;
use hyprland::{
    data::{Client, Clients, CursorPosition, Workspace},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use linicon::IconPath;
use serde::{Deserialize, Serialize};
use tokio::io::BufWriter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
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

    /// move monitors to a valid hyprkool activity on daemon init
    /// also moves newly added monitors to a valid hyprkool activity
    pub move_monitors_to_hyprkool_activity: bool,

    pub mouse: MouseConfig,
}
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            move_monitors_to_hyprkool_activity: true,
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

    pub icon_theme: Option<String>,
    pub window_icon_try_min_size: Option<u16>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            activities: vec!["default".into()],
            workspaces: (2, 2),
            multi_monitor_strategy: MultiMonitorStrategy::SharedWorkspacesUnsyncActivities,
            named_focii: Default::default(),
            daemon: Default::default(),
            icon_theme: None,
            window_icon_try_min_size: None,
        }
    }
}

#[derive(Subcommand, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum InfoCommand {
    Submap,

    /// shows all info needed to create widgets for windows, workspaces, activities, monitors
    MonitorsAllInfo {
        /// try to find smallest icon bigger/equal to this size in px
        /// default is 0
        /// returns the biggest found size if none is bigger than/equal to the specified size
        #[arg(long, short = 's')]
        window_icon_try_min_size: Option<u16>,

        /// default value is the current icon theme
        /// will use fallback theme is this is not found
        #[arg(long, short = 't')]
        window_icon_theme: Option<String>,
    },
}

impl InfoCommand {
    async fn fire_events(&self, tx: mpsc::Sender<KEvent>) -> Result<()> {
        match self {
            InfoCommand::MonitorsAllInfo { .. } => {
                tx.send(KEvent::MonitorInfoRequested).await?;
            }
            InfoCommand::Submap => {}
        }

        Ok(())
    }

    #[allow(clippy::single_match)]
    async fn listen(
        &self,
        rx: &mut broadcast::Receiver<KInfoEvent>,
        info_ctx: &Arc<Mutex<InfoCommandContext>>,
    ) -> Result<Option<String>> {
        let event = rx.recv().await?;

        match self {
            InfoCommand::Submap => match event {
                KInfoEvent::Submap(submap_status) => {
                    Ok(Some(serde_json::to_string(&submap_status)?))
                }
                _ => Ok(None),
            },
            InfoCommand::MonitorsAllInfo {
                window_icon_try_min_size,
                window_icon_theme,
            } => match event {
                KInfoEvent::Monitors(mut vec) => {
                    let mut ctx = info_ctx.lock().await;

                    for m in vec.iter_mut() {
                        for a in m.activities.iter_mut() {
                            for w in a.workspaces.iter_mut() {
                                for c in w.windows.iter_mut() {
                                    c.icon = ctx.get_icon_path(
                                        &c.initial_title,
                                        window_icon_theme.as_ref(),
                                        *window_icon_try_min_size,
                                    )?;
                                }
                            }
                        }
                    }

                    Ok(Some(serde_json::to_string(&vec)?))
                }
                _ => Ok(None),
            },
        }
    }

    async fn listen_loop(
        self,
        mut sock: UnixStream,
        tx: mpsc::Sender<KEvent>,
        mut rx: broadcast::Receiver<KInfoEvent>,
        monitor: bool,
        info_ctx: Arc<Mutex<InfoCommandContext>>,
    ) -> Result<()> {
        // NOTE: DO NOT return errors other than socket errors

        if let Err(e) = self.fire_events(tx).await {
            println!("error when firing info events: {e}");
            sock.write_all(&Message::IpcErr(format!("error: {:?}", e)).msg())
                .await?;
            sock.flush().await?;
            return Ok(());
        }

        loop {
            match self.listen(&mut rx, &info_ctx).await {
                Ok(Some(msg)) => {
                    sock.write_all(&Message::IpcMessage(msg).msg()).await?;
                }
                Ok(None) => {}
                Err(e) => {
                    println!("error when listening for info messages: {e}");
                    sock.write_all(&Message::IpcErr(format!("error: {:?}", e)).msg())
                        .await?;
                }
            }
            sock.flush().await?;

            if !monitor {
                break;
            }
        }

        Ok(())
    }
}

#[derive(Subcommand, Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Command {
    Daemon,
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
        let mut bytes = serde_json::to_string(self).unwrap().into_bytes();
        bytes.extend_from_slice(b"\n");
        bytes
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

    async fn update_monitors(&mut self) -> Result<()> {
        let monitors = Monitors::get_async().await?.into_iter().collect::<Vec<_>>();

        let mut known = HashSet::new();

        for m in self.monitors.iter() {
            known.insert(m.monitor.name.clone());
        }
        for m in monitors.iter() {
            if !known.contains(&m.name) {
                self.monitors
                    .push(KMonitor::new(m.clone(), &self.config.activities));
            }
        }

        for m in monitors {
            for mm in self.monitors.iter_mut() {
                if mm.monitor.id == m.id {
                    mm.monitor = m;
                    break;
                }
            }
        }

        Ok(())
    }

    async fn move_monitor_to_valid_activity(
        &mut self,
        name: &str,
        move_window: bool,
    ) -> Result<()> {
        let cursor = CursorPosition::get_async().await?;

        let mut taken = HashSet::new();

        for m in self.monitors.iter() {
            if m.monitor.disabled {
                if m.monitor.name == name {
                    return Ok(());
                }
                continue;
            }
            taken.insert(m.monitor.active_workspace.name.clone());
            if m.monitor.name == name && m.current().is_some() {
                return Ok(());
            }
        }

        'outer: for a in self.config.activities.iter() {
            for x in 0..self.config.workspaces.0 {
                for y in 0..self.config.workspaces.1 {
                    let ws = KWorkspace { x, y };
                    if taken.contains(&ws.name(a, false)) {
                        continue;
                    }
                    for m in self.monitors.iter_mut() {
                        if m.monitor.name != name {
                            continue;
                        }
                        m.move_to(a.into(), ws, move_window).await?;
                        break 'outer;
                    }
                }
            }
        }

        // focus the monitor that was focused before moving the other monitor to another ws
        if let Some((a, ws)) = self.focused_monitor_mut().current() {
            self.focused_monitor_mut().move_to(a, ws, false).await?;
            Dispatch::call_async(DispatchType::MoveCursor(cursor.x, cursor.y)).await?;
        }

        Ok(())
    }

    async fn move_towards(&mut self, x: i32, y: i32, cycle: bool, move_window: bool) -> Result<()> {
        let (a, ws) = self
            .focused_monitor_mut()
            .current()
            .context("not in a hyprkool workspace")?;
        let ws = self.moved_ws(ws, cycle, x, y);
        let res = KWorkspace::set_anim(x, y).await;
        self.focused_monitor_mut()
            .move_to(a, ws, move_window)
            .await?;
        res
    }

    async fn cycle_activity(&mut self, z: i32, cycle: bool, move_window: bool) -> Result<()> {
        let m = self.focused_monitor_mut();
        let a = if let Some((a, _)) = m.current() {
            let mut ai = m.get_activity_index(&a).context("unknown activity name")? as isize;
            ai += z as isize;
            if cycle {
                ai += self.config.activities.len() as isize;
                ai %= self.config.activities.len() as isize;
            } else {
                ai = ai.min(self.config.activities.len() as isize - 1).max(0);
            }
            self.config.activities[ai as usize].clone()
        } else {
            self.config.activities[0].clone()
        };
        self.focused_monitor_mut()
            .move_to_activity(a, move_window)
            .await?;
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
                _ = set_workspace_anim(Animation::Fade).await;
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

                if window.workspace.name == special_workspace {
                    if silent {
                        let windows = Clients::get_async().await?;
                        let c = windows
                            .iter()
                            .filter(|w| w.workspace.id == window.workspace.id)
                            .count();
                        if c == 1 {
                            // keep focus if moving the last window from special to active workspace
                            self.focused_monitor_mut()
                                .move_focused_window_to_raw(&active_workspace)
                                .await?;
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
                        self.focused_monitor_mut()
                            .move_focused_window_to_raw(&active_workspace)
                            .await?;
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
                self.focused_monitor_mut()
                    .move_to_activity(name, move_window)
                    .await?;
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
                let res = set_workspace_anim(Animation::Fade).await;
                self.focused_monitor_mut()
                    .move_to(a, ws, move_window)
                    .await?;
                res?;
            }
            Command::SwitchToWorkspace { name, move_window } => {
                let a = KActivity::from_ws_name(&name).context("activity not found")?;
                let ws = KWorkspace::from_ws_name(&name).context("workspace not found")?;
                let res = set_workspace_anim(Animation::Fade).await;
                self.focused_monitor_mut()
                    .move_to(a.name, ws, move_window)
                    .await?;
                res?;
            }
            Command::Daemon => todo!(),
            Command::DaemonQuit => todo!(),
            Command::Info { command, monitor } => todo!(),
            Command::SwitchNamedFocus { name, move_window } => todo!(),
            Command::SetNamedFocus { name } => todo!(),
        }

        Ok(())
    }

    async fn update(&mut self, event: KEvent, tx: broadcast::Sender<KInfoEvent>) -> Result<()> {
        self.update_monitors().await?;

        #[allow(clippy::single_match)]
        match &event {
            KEvent::MonitorAdded { name } => {
                if self.config.daemon.move_monitors_to_hyprkool_activity {
                    self.move_monitor_to_valid_activity(name, false).await?;
                }
            }
            _ => {}
        }

        match event {
            KEvent::MonitorInfoRequested
            | KEvent::WindowChange
            | KEvent::WindowOpen
            | KEvent::WindowMoved
            | KEvent::WindowClosed
            | KEvent::MonitorChange
            | KEvent::WorkspaceChange
            | KEvent::MonitorAdded { .. }
            | KEvent::MonitorRemoved { .. } => {
                let clients = Clients::get_async().await?.into_iter().collect::<Vec<_>>();
                tx.send(KInfoEvent::Monitors(self.gather_info(&clients)))?;
            }
            KEvent::Submap { name } => {
                tx.send(KInfoEvent::Submap(SubmapStatus { submap: name }))?;
            }
        }
        Ok(())
    }

    async fn tick(&mut self) -> Result<()> {
        if !self.config.daemon.mouse.switch_workspace_on_edge {
            return Ok(());
        }

        let w = self.config.daemon.mouse.edge_width as i64;
        let m = self.config.daemon.mouse.edge_margin as i64;

        let nx = self.config.workspaces.0 as i64;
        let ny = self.config.workspaces.1 as i64;

        let mut c = CursorPosition::get_async().await?;
        let monitor = self.focused_monitor_mut();

        // OOF:
        // hyprland returns wrong scale.
        // hyprland seems to support only a few scales (ig depending on the screen resolution)
        //  but it returns only 2 decimal places of the scale. which hurts calculation precision.
        //  not sure what to do here.
        let mut scale = monitor.monitor.scale;
        if scale == 0.83 {
            scale = 0.8333333;
        }
        let width = (monitor.monitor.width as f64 / scale as f64) as i64;
        let height = (monitor.monitor.height as f64 / scale as f64) as i64;

        c.x -= monitor.monitor.x as i64;
        c.y -= monitor.monitor.y as i64;

        // dbg!(&c, width, height, scale);

        let mut y: i64 = 0;
        let mut x: i64 = 0;
        let mut anim = Animation::Fade;
        if c.x <= w {
            x += nx - 1;
            c.x = width - m;
            anim = Animation::Left;
        } else if c.x >= width - 1 - w {
            x += 1;
            c.x = m;
            anim = Animation::Right;
        }
        if c.y <= w {
            y += ny - 1;
            c.y = height - m;
            anim = Animation::Up;
        } else if c.y >= height - 1 - w {
            y += 1;
            c.y = m;
            anim = Animation::Down;
        }

        if x + y == 0 {
            return Ok(());
        }

        if x > 0 && y > 0 {
            anim = Animation::Fade;
        }

        c.x += monitor.monitor.x as i64;
        c.y += monitor.monitor.y as i64;

        let Some((a, ws)) = monitor.current() else {
            println!(
                "not in a hyprkool workspace: {}",
                monitor.monitor.active_workspace.name
            );
            return Ok(());
        };

        if let Some(window) = Client::get_active_async().await? {
            // should i use window.fullscreen or window.fullscreen_client ?
            if window.fullscreen as u8 > FullscreenMode::Maximized as u8 {
                return Ok(());
            }
        }

        let new_ws = self.moved_ws(ws, true, x as _, y as _);
        if new_ws != ws {
            _ = set_workspace_anim(anim).await;
            self.focused_monitor_mut().move_to(a, new_ws, false).await?;
            Dispatch::call_async(DispatchType::MoveCursor(c.x, c.y)).await?;
        }
        Ok(())
    }

    fn gather_info(&mut self, clients: &[Client]) -> Vec<MonitorStatus> {
        let mut monitors = vec![];
        for m in self.monitors.clone().iter() {
            let mut activities = vec![];
            for a in m.activities.iter() {
                let mut workspaces = vec![];
                for x in 0..self.config.workspaces.0 {
                    for y in 0..self.config.workspaces.1 {
                        let ws = KWorkspace { x, y };
                        let ws_name = ws.name(&a.name, false);

                        let mut windows = vec![];
                        for client in clients {
                            if client.workspace.name != ws_name {
                                continue;
                            }
                            windows.push(WindowStatus {
                                title: client.title.clone(),
                                class: client.class.clone(),
                                initial_title: client.initial_title.clone(),
                                icon: None,
                                address: client.address.to_string(),
                            });
                        }
                        workspaces.push(WorkspaceStatus {
                            focused: m.monitor.active_workspace.name == ws_name,
                            name: ws_name,
                            // TODO:
                            named_focus: vec![],
                            windows,
                        });
                    }
                }
                activities.push(ActivityStatus {
                    name: a.name.clone(),
                    focused: KActivity::from_ws_name(&m.monitor.active_workspace.name)
                        .map(|ka| ka.name == a.name)
                        .unwrap_or_default(),
                    workspaces,
                });
            }
            monitors.push(MonitorStatus {
                name: m.monitor.name.clone(),
                id: m.monitor.id as _,
                focused: m.monitor.focused,
                activities,
            });
        }

        monitors
    }
}

#[derive(Clone, Debug)]
enum KEvent {
    WindowChange,
    WindowOpen,
    WindowMoved,
    WindowClosed,
    WorkspaceChange,
    MonitorChange,
    MonitorAdded { name: String },
    MonitorRemoved { name: String },
    Submap { name: String },

    MonitorInfoRequested,
}

struct InfoCommandContext {
    config: Config,

    // TODO:
    /// (theme, size, class)
    icons: HashMap<(String, u32, String), PathBuf>,
}

impl InfoCommandContext {
    fn get_icon_path(
        &mut self,
        class: &str,
        theme: Option<&String>,
        window_icon_try_min_size: Option<u16>,
    ) -> Result<Option<PathBuf>> {
        let mut icons = linicon::lookup_icon(class);
        if let Some(theme) = theme {
            icons = icons.from_theme(theme);
        } else if let Some(theme) = self.config.icon_theme.as_ref() {
            icons = icons.from_theme(theme);
        }

        let icon_min_size = window_icon_try_min_size
            .or(self.config.window_icon_try_min_size)
            .unwrap_or(0);
        let mut icon = None;
        let mut alt = None;
        for next in icons {
            let next = next?;
            if next.min_size >= icon_min_size
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
        let mut icon = icon.or(alt).map(|i| i.path);
        if icon.is_none() {
            icon = self.get_icon_path("wayland", theme, window_icon_try_min_size)?;
        }
        Ok(icon)
    }
}

#[derive(Clone, Debug)]
enum KInfoEvent {
    Submap(SubmapStatus),
    Monitors(Vec<MonitorStatus>),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct SubmapStatus {
    submap: String,
}

#[derive(Serialize, Debug, Clone)]
struct MonitorStatus {
    name: String,
    id: i64,
    focused: bool,
    activities: Vec<ActivityStatus>,
}

#[derive(Serialize, Debug, Clone)]
struct ActivityStatus {
    name: String,
    focused: bool,
    workspaces: Vec<WorkspaceStatus>,
}

#[derive(Serialize, Debug, Clone)]
struct WorkspaceStatus {
    name: String,
    focused: bool,
    named_focus: Vec<String>,
    windows: Vec<WindowStatus>,
}

#[derive(Serialize, Debug, Clone)]
struct WindowStatus {
    title: String,
    class: String,
    initial_title: String,
    icon: Option<PathBuf>,
    address: String,
}

struct KEventListener {
    sock: UnixListener,
    hl_events: AsyncEventListener,

    event_tx: mpsc::Sender<KEvent>,
    event_rx: mpsc::Receiver<KEvent>,

    info_event_tx: broadcast::Sender<KInfoEvent>,
    info_event_rx: broadcast::Receiver<KInfoEvent>,
}

impl KEventListener {
    pub async fn new() -> Result<Self> {
        let (hl_tx, hl_rx) = mpsc::channel(100);
        let (info_tx, info_rx) = broadcast::channel(100);
        Ok(Self {
            sock: Self::ipc_sock().await?,
            hl_events: Self::hl_event_listener(hl_tx.clone())?,
            event_tx: hl_tx,
            event_rx: hl_rx,
            info_event_tx: info_tx,
            info_event_rx: info_rx,
        })
    }

    fn hl_event_listener(_tx: mpsc::Sender<KEvent>) -> Result<AsyncEventListener> {
        let mut el = AsyncEventListener::new();
        // TODO: subscribe all required events and fire events in channel
        let tx = _tx.clone();
        el.add_sub_map_change_handler(move |name| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::Submap { name }).await;
            })
        });
        let tx = _tx.clone();
        el.add_workspace_change_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WorkspaceChange).await;
            })
        });
        let tx = _tx.clone();
        el.add_active_window_change_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowChange).await;
            })
        });
        let tx = _tx.clone();
        el.add_window_open_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowOpen).await;
            })
        });
        let tx = _tx.clone();
        el.add_window_moved_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowMoved).await;
            })
        });
        let tx = _tx.clone();
        el.add_window_close_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowClosed).await;
            })
        });
        let tx = _tx.clone();
        el.add_active_monitor_change_handler(move |_m| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::MonitorChange).await;
            })
        });
        let tx = _tx.clone();
        el.add_monitor_added_handler(move |name| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::MonitorAdded { name }).await;
            })
        });
        let tx = _tx.clone();
        el.add_monitor_removed_handler(move |name| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::MonitorRemoved { name }).await;
            })
        });
        Ok(el)
    }

    async fn ipc_sock() -> Result<UnixListener> {
        let sock_path = get_socket_path()?;

        // - [Unix sockets, the basics in Rust - Emmanuel Bosquet](https://emmanuelbosquet.com/2022/whatsaunixsocket/)
        // send a quit message to any daemon that might be running. ignore all errors
        if let Ok(sock) = UnixStream::connect(&sock_path).await {
            let mut sock = BufWriter::new(sock);
            let _ = sock
                .write_all(&Message::Command(Command::DaemonQuit).msg())
                .await;
            let _ = sock.flush().await;
            let _ = sock.shutdown().await;

            let sleep = tokio::time::sleep(Duration::from_millis(300));
            let mut sock = BufReader::new(sock);
            let mut line = String::new();
            tokio::select! {
                res = sock.read_line(&mut line) => {
                    let _ = res;
                    if let Ok(command) = serde_json::from_str(&line) {
                        match command {
                            Message::IpcOk => { }
                            Message::IpcErr(message) => {
                                println!("{}", message);
                            }
                            _ => {
                                unreachable!();
                            }
                        }
                    }
                }
                _ = sleep => { }
            }
        }

        if std::fs::metadata(&sock_path).is_ok() {
            std::fs::remove_file(&sock_path)
                .with_context(|| format!("could not delete previous socket at {:?}", &sock_path))?;
        }

        let sock = UnixListener::bind(&sock_path)?;
        Ok(sock)
    }

    async fn process_ipc_conn(
        stream: UnixStream,
        state: &mut State,
        kevent_tx: &mpsc::Sender<KEvent>,
        kinfo_event_tx: &broadcast::Sender<KInfoEvent>,
        info_ctx: Arc<Mutex<InfoCommandContext>>,
    ) -> Result<()> {
        let mut sock = BufReader::new(stream);
        let mut line = String::new();
        sock.read_line(&mut line).await?;
        if line.is_empty() {
            return Ok(());
        }
        let message = serde_json::from_str::<Message>(&line)?;
        match message {
            Message::Command(Command::DaemonQuit) => {
                sock.write_all(&Message::IpcOk.msg()).await?;
                sock.flush().await?;
                return Ok(());
            }
            Message::Command(Command::Info { command, monitor }) => {
                let tx = kevent_tx.clone();
                let rx = kinfo_event_tx.subscribe();

                #[allow(clippy::let_underscore_future)]
                tokio::spawn(async move {
                    match command
                        .listen_loop(sock.into_inner(), tx, rx, monitor, info_ctx)
                        .await
                    {
                        Ok(()) => {}
                        Err(_e) => {
                            // NOTE: we ignore these errors, as the only errors can be when socket is broken
                            // println!("error in info command: {:?}", _e);
                        }
                    }
                });
            }
            Message::Command(command) => {
                match state.execute(command).await {
                    Ok(_) => {
                        sock.write_all(&Message::IpcOk.msg()).await?;
                    }
                    Err(e) => {
                        sock.write_all(&Message::IpcErr(format!("error: {:?}", e)).msg())
                            .await?;
                    }
                }
                sock.flush().await?;
            }
            _ => {
                unreachable!();
            }
        }

        Ok(())
    }
}

async fn daemon(config: Config) -> Result<()> {
    let mut state = State::new(config.clone()).await?;
    let mut el = KEventListener::new().await?;
    let info_ctx = InfoCommandContext {
        config: config.clone(),
        icons: Default::default(),
    };
    let info_ctx = Arc::new(Mutex::new(info_ctx));

    let sleep_duration = std::time::Duration::from_millis(config.daemon.mouse.polling_rate);

    if config.daemon.move_monitors_to_hyprkool_activity {
        for name in state
            .monitors
            .iter()
            .filter(|m| !m.monitor.disabled)
            .map(|m| m.monitor.name.clone())
            .collect::<Vec<_>>()
        {
            state.move_monitor_to_valid_activity(&name, false).await?;
            state.update_monitors().await?;
        }
    }

    let mut hl_fut = std::pin::pin!(el.hl_events.start_listener_async());
    let mut tick_fut = std::pin::pin!(tokio::time::sleep(sleep_duration));

    loop {
        tokio::select! {
            event = hl_fut.as_mut() => {
                event?;
                return Err(anyhow!("Hyprland socket closed?"));
            }
            event = el.info_event_rx.recv() => {
                match event {
                    Ok(_e) => {
                        // nothing to do here
                        // dbg!(_e);
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => { },
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(anyhow!("info event channel closed"));
                    },
                }
            }
            event = el.event_rx.recv() => {
                match event {
                    Some(event) => {
                        match state.update(event, el.info_event_tx.clone()).await {
                            Ok(()) => {},
                            Err(e) => println!("error during updating state: {:?}", e),
                        }
                    },
                    None => {
                        return Err(anyhow!("hl event channel closed"));
                    }
                }
            }
            event = el.sock.accept() => {
                match event {
                    Ok((stream, _addr)) => {
                        match KEventListener::process_ipc_conn(stream, &mut state, &el.event_tx, &el.info_event_tx, info_ctx.clone()).await {
                            Ok(()) => {},
                            Err(e) => println!("error during ipc connection: {:?}", e),
                        }
                    },
                    Err(e) =>  println!("hyprkool socket conn error: {:?}", e),
                }
            }
            _  = tick_fut.as_mut() => {
                tick_fut.as_mut().set(tokio::time::sleep(sleep_duration));

                match state.tick().await {
                    Ok(()) => {},
                    Err(e) =>  println!("hyprkool errored while ticking: {:?}", e),
                }
            }
        }
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

    async fn move_to_activity(&mut self, activity: String, move_window: bool) -> Result<()> {
        let res = set_workspace_anim(Animation::Fade).await;

        if let Some(ws) = self
            .get_activity_index(&activity)
            .and_then(|i| self.activities[i].last_workspace.as_ref())
            .copied()
        {
            self.move_to(activity, ws, move_window).await?;
        } else if let Some((_, ws)) = self.current() {
            self.move_to(activity, ws, move_window).await?;
        } else {
            self.move_to(activity, KWorkspace { x: 1, y: 1 }, move_window)
                .await?;
        }

        res
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

    async fn move_to(
        &mut self,
        activity: String,
        new_ws: KWorkspace,
        move_window: bool,
    ) -> Result<()> {
        if let Some((a, ws)) = self.current() {
            if let Some(ai) = self.get_activity_index(&a) {
                self.activities[ai].last_workspace = Some(ws);
            }
        };

        if move_window {
            self.move_focused_window_to(&activity, new_ws).await?;
        }

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

        Ok(())
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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

    async fn set_anim(x: i32, y: i32) -> Result<()> {
        if x > 0 && y == 0 {
            return set_workspace_anim(Animation::Right).await;
        }
        if x < 0 && y == 0 {
            return set_workspace_anim(Animation::Left).await;
        }
        if x == 0 && y > 0 {
            return set_workspace_anim(Animation::Down).await;
        }
        if x == 0 && y < 0 {
            return set_workspace_anim(Animation::Up).await;
        }

        set_workspace_anim(Animation::Fade).await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let sock_path = get_socket_path()?;

    match cli.command.clone() {
        Command::Daemon => {
            if cli.force_no_daemon {
                println!("--force-no-daemon not allowed with this command");
                return Ok(());
            }

            daemon(cli.config()?).await?;
            println!("exiting daemon");
        }
        Command::Info { command, monitor } => {
            if !cli.force_no_daemon {
                if let Ok(sock) = UnixStream::connect(&sock_path).await {
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

                        if !monitor && line.is_empty() {
                            return Ok(());
                        }

                        if line.is_empty() {
                            continue;
                        }

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
            }

            // let state = match State::new(cli.config()?) {
            //     Ok(s) => s,
            //     Err(e) => {
            //         println!("{}", e);
            //         return Ok(());
            //     }
            // };
            // command
            //     .execute(
            //         InfoOutputStream::Stdout,
            //         Arc::new(Mutex::new(state)),
            //         monitor,
            //     )
            //     .await?;
            todo!();
        }
        cmd => {
            if !cli.force_no_daemon {
                if let Ok(sock) = UnixStream::connect(&sock_path).await {
                    let mut sock = BufWriter::new(sock);
                    sock.write_all(&Message::Command(cmd.clone()).msg()).await?;
                    sock.flush().await?;
                    sock.shutdown().await?;

                    let sleep = tokio::time::sleep(Duration::from_millis(300));
                    let mut sock = BufReader::new(sock);
                    let mut line = String::new();
                    tokio::select! {
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
                                    return Ok(());
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
            }

            let mut state = match State::new(cli.config()?).await {
                Ok(s) => s,
                Err(e) => {
                    println!("{}", e);
                    return Ok(());
                }
            };
            state.execute(cmd).await?;
        }
    }

    Ok(())
}
