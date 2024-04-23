use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use clap::{arg, command, Parser};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::UnixStream,
    sync::Mutex,
};

use crate::{
    command::Command,
    config::Config,
    daemon::{IpcDaemon, MouseDaemon},
    info::InfoOutputStream,
    state::State,
};

mod command;
mod config;
mod daemon;
mod info;
mod state;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    fn config(&self) -> Result<Config> {
        let config = self
            .config_dir
            .clone()
            .map(PathBuf::from)
            .or(dirs::config_dir().map(|pb| pb.join("hypr")))
            .map(|pb| pb.join("hyprkool.toml"))
            .filter(|p| p.exists())
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
        Ok(config)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum Message {
    IpcOk,
    IpcErr(String),
    IpcMessage(String),
    Command(Command),
}
impl Message {
    fn msg(&self) -> Vec<u8> {
        serde_json::to_string(self).unwrap().into_bytes()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let sock_path = daemon::get_socket_path()?;

    match cli.command.clone() {
        Command::Daemon {
            move_to_hyprkool_activity,
        } => {
            let state = match State::new(cli.config()?) {
                Ok(s) => s,
                Err(e) => {
                    println!("{}", e);
                    return Ok(());
                }
            };
            let state = Arc::new(Mutex::new(state));
            let mut md = MouseDaemon::new(state.clone()).await?;
            let id = IpcDaemon::new(state.clone()).await?;
            let mut id_fut = std::pin::pin!(id.run());

            loop {
                tokio::select! {
                    mouse = md.run(move_to_hyprkool_activity) => {
                        match mouse {
                            Ok(_) => {
                                break;
                            }
                            Err(e) => {
                                println!("{}", e);
                            }
                        }
                    }
                    ipc = &mut id_fut => {
                        match ipc {
                            Ok(_) => {
                                break;
                            }
                            Err(e) => {
                                println!("{}", e);
                                id_fut.set(id.run());
                            }
                        }
                    }
                }
            }
            println!("exiting daemon");
        }
        Command::Info { command, monitor } => {
            if let Ok(sock) = UnixStream::connect(&sock_path).await {
                let mut sock = BufWriter::new(sock);
                sock.write_all(
                    &Message::Command(Command::Info {
                        command: command.clone(),
                        monitor,
                    })
                    .msg(),
                )
                .await?;
                sock.flush().await?;
                sock.shutdown().await?;

                let mut sock = BufReader::new(sock);
                loop {
                    let mut line = String::new();
                    let _ = sock.read_line(&mut line).await?;

                    if !monitor && line.is_empty() {
                        return Ok(());
                    }

                    let command = serde_json::from_str(&line)?;
                    match command {
                        Message::IpcMessage(message) => {
                            println!("{}", message);
                        }
                        Message::IpcErr(message) => {
                            println!("{}", message);
                        }
                        _ => {
                            unreachable!();
                        }
                    }
                }
            }

            let config = cli.config()?;
            if !config.daemon.fallback_commands {
                return Ok(());
            }
            dbg!("falling back to stateless commands");

            let state = match State::new(cli.config()?) {
                Ok(s) => s,
                Err(e) => {
                    println!("{}", e);
                    return Ok(());
                }
            };
            command
                .execute(
                    InfoOutputStream::Stdout,
                    Arc::new(Mutex::new(state)),
                    monitor,
                )
                .await?;
        }
        comm => {
            if let Ok(sock) = UnixStream::connect(&sock_path).await {
                let mut sock = BufWriter::new(sock);
                sock.write_all(&Message::Command(comm.clone()).msg())
                    .await?;
                sock.flush().await?;
                sock.shutdown().await?;

                let sleep = tokio::time::sleep(Duration::from_millis(300));
                let mut sock = BufReader::new(sock);
                let mut line = String::new();
                tokio::select! {
                    res = sock.read_line(&mut line) => {
                        res?;
                        let command = serde_json::from_str(&line)?;
                        match command {
                            Message::IpcOk => {
                                println!("Ok");
                                return Ok(());
                            }
                            Message::IpcErr(message) => {
                                println!("{}", message);
                                return Ok(());
                            }
                            _ => {
                                unreachable!();
                            }
                        }
                    }
                    _ = sleep => {
                        println!("timeout. could not connect to hyprkool");
                    }
                }
            }

            let config = cli.config()?;
            if !config.daemon.fallback_commands {
                return Ok(());
            }
            println!("falling back to stateless commands");
            let state = match State::new(cli.config()?) {
                Ok(s) => s,
                Err(e) => {
                    println!("{}", e);
                    return Ok(());
                }
            };
            comm.execute(Arc::new(Mutex::new(state)), false).await?;
        }
    }

    Ok(())
}
