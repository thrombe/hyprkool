use std::{cmp::Ordering, collections::HashMap, time::Duration};

use anyhow::{anyhow, Context, Result};
use hyprland::{
    data::{Client, CursorPosition, Monitors, Workspace},
    dispatch::{Dispatch, DispatchType, WindowIdentifier, WorkspaceIdentifierWithSpecial},
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::UnixStream,
};

use crate::{
    config::{Config, MultiMonitorStrategy},
    daemon::get_plugin_socket_path,
    Message,
};

#[derive(Debug)]
pub struct State {
    pub focused: HashMap<String, String>,
    pub named_focii: HashMap<String, String>,
    pub activities: Vec<String>,
    pub workspaces: Vec<Vec<String>>,
    pub config: Config,
}

impl State {
    pub fn new(config: Config) -> Result<Self> {
        for a in config.activities.iter() {
            for c in a.chars() {
                if !c.is_alphanumeric() && !"-_".contains(c) {
                    return Err(anyhow!(
                        "Activity names can only contain a-z A-Z 0-9 - and _ characters. char '{}' in '{}' is not allowed",
                        c,
                        a,
                    ));
                }
            }
        }
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
                    .map(|(x, y)| format!("{name}:({x} {y}$)"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        Ok(Self {
            focused: HashMap::new(),
            named_focii: config.named_focii.clone(),
            activities,
            workspaces: cooked_workspaces,
            config,
        })
    }

    pub fn get_activity_index(&self, name: impl AsRef<str>) -> Option<usize> {
        let name = name.as_ref();
        let activity_index = self.activities.iter().position(|a| name.starts_with(a))?;
        Some(activity_index)
    }

    /// (activity index, workspace index)
    pub fn get_indices(&self, name: impl AsRef<str>) -> Option<(usize, Option<usize>)> {
        let name = name.as_ref();
        let activity_index = self.get_activity_index(name)?;
        let workspace_index = self.workspaces[activity_index]
            .iter()
            .position(|w| w == name);
        Some((activity_index, workspace_index))
    }

    pub async fn moved_workspace(&self, x: i64, y: i64, cycle: bool) -> Result<&str> {
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

    pub async fn move_to_workspace(
        &self,
        name: impl AsRef<str>,
        move_window: bool,
        anim: Animation,
    ) -> Result<()> {
        let res = set_workspace_anim(anim).await;
        let name = name.as_ref();

        let mut window = None;
        if move_window {
            window = Client::get_active_async().await?;
        }
        if window.is_some() {
            Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
                WorkspaceIdentifierWithSpecial::Name(name),
                None,
            ))
            .await?;
        }
        dbg!(&self.config.multi_monitor_strategy);
        match self.config.multi_monitor_strategy {
            MultiMonitorStrategy::SeparateWorkspaces => {
                // get x y for current monitor and switch all monitors to x y w
                let monitors = Monitors::get_async().await?;
                let mut monitors = monitors.into_iter().collect::<Vec<_>>();
                monitors.sort_by(|a, b| {
                    if a.focused {
                        Ordering::Greater
                    } else if b.focused {
                        Ordering::Less
                    } else {
                        Ordering::Equal
                    }
                });
                for m in monitors.iter() {
                    let name = name.replace("$", &m.id.to_string());
                    dbg!(&name, &m);
                    Dispatch::call_async(DispatchType::Custom(
                        "moveworkspacetomonitor",
                        &format!("name:{} {}", name, m.id),
                    ))
                    .await?;
                }
            }
            MultiMonitorStrategy::SharedWorkspacesSyncActivities => {
                // switch all monitors to their corresponding ws in activity mentioned in their current ws
                let name = name.replace("$", "");

                let monitors = Monitors::get_async().await?;
                let mut monitors = monitors.into_iter().collect::<Vec<_>>();
                monitors.sort_by(|a, b| {
                    if a.focused {
                        Ordering::Greater
                    } else if b.focused {
                        Ordering::Less
                    } else {
                        Ordering::Equal
                    }
                });

                let ai = self
                    .get_activity_index(&name)
                    .expect("name passed here is expected to be valid");
                let oa = self.activities[ai].clone();

                let mut free = self.workspaces[ai].clone();
                free.reverse();

                for m in monitors.iter() {
                    let mut name = name.clone();
                    match self
                        .get_activity_index(&m.name)
                        .map(|i| self.activities[i].clone())
                    {
                        Some(a) => {
                            name = name.replace(&oa, &a);
                            free.retain(|w| w != &name);
                        }
                        None => {
                            name = free.pop().context("no free workspace left in current activity. you might have more monitors than workspaces.")?;
                        }
                    }
                    dbg!(&name, &m);
                    Dispatch::call_async(DispatchType::Custom(
                        "focusworkspaceoncurrentmonitor",
                        &format!("name:{}", name),
                    ))
                    .await?;
                }
            }
            MultiMonitorStrategy::SharedWorkspacesUnsyncActivities => {
                let name = name.replace("$", "");
                let monitors = Monitors::get_async().await?;
                let mut monitors = monitors.into_iter().collect::<Vec<_>>();
                monitors.sort_by(|a, b| {
                    if a.focused {
                        Ordering::Greater
                    } else if b.focused {
                        Ordering::Less
                    } else {
                        Ordering::Equal
                    }
                });

                let ai = self
                    .get_activity_index(&name)
                    .expect("name passed here is expected to be valid");
                let mut free = self.workspaces[ai].iter().map(|w| w.replace("$", "")).collect::<Vec<_>>();
                free.retain(|w| w != &name);

                for m in monitors.iter() {
                    if m.focused {
                        dbg!(&name, &m);
                        Dispatch::call_async(DispatchType::Custom(
                            "focusworkspaceoncurrentmonitor",
                            &format!("name:{}", name),
                        ))
                        .await?;
                    } else if self.get_activity_index(&m.active_workspace.name).is_none() {
                        let name = free.pop().context("no free workspace left in current activity.")?;
                        dbg!(&name, &m);
                        Dispatch::call_async(DispatchType::Custom(
                            "moveworkspacetomonitor",
                            &format!("name:{} {}", name, &m.id),
                        ))
                        .await?;
                    }
                }
            }
        }
        if let Some(w) = window {
            let cursor = CursorPosition::get_async().await?;
            Dispatch::call_async(DispatchType::FocusWindow(WindowIdentifier::Address(
                w.address,
            )))
            .await?;
            Dispatch::call_async(DispatchType::MoveCursor(cursor.x, cursor.y)).await?;
        }
        res
    }

    pub async fn move_window_to_workspace(&self, name: impl AsRef<str>) -> Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Name(name),
            None,
        ))
        .await?;
        Ok(())
    }

    pub async fn move_window_to_special_workspace(&self, name: impl AsRef<str>) -> Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Special(Some(name)),
            None,
        ))
        .await?;
        Ok(())
    }

    pub async fn toggle_special_workspace(&self, name: String, anim: Animation) -> Result<()> {
        let res = set_workspace_anim(anim).await;
        Dispatch::call_async(DispatchType::ToggleSpecialWorkspace(Some(name))).await?;
        res
    }

    pub fn get_activity_status_repr(&self, workspace_name: &str) -> Option<String> {
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

    pub fn remember_workspace(&mut self, w: &Workspace) {
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
