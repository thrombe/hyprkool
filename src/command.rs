use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{arg, command, Subcommand};
use hyprland::{
    data::{Client, Clients, CursorPosition, Workspace},
    dispatch::{Dispatch, DispatchType, WindowIdentifier},
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{info::InfoCommand, state::NamedFocus, State};

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
        name: Option<String>,

        /// move focused window and move to workspace
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    LockNamedFocus {
        /// lock current named focus if none
        #[arg(short, long)]
        name: Option<String>,

        #[arg(short, long)]
        lock: Option<bool>,
    },
    DeleteNamedFocus {
        /// delete current named focus if none
        #[arg(short, long)]
        name: Option<String>,
    },
}

impl Command {
    pub async fn execute(self, state: Arc<Mutex<State>>, stateful: bool) -> Result<()> {
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
                let new_workspace = state.workspaces[activity_index][workspace_index].clone();
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
                if state.get_activity_index(&name).is_none() {
                    state.activities.push(name.clone());
                    let w = state.workspaces[0]
                        .iter()
                        .flat_map(|w| w.split(':').skip(1))
                        .map(|w| format!("{}:{}", &name, w))
                        .collect();
                    state.workspaces.push(w);
                }
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
                let workspace = state.moved_workspace(1, 0, cycle).await?.to_owned();
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveLeft { cycle, move_window } => {
                let workspace = state.moved_workspace(-1, 0, cycle).await?.to_owned();
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveUp { cycle, move_window } => {
                let workspace = state.moved_workspace(0, -1, cycle).await?.to_owned();
                state.move_to_workspace(workspace, move_window).await?;
            }
            Command::MoveDown { cycle, move_window } => {
                let workspace = state.moved_workspace(0, 1, cycle).await?.to_owned();
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
            Command::SwitchNamedFocus { name, move_window } => {
                let Some(name) = name else {
                    state.current_named_focus = None;
                    return Ok(());
                };

                let workspace = Workspace::get_active_async().await?;

                if let Some(current) = state.current_named_focus.clone() {
                    if let Some(nf) = state.named_focii.get_mut(&current) {
                        if !nf.locked {
                            nf.workspace = workspace.name;
                        }
                    } else {
                        state.named_focii.insert(
                            current,
                            NamedFocus {
                                workspace: workspace.name,
                                locked: false,
                            },
                        );
                    }
                }
                state.current_named_focus = None;
                if let Some(nf) = state.named_focii.get(&name).map(|s| s.to_owned()) {
                    if !nf.locked {
                        state.current_named_focus = Some(name);
                    }
                    state.move_to_workspace(nf.workspace, move_window).await?;
                }
            }
            Command::LockNamedFocus { name, lock } => {
                let Some(name) = name else {
                    if let Some(name) = state.current_named_focus.clone() {
                        if let Some(nf) = state.named_focii.get_mut(&name) {
                            match lock {
                                Some(l) => {
                                    nf.locked = l;
                                }
                                None => {
                                    nf.locked = !nf.locked;
                                }
                            }
                        }
                    }
                    return Ok(());
                };

                let locked;
                if let Some(nf) = state.named_focii.get_mut(&name) {
                    match lock {
                        Some(l) => {
                            nf.locked = l;
                        }
                        None => {
                            nf.locked = !nf.locked;
                        }
                    }
                    locked = nf.locked;
                } else {
                    let workspace = Workspace::get_active_async().await?;
                    locked = lock.unwrap_or(true);
                    state.named_focii.insert(
                        name,
                        NamedFocus {
                            workspace: workspace.name,
                            locked,
                        },
                    );
                }

                if locked {
                    state.current_named_focus = None;
                }
            }
            Command::DeleteNamedFocus { name } => {
                if let Some(name) = name.or_else(|| state.current_named_focus.take()) {
                    let _ = state
                        .named_focii
                        .remove(&name)
                        .context("named focus with this name does not exist")?;
                }
            }
            _ => {
                return Err(anyhow!("cannot ececute these commands here"));
            }
        }

        Ok(())
    }
}
