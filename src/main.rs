use anyhow::Context;
use clap::{arg, command, Parser, Subcommand};
use hyprland::{
    data::{CursorPosition, Monitor, Workspace},
    dispatch::{Dispatch, DispatchType, WorkspaceIdentifierWithSpecial},
    event_listener::EventListener,
    shared::{HyprData, HyprDataActive},
};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    MouseLoop,
    MoveRight {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    MoveLeft {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    MoveUp {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    MoveDown {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    NextActivity {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    PrevActivity {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,
    },
    SwitchToActivity {
        /// <activity name>
        #[arg(short, long)]
        name: String,
    },
    SwitchToWorkspaceInActivity {
        /// <workspace name>
        #[arg(short, long)]
        name: String,
    },
    SwitchToWorkspace {
        /// <activity name>:<workspace name>
        #[arg(short, long)]
        name: String,
    },
}

#[derive(Debug)]
pub struct State {
    pub activities: Vec<String>,
    pub workspaces: Vec<Vec<String>>,
}

impl State {
    fn new() -> Self {
        let raw_workspaces = [0, 1, 2, 3, 4, 5, 6, 7, 8];
        let activities = ["issac", "colg"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let cooked_workspaces = activities
            .iter()
            .map(|name| {
                raw_workspaces
                    .iter()
                    .cloned()
                    .map(|id| format!("{name}:{id}"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        Self {
            activities,
            workspaces: cooked_workspaces,
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let state = State::new();

    match cli.command {
        Command::MouseLoop => {
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&state.workspaces[0][0]),
            ))
            .await?;

            // TODO: multi monitor setup yaaaaaaaaaaaaaaaaa
            let monitor = Monitor::get_active_async().await?;

            loop {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let mut c = CursorPosition::get_async().await?;
                let mut y = 0;
                let mut x = 0;
                if c.x == 0 {
                    x += 3 - 1;
                    c.x = monitor.width as i64 - 2;
                } else if c.x == monitor.width as i64 - 1 {
                    x += 1;
                    c.x = 2;
                }
                if c.y == 0 {
                    y += 3 - 1;
                    c.y = monitor.height as i64 - 2;
                } else if c.y == monitor.height as i64 - 1 {
                    y += 1;
                    c.y = 2;
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

                y += current_workspace_index / 3;
                y %= 3;
                x += current_workspace_index % 3;
                x %= 3;

                let new_workspace = &state.workspaces[current_activity_index][y * 3 + x];
                if new_workspace != &workspace.name {
                    Dispatch::call_async(DispatchType::Workspace(
                        WorkspaceIdentifierWithSpecial::Name(new_workspace),
                    ))
                    .await?;
                    Dispatch::call_async(DispatchType::MoveCursor(c.x, c.y)).await?;
                }
            }
        }
        Command::SwitchToWorkspace { name } => {
            let (activity_index, workspace_index) =
                state.get_indices(&name).context("activity not found")?;
            let workspace_index = workspace_index.context("workspace not found")?;
            let new_workspace = &state.workspaces[activity_index][workspace_index];
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(new_workspace),
            ))
            .await?;
        }
        Command::SwitchToWorkspaceInActivity { name } => {
            let workspace = Workspace::get_active_async().await?;
            let activity_index = state.get_activity_index(&workspace.name).context("could not get current activity")?;
            let activity = &state.activities[activity_index];
            let new_workspace = format!("{activity}:{name}");
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&new_workspace),
            ))
            .await?;
        }
        Command::SwitchToActivity { mut name } => {
            let workspace = Workspace::get_active_async().await?;
            if let Some(activity_index) = state.get_activity_index(&workspace.name) {
                let activity = &state.activities[activity_index];
                let id = workspace.name.strip_prefix(activity).expect("just checked this");
                name.push_str(id);
            } else {
                name.push('0');
            };
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&name),
            ))
            .await?;
        }
        Command::NextActivity { cycle } => {
            let workspace = Workspace::get_active_async().await?;
            let mut activity_index = state.get_activity_index(&workspace.name).map(|i| i+1).unwrap_or(0);
            let id = workspace.name.strip_prefix(&state.activities[activity_index]);
            if cycle {
                activity_index %= state.activities.len();
            } else {
                activity_index = activity_index.min(state.activities.len() - 1);
            }
            let mut name = state.activities[activity_index].clone();
            if let Some(id) = id {
                name.push_str(id);
            } else {
                name = state.workspaces[activity_index][0].clone();
            };
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&name),
            ))
            .await?;
        }
        Command::PrevActivity { cycle } => {
            let workspace = Workspace::get_active_async().await?;
            let mut activity_index = state.get_activity_index(&workspace.name).map(|i| i as isize-1).unwrap_or(0);
            let id = (activity_index >= 0).then(|| workspace.name.strip_prefix(&state.activities[activity_index as usize])).flatten();
            if cycle {
                activity_index += state.activities.len() as isize;
                activity_index %= state.activities.len() as isize;
            } else {
                activity_index = activity_index.max(0);
            }
            let activity_index = activity_index as usize;
            let mut name = state.activities[activity_index].clone();
            if let Some(id) = id {
                name.push_str(id);
            } else {
                name = state.workspaces[activity_index][0].clone();
            };
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(&name),
            ))
            .await?;
        }
        _ => {
            todo!()
        }
    }

    Ok(())
}
