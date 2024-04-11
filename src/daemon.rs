use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use hyprland::{
    data::{Client, CursorPosition, Monitor, Workspace},
    dispatch::{Dispatch, DispatchType, WorkspaceIdentifierWithSpecial},
    event_listener::EventListener,
    shared::{HyprData, HyprDataActive, HyprDataActiveOptional},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{UnixListener, UnixStream},
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

        let sock_path = get_socket_path()?;

        // - [Unix sockets, the basics in Rust - Emmanuel Bosquet](https://emmanuelbosquet.com/2022/whatsaunixsocket/)
        // send a quit message to any daemon that might be running. ignore all errors
        if let Ok(sock) = UnixStream::connect(&sock_path).await {
            let mut sock = BufWriter::new(sock);
            let _ = sock
                .write_all(&Message::Command(Command::DaemonQuit).msg())
                .await;
            let _ = sock.write_all("\n".as_bytes()).await;
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
        Ok(Self {
            sock,
            _config: config,
            state,
        })
    }
    async fn listen_loop(&self) -> Result<()> {
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
                            let state = self.state.clone();
                            tokio::spawn(async move {
                                let sock = Arc::new(Mutex::new(sock.into_inner()));
                                loop {
                                    let state = state.clone();
                                    let res = command
                                        .execute(
                                            InfoOutputStream::Stream(sock.clone()),
                                            state,
                                            monitor,
                                        )
                                        .await;
                                    match res {
                                        Ok(_) => {
                                            break;
                                        }
                                        Err(e) => {
                                            // TODO: maybe also try to write this in the socket
                                            println!("error in info command: {}", e);
                                        }
                                    }
                                }
                            });
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

    async fn update(_state: Arc<Mutex<State>>) -> Result<()> {
        let mut el = EventListener::new();

        // let s = state.clone();
        // el.add_workspace_change_handler(move |e| {
        //     let s = s.clone();
        //     tokio::spawn(async move {
        //         let name = match e {
        //             WorkspaceType::Regular(n) => n,
        //             WorkspaceType::Special(n) => n.unwrap_or("special".into()),
        //         };
        //         let mut state = s.lock().await;
        //         let f = state.current_focus.clone();
        //         state.focii.insert(f, name);
        //     });
        // });

        el.start_listener_async().await?;
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        let s = self.state.clone();

        tokio::select! {
            listen = self.listen_loop() => {
                listen
            }
            update = Self::update(s) => {
                update
            }
        }
    }
}

pub fn get_socket_path() -> Result<PathBuf> {
    let hypr_signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("could not get HYPRLAND_INSTANCE_SIGNATURE")?;
    if fs::metadata("/tmp/hyprkool").is_err() {
        fs::create_dir("/tmp/hyprkool")?;
    }
    let sock_path = format!("/tmp/hyprkool/{hypr_signature}.sock");
    Ok(sock_path.into())
}
