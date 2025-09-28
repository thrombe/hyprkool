use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};
use hyprland::data::FullscreenMode;
use hyprland::data::Monitor;
use hyprland::data::Monitors;
use hyprland::dispatch::MonitorIdentifier;
use hyprland::dispatch::WorkspaceIdentifierWithSpecial;
use hyprland::{
    data::{Client, Clients, CursorPosition},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{HyprData, HyprDataActiveOptional},
};
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use crate::command::Command;
use crate::config::Config;
use crate::event::set_workspace_anim;
use crate::event::Animation;
use crate::event::KEvent;
use crate::info::ActivityStatus;
use crate::info::KInfoEvent;
use crate::info::MonitorStatus;
use crate::info::SubmapStatus;
use crate::info::WindowStatus;
use crate::info::WorkspaceStatus;

pub struct State {
    pub config: Config,
    pub monitors: Vec<KMonitor>,
    pub harpoon_map: HashMap<String, String>,
}

impl State {
    pub async fn new(config: Config) -> Result<Self> {
        let m = Monitors::get_async().await?;
        let monitors = m
            .into_iter()
            .map(|m| KMonitor::new(m, &config.activities))
            .collect();

        Ok(Self {
            config,
            monitors,
            harpoon_map: Default::default(),
        })
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

    pub async fn update_monitors(&mut self) -> Result<()> {
        let monitors = Monitors::get_async().await?.into_iter().collect::<Vec<_>>();

        let present = monitors.iter().map(|m| m.name.as_str()).collect::<HashSet<_>>();
        let known = self.monitors.iter().map(|m| m.monitor.name.clone()).collect::<HashSet<_>>();

        // add any that don't already exist
        for m in monitors.iter() {
            if !known.contains(&m.name) {
                self.monitors
                    .push(KMonitor::new(m.clone(), &self.config.activities));
            }
        }

        // remove any that no longer exist
        self.monitors.retain(|m| present.contains(m.monitor.name.as_str()));

        // update the monitor attrs with whatever hyprland gave us
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

    pub async fn move_monitor_to_valid_activity(
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
            for y in 1..=self.config.workspaces.1 {
                for x in 1..=self.config.workspaces.0 {
                    let ws = KWorkspace { x, y };
                    if taken.contains(&ws.name(a)) {
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
        Dispatch::call_async(DispatchType::Custom(
            "focusmonitor",
            &self.focused_monitor_mut().monitor.name.to_string(),
        ))
        .await?;
        Dispatch::call_async(DispatchType::MoveCursor(cursor.x, cursor.y)).await?;

        Ok(())
    }

    async fn move_towards(&mut self, x: i32, y: i32, cycle: bool, move_window: bool) -> Result<()> {
        let (a, ws) = self
            .focused_monitor_mut()
            .current()
            .context("not in a hyprkool workspace")?;
        let ws = self.moved_ws(ws, cycle, x, y);
        _ = KWorkspace::set_anim(x, y).await;
        self.focused_monitor_mut()
            .move_to(a, ws, move_window)
            .await?;
        Ok(())
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

    async fn cycle_monitor(&mut self, z: i32, cycle: bool, move_window: bool) -> Result<()> {
        let mut mi = self
            .monitors
            .iter()
            .position(|m| m.monitor.focused)
            .context("no monitor focused :|")? as i32;
        mi += z;
        if cycle {
            mi += self.monitors.len() as i32;
            mi %= self.monitors.len() as i32;
        } else {
            mi = mi.min(self.monitors.len() as i32 - 1).max(0);
        }
        let m = &mut self.monitors[mi as usize];
        let name = m.monitor.active_workspace.name.clone();
        if move_window {
            m.move_focused_window_to_raw(&name).await?;
        }
        _ = set_workspace_anim(Animation::Fade).await;
        Dispatch::call_async(DispatchType::FocusMonitor(MonitorIdentifier::Name(
            &m.monitor.name,
        )))
        .await?;
        Ok(())
    }

    pub async fn execute(
        &mut self,
        command: Command,
        tx: Option<mpsc::Sender<KEvent>>,
    ) -> Result<()> {
        self._execute(command, tx).await?;
        self.update_monitors().await?;
        Ok(())
    }

    pub async fn _execute(
        &mut self,
        command: Command,
        tx: Option<mpsc::Sender<KEvent>>,
    ) -> Result<()> {
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
                _ = set_workspace_anim(Animation::Fade).await;
                self.focused_monitor_mut()
                    .move_to(a, ws, move_window)
                    .await?;
            }
            Command::SwitchToWorkspace { name, move_window } => {
                let a = KActivity::from_ws_name(&name).context("activity not found")?;
                let ws = KWorkspace::from_ws_name(&name).context("workspace not found")?;
                _ = set_workspace_anim(Animation::Fade).await;
                self.focused_monitor_mut()
                    .move_to(a.name, ws, move_window)
                    .await?;
            }
            Command::NextMonitor { cycle, move_window } => {
                self.cycle_monitor(1, cycle, move_window).await?;
            }
            Command::PrevMonitor { cycle, move_window } => {
                self.cycle_monitor(-1, cycle, move_window).await?;
            }
            Command::SwitchToMonitor { name, move_window } => {
                let m = self
                    .monitors
                    .iter()
                    .find(|m| m.monitor.name == name)
                    .context("monitor with provided name does not exist")?;
                if move_window {
                    m.move_focused_window_to_raw(&m.monitor.active_workspace.name)
                        .await?;
                }
                Dispatch::call_async(DispatchType::FocusMonitor(MonitorIdentifier::Name(
                    &m.monitor.name,
                )))
                .await?;
            }
            Command::SwapMonitorsActiveWorkspace {
                monitor_1,
                monitor_2,
                move_window,
            } => {
                if monitor_1 == monitor_2 && monitor_1.is_some() {
                    return Ok(());
                }
                let (monitor_1, monitor_2) = match (monitor_1, monitor_2) {
                    (None, Some(_)) | (Some(_), None) => {
                        return Err(anyhow!("you need to pass either no or both monitor names"));
                    }
                    (None, None) => {
                        if self.monitors.len() != 2 {
                            return Err(anyhow!("you don't have exactly 2 monitors. you must pass names of both monitors"));
                        }
                        (
                            self.monitors[0].monitor.name.clone(),
                            self.monitors[1].monitor.name.clone(),
                        )
                    }
                    (Some(monitor_1), Some(monitor_2)) => (monitor_1, monitor_2),
                };

                let ws1 = self
                    .monitors
                    .iter()
                    .find(|m| m.monitor.name == monitor_1)
                    .context("monitor_1 does not exist")?
                    .monitor
                    .active_workspace
                    .name
                    .clone();
                let ws2 = self
                    .monitors
                    .iter()
                    .find(|m| m.monitor.name == monitor_2)
                    .context("monitor_2 does not exist")?
                    .monitor
                    .active_workspace
                    .name
                    .clone();
                let m = self.focused_monitor_mut();
                if m.monitor.name == monitor_1 {
                    self.focused_monitor_mut()
                        .move_to_raw(&ws2, move_window)
                        .await?;
                } else if m.monitor.name == monitor_2 {
                    self.focused_monitor_mut()
                        .move_to_raw(&ws1, move_window)
                        .await?;
                } else {
                    // if move_window {
                    //     return Err(anyhow!("--move_window is only supported when one of the monitors is focused"));
                    // }

                    let m_1 = self
                        .monitors
                        .iter_mut()
                        .find(|m| m.monitor.name == monitor_1)
                        .expect("won't get here if it's none");
                    m_1.move_to_raw(&ws2, false).await?;

                    Dispatch::call_async(DispatchType::FocusMonitor(MonitorIdentifier::Name(
                        &self.focused_monitor_mut().monitor.name,
                    )))
                    .await?;
                }
            }
            Command::SetNamedFocus { name } => {
                let w = self
                    .focused_monitor_mut()
                    .monitor
                    .active_workspace
                    .name
                    .clone();
                if self
                    .harpoon_map
                    .get(&name)
                    .map(|ws| &w == ws)
                    .unwrap_or_default()
                {
                    self.harpoon_map.remove(&name);
                } else {
                    self.harpoon_map.insert(name, w);
                }
                if let Some(tx) = &tx {
                    tx.send(KEvent::MonitorInfoRequested).await?;
                }
            }
            Command::SwitchNamedFocus { name, move_window } => {
                match self.harpoon_map.get(&name).cloned() {
                    Some(ws) => {
                        _ = set_workspace_anim(Animation::Fade).await;
                        self.focused_monitor_mut()
                            .move_to_raw(&ws, move_window)
                            .await?;
                    }
                    None => return Err(anyhow!("no workspace set to the provided name")),
                }
            }
            Command::Daemon | Command::DaemonQuit | Command::Info { .. } => {
                return Err(anyhow!("Can't run this command here"))
            }
        }

        Ok(())
    }

    #[allow(clippy::single_match)]
    pub async fn update(&mut self, event: KEvent, tx: broadcast::Sender<KInfoEvent>) -> Result<()> {
        self.update_monitors().await?;

        println!("{:?}", &event);

        match &event {
            KEvent::MonitorAdded { name } => {
                if self.config.daemon.move_monitors_to_hyprkool_activity {
                    self.move_monitor_to_valid_activity(name, false).await?;
                }
            }
            KEvent::MonitorChange { .. } => {
                let clients = Clients::get_async().await?.into_iter().collect::<Vec<_>>();
                tx.send(KInfoEvent::Monitors(self.gather_info(&clients)))?;

                if self.config.daemon.focus_last_window_on_monitor_change {
                    let m = self.focused_monitor_mut();
                    let w = if m.monitor.special_workspace.id != 0 {
                        m.monitor.special_workspace.id
                    } else {
                        m.monitor.active_workspace.id
                    };
                    let w = clients
                        .iter()
                        .filter(|c| w == c.workspace.id)
                        .min_by_key(|c| c.focus_history_id);
                    if let Some(w) = w {
                        let c = CursorPosition::get_async().await?;
                        Dispatch::call_async(DispatchType::FocusWindow(WindowIdentifier::Address(
                            w.address.clone(),
                        )))
                        .await?;
                        Dispatch::call_async(DispatchType::MoveCursor(c.x, c.y)).await?;
                    }
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
            | KEvent::WorkspaceChange
            | KEvent::MonitorAdded { .. }
            | KEvent::MonitorRemoved { .. } => {
                let clients = Clients::get_async().await?.into_iter().collect::<Vec<_>>();
                tx.send(KInfoEvent::Monitors(self.gather_info(&clients)))?;
            }
            KEvent::Submap { name } => {
                tx.send(KInfoEvent::Submap(SubmapStatus { submap: name }))?;
            }
            KEvent::MonitorChange { .. } => {}
        }
        Ok(())
    }

    pub async fn tick(&mut self) -> Result<()> {
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
        let mut harpoons = HashMap::new();
        for (k, v) in self.harpoon_map.iter() {
            if !harpoons.contains_key(v) {
                harpoons.insert(v.clone(), vec![]);
            }

            harpoons.get_mut(v).expect("just inserted").push(k.clone());
        }
        for (_, ks) in harpoons.iter_mut() {
            ks.sort();
        }

        let mut monitors = vec![];
        for m in self.monitors.clone().iter() {
            let mut activities = vec![];
            for a in m.activities.iter() {
                let mut workspaces = vec![];
                for y in 1..=self.config.workspaces.1 {
                    let mut row = vec![];
                    for x in 1..=self.config.workspaces.0 {
                        let ws = KWorkspace { x, y };
                        let ws_name = ws.name(&a.name);

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
                                focused: client.focus_history_id == 0,
                                focus_history_id: client.focus_history_id as _,
                            });
                        }
                        row.push(WorkspaceStatus {
                            focused: m.monitor.active_workspace.name == ws_name,
                            named_focus: harpoons.get(&ws_name).cloned().unwrap_or_default(),
                            name: ws_name,
                            windows,
                        });
                    }
                    workspaces.push(row);
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
                scale: m.monitor.scale,
                activities,
            });
        }

        monitors
    }
}

#[derive(Clone, Debug)]
pub struct KMonitor {
    pub monitor: Monitor,
    pub activities: Vec<KActivity>,
}

impl KMonitor {
    pub fn new(m: Monitor, activities: &[String]) -> Self {
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
        _ = set_workspace_anim(Animation::Fade).await;

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

        Ok(())
    }

    async fn move_to_raw(&mut self, ws_name: &str, move_window: bool) -> Result<()> {
        if let Some((a, ws)) = self.current() {
            if let Some(ai) = self.get_activity_index(&a) {
                self.activities[ai].last_workspace = Some(ws);
            }
        };

        if move_window {
            self.move_focused_window_to_raw(ws_name).await?;
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
            &format!("name:{}", ws_name),
        ))
        .await?;

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
            &format!("name:{}", new_ws.name(&activity)),
        ))
        .await?;

        Ok(())
    }

    async fn move_focused_window_to(&self, activity: &str, ws: KWorkspace) -> Result<()> {
        if let Some(_window) = Client::get_active_async().await? {
            Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                WorkspaceIdentifierWithSpecial::Name(&ws.name(activity)),
                None,
            ))
            .await?;
        }

        Ok(())
    }

    async fn move_focused_window_to_raw(&self, ws: &str) -> Result<()> {
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
pub struct KActivity {
    pub name: String,
    pub last_workspace: Option<KWorkspace>,
}

impl KActivity {
    pub fn from_ws_name(name: &str) -> Option<Self> {
        let (a, _ws) = name.split_once(':')?;
        Some(KActivity {
            name: a.to_owned(),
            last_workspace: None,
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KWorkspace {
    pub x: i32,
    pub y: i32,
}

impl KWorkspace {
    pub fn from_ws_name(name: &str) -> Option<Self> {
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

    pub fn name(&self, activity: &str) -> String {
        format!("{}:({} {})", activity, self.x, self.y)
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
