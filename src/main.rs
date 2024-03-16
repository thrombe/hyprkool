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
    InitLoop,
    MoveRight,
    MoveLeft,
    MoveUp,
    MoveDown,
    NextActivity,
    PrevActivity,
    SwitchActivity {
        /// activity name
        #[arg(short, long)]
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let raw_workspaces = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    let activities = ["issac", "colg"];
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

    Dispatch::call_async(DispatchType::Workspace(
        WorkspaceIdentifierWithSpecial::Name(&cooked_workspaces[0][0]),
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
        let Some(current_activity_index) = activities
            .iter()
            .position(|a| workspace.name.starts_with(a))
        else {
            println!("unknown workspace {}", workspace.name);
            continue;
        };
        let Some(current_workspace_index) = cooked_workspaces[current_activity_index]
            .iter()
            .position(|w| w == &workspace.name)
        else {
            println!("unknown workspace {}", workspace.name);
            continue;
        };

        y += current_workspace_index / 3;
        y %= 3;
        x += current_workspace_index % 3;
        x %= 3;

        let new_workspace = &cooked_workspaces[current_activity_index][y * 3 + x];
        if new_workspace != &workspace.name {
            Dispatch::call_async(DispatchType::Workspace(
                WorkspaceIdentifierWithSpecial::Name(new_workspace),
            ))
            .await?;
            Dispatch::call_async(DispatchType::MoveCursor(c.x, c.y)).await?;
        }
    }
}
