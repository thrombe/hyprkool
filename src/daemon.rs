use std::sync::Arc;

use anyhow::{Context, Result};
use hyprland::{
    data::{Client, CursorPosition, Monitor, Workspace},
    dispatch::{Dispatch, DispatchType, WorkspaceIdentifierWithSpecial},
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::Mutex,
};

use crate::{Command, Config, InfoOutputStream, Message, State};

pub struct MouseDaemon {
    state: Arc<Mutex<State>>,

    // TODO: multi monitor setup yaaaaaaaaaaaaaaaaa
    monitor: Monitor,

    config: Config,
}
impl MouseDaemon {
    pub async fn new(state: Arc<Mutex<State>>) -> Result<Self> {
        let s = state.lock().await;
        let monitor = Monitor::get_active_async().await?;
        let config = s.config.clone();
        drop(s);

        Ok(Self {
            config,
            monitor,
            state,
        })
    }

    pub async fn run(&mut self, move_to_hyprkool_activity: bool) -> Result<()> {
        let workspace = Workspace::get_active_async().await?;

        {
            let state = self.state.lock().await;

            if state.get_indices(&workspace.name).is_none() && move_to_hyprkool_activity {
                Dispatch::call_async(DispatchType::Workspace(
                    WorkspaceIdentifierWithSpecial::Name(&state.workspaces[0][0]),
                ))
                .await?;
            };
        }

        let w = self.config.daemon.mouse.edge_width as i64;
        let m = self.config.daemon.mouse.edge_margin as i64;
        let enabled = self.config.daemon.mouse.switch_workspace_on_edge;

        let mut sleep_duration =
            std::time::Duration::from_millis(self.config.daemon.mouse.polling_rate);

        if !enabled {
            sleep_duration = std::time::Duration::from_secs(10000000);
        }

        loop {
            tokio::time::sleep(sleep_duration).await;
            if !enabled {
                continue;
            }

            let nx = self.config.workspaces.0 as usize;
            let ny = self.config.workspaces.1 as usize;
            let mut c = CursorPosition::get_async().await?;
            let mut y = 0;
            let mut x = 0;
            if c.x <= w {
                x += nx - 1;
                c.x = self.monitor.width as i64 - m;
            } else if c.x >= self.monitor.width as i64 - 1 - w {
                x += 1;
                c.x = m;
            }
            if c.y <= w {
                y += ny - 1;
                c.y = self.monitor.height as i64 - m;
            } else if c.y >= self.monitor.height as i64 - 1 - w {
                y += 1;
                c.y = m;
            }

            if x + y == 0 {
                continue;
            }

            let workspace = Workspace::get_active_async().await?;

            let state = self.state.lock().await;

            let Some((current_activity_index, Some(current_workspace_index))) =
                state.get_indices(&workspace.name)
            else {
                println!("unknown workspace {}", workspace.name);
                continue;
            };

            if let Some(window) = Client::get_active_async().await? {
                if window.fullscreen && window.fullscreen_mode == 0 {
                    continue;
                }
            }

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
}

pub struct IpcDaemon {
    state: Arc<Mutex<State>>,
    _config: Config,
    sock: UnixListener,
}
impl IpcDaemon {
    pub async fn new(state: Arc<Mutex<State>>) -> Result<Self> {
        let s = state.lock().await;
        let config = s.config.clone();
        drop(s);

        // - [Unix sockets, the basics in Rust - Emmanuel Bosquet](https://emmanuelbosquet.com/2022/whatsaunixsocket/)
        let sock_path = "/tmp/hyprkool.sock";
        if std::fs::metadata(sock_path).is_ok() {
            println!("A socket is already present. Deleting...");
            std::fs::remove_file(sock_path)
                .with_context(|| format!("could not delete previous socket at {:?}", sock_path))?;
        }

        let sock = UnixListener::bind(sock_path)?;
        Ok(Self {
            sock,
            _config: config,
            state,
        })
    }
    pub async fn run(&mut self) -> Result<()> {
        loop {
            match self.sock.accept().await {
                Ok((stream, _addr)) => {
                    let mut sock = BufReader::new(stream);
                    let mut line = String::new();
                    sock.read_line(&mut line).await?;
                    let message = serde_json::from_str::<Message>(&line)?;
                    match message {
                        Message::Command(Command::DaemonQuit) => {
                            sock.write_all(&Message::IpcOk.msg()).await?;
                            return Ok(());
                        }
                        Message::Command(Command::Info { command, monitor }) => {
                            tokio::spawn(command.execute(
                                InfoOutputStream::Stream(Arc::new(Mutex::new(sock.into_inner()))),
                                self.state.clone(),
                                monitor,
                            ));
                            // return Ok(());
                            continue;
                        }
                        Message::Command(command) => {
                            match command.execute(self.state.clone(), true).await {
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
}
