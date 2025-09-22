use std::sync::Arc;

use anyhow::Result;
use clap::{arg, command, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::event::KEvent;
use crate::event::Message;
use crate::info::InfoCommandContext;
use crate::info::KInfoEvent;

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
                            for row in a.workspaces.iter_mut() {
                                for w in row.iter_mut() {
                                    for c in w.windows.iter_mut() {
                                        c.icon = ctx.get_icon_path(
                                            &c.initial_title,
                                            window_icon_theme.as_ref(),
                                            *window_icon_try_min_size,
                                        );
                                    }
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

    pub async fn listen_loop(
        self,
        mut sock: UnixStream,
        tx: mpsc::Sender<KEvent>,
        mut rx: broadcast::Receiver<KInfoEvent>,
        monitor: bool,
        info_ctx: Arc<Mutex<InfoCommandContext>>,
    ) -> Result<()> {
        // NOTE: DO NOT return errors other than socket errors

        if let Err(e) = self.fire_events(tx).await {
            println!("error when firing info events: {:?}", e);
            sock.write_all(&Message::IpcErr(format!("error: {}", e)).msg())
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
                    println!("error when listening for info messages: {:?}", e);
                    sock.write_all(&Message::IpcErr(format!("error: {}", e)).msg())
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
    NextMonitor {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to monitor
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    PrevMonitor {
        #[arg(long, short, default_value_t = false)]
        cycle: bool,

        /// move focused window and move to monitor
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SwapMonitorsActiveWorkspace {
        /// name of 1st monitor (only necessary if you have more than 2 monitors)
        #[arg(long, short = 'm')]
        monitor_1: Option<String>,

        /// name of 2nd monitor (only necessary if you have more than 2 monitors)
        #[arg(long, short = 'n')]
        monitor_2: Option<String>,

        /// move focused window and swap workspaces
        #[arg(long, short = 'w', default_value_t = false)]
        move_window: bool,
    },
    SwitchToMonitor {
        /// <monitor name> (see `hyprctl monitors`)
        #[arg(short, long)]
        name: String,

        /// move focused window and move to monitor
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
}
