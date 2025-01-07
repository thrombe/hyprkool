use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use hyprland::event_listener::AsyncEventListener;
use hyprland::shared::WorkspaceType;
use serde::{Deserialize, Serialize};
use tokio::io::BufWriter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::UnixStream;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::command::Command;
use crate::config::Config;
use crate::info::InfoCommandContext;
use crate::info::KInfoEvent;
use crate::state::State;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Message {
    IpcOk,
    IpcErr(String),
    IpcMessage(String),
    Command(Command),
}
impl Message {
    pub fn msg(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_string(self).unwrap().into_bytes();
        bytes.extend_from_slice(b"\n");
        bytes
    }
}

#[derive(Clone, Debug)]
pub enum KEvent {
    WindowChange,
    WindowOpen,
    WindowMoved,
    WindowClosed,
    WorkspaceChange,
    MonitorChange {
        name: String,
        workspace: Option<WorkspaceType>,
    },
    MonitorAdded {
        name: String,
    },
    MonitorRemoved {
        name: String,
    },
    Submap {
        name: String,
    },

    MonitorInfoRequested,
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
        let tx = _tx.clone();
        el.add_sub_map_changed_handler(move |name| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::Submap { name }).await;
            })
        });
        let tx = _tx.clone();
        el.add_workspace_changed_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WorkspaceChange).await;
            })
        });
        let tx = _tx.clone();
        el.add_active_window_changed_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowChange).await;
            })
        });
        let tx = _tx.clone();
        el.add_window_opened_handler(move |_w| {
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
        el.add_window_closed_handler(move |_w| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::WindowClosed).await;
            })
        });
        let tx = _tx.clone();
        el.add_active_monitor_changed_handler(move |m| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx
                    .send(KEvent::MonitorChange {
                        name: m.monitor_name,
                        workspace: m.workspace_name,
                    })
                    .await;
            })
        });
        let tx = _tx.clone();
        el.add_monitor_added_handler(move |m| {
            let tx = tx.clone();
            Box::pin(async move {
                _ = tx.send(KEvent::MonitorAdded { name: m.name }).await;
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
    ) -> Result<bool> {
        let mut sock = BufReader::new(stream);
        let mut line = String::new();
        sock.read_line(&mut line).await?;
        if line.is_empty() {
            return Ok(false);
        }
        let message = serde_json::from_str::<Message>(&line)?;
        match message {
            Message::Command(Command::DaemonQuit) => {
                sock.write_all(&Message::IpcOk.msg()).await?;
                sock.flush().await?;
                return Ok(true);
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
                match state.execute(command, Some(kevent_tx.clone())).await {
                    Ok(_) => {
                        sock.write_all(&Message::IpcOk.msg()).await?;
                    }
                    Err(e) => {
                        println!("error when executing command: {:?}", e);
                        sock.write_all(&Message::IpcErr(format!("error: {}", e)).msg())
                            .await?;
                    }
                }
                sock.flush().await?;
            }
            _ => {
                unreachable!();
            }
        }

        Ok(false)
    }
}

pub async fn daemon(config: Config) -> Result<()> {
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
                            Ok(quit) => {
                                if quit {
                                    break Ok(());
                                }
                            },
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
    match _send_plugin_event(anim as _).await {
        Ok(_) => {}
        Err(err) => {
            println!("could not set workspace animation: {:?}", err);
            return Err(err);
        }
    }
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
