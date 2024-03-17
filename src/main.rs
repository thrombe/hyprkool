use std::path::PathBuf;

use anyhow::{anyhow, Context};
use clap::{arg, command, Parser, Subcommand};
use hyprland::{
    data::{Client, Clients, CursorPosition, Monitor, Workspace},
    dispatch::{Dispatch, DispatchType, WorkspaceIdentifierWithSpecial},
    event_listener::EventListener,
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional, WorkspaceType},
};
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub activities: Vec<String>,
    /// number of workspaces in x and y dimensions
    pub workspaces: (u32, u32),

    /// mouse polling rate in ms
    pub polling_rate: u64,
    /// number of pixels to consider as edge
    pub edge_width: u64,
    /// push cursor inside margin when it loops
    pub edge_margin: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            activities: vec!["default".into()],
            workspaces: (2, 2),
            polling_rate: 300,
            edge_width: 0,
            edge_margin: 2,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    MouseLoop,
    PrintActivityStatus,
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

#[derive(Debug)]
pub struct State {
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

    async fn moved_workspace(&self, x: i64, y: i64, cycle: bool) -> anyhow::Result<&str> {
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

    async fn move_to_workspace(
        &self,
        name: impl AsRef<str>,
        move_window: bool,
    ) -> anyhow::Result<()> {
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

    async fn move_window_to_workspace(&self, name: impl AsRef<str>) -> anyhow::Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Name(name),
            None,
        ))
        .await?;
        Ok(())
    }

    async fn move_window_to_special_workspace(&self, name: impl AsRef<str>) -> anyhow::Result<()> {
        let name = name.as_ref();
        Dispatch::call_async(DispatchType::MoveToWorkspaceSilent(
            WorkspaceIdentifierWithSpecial::Special(Some(name)),
            None,
        ))
        .await?;
        Ok(())
    }

    async fn toggle_special_workspace(&self, name: String) -> anyhow::Result<()> {
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = cli
        .config_dir
        .clone()
        .map(PathBuf::from)
        .or(dirs::config_dir().map(|pb| pb.join("hypr/hyprkool.toml")))
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
    let state = State::new(config);

    match cli.command {
        Command::MouseLoop => {
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&state.workspaces[0][0]),
            ))
            .await?;

            // TODO: multi monitor setup yaaaaaaaaaaaaaaaaa
            let monitor = Monitor::get_active_async().await?;
            let w = state.config.edge_width as i64;
            let m = state.config.edge_margin as i64;

            loop {
                tokio::time::sleep(std::time::Duration::from_millis(state.config.polling_rate))
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
                name.push('0');
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
            let id = activity_index.and_then(|i| workspace.name.strip_prefix(&state.activities[i]));
            let mut name = state.activities[new_activity_index].clone();
            if let Some(id) = id {
                name.push_str(id);
            } else {
                name = state.workspaces[new_activity_index][0].clone();
            };
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
            let id = activity_index.and_then(|i| workspace.name.strip_prefix(&state.activities[i]));
            let activity_index = new_activity_index;
            let mut name = state.activities[activity_index].clone();
            if let Some(id) = id {
                name.push_str(id);
            } else {
                name = state.workspaces[activity_index][0].clone();
            };
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
        Command::PrintActivityStatus => {
            #[derive(Deserialize, Serialize, Debug)]
            struct ActivityStatus {
                text: String,
            }
            fn print_state(state: &State, name: &str) {
                state
                    .get_activity_status_repr(name)
                    .into_iter()
                    .for_each(|a| {
                        println!(
                            "{}",
                            serde_json::to_string(&ActivityStatus { text: a })
                                .expect("it will work")
                        );
                    });
            }

            let workspace = Workspace::get_active_async().await?;
            print_state(&state, &workspace.name);

            let mut ael = EventListener::new();
            ael.add_workspace_change_handler(move |e| match e {
                WorkspaceType::Regular(name) => {
                    print_state(&state, &name);
                }
                WorkspaceType::Special(..) => {}
            });
            ael.start_listener_async().await?;
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
    }

    Ok(())
}
