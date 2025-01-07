#![allow(dead_code)]

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use tokio::io::BufWriter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

mod command;
mod config;
mod event;
mod info;
mod state;

use command::Command;
use config::Cli;
use event::daemon;
use event::get_socket_path;
use event::Message;
use state::State;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let sock_path = get_socket_path()?;

    match cli.command.clone() {
        Command::Daemon => {
            if cli.force_no_daemon {
                println!("--force-no-daemon not allowed with this command");
                return Ok(());
            }

            daemon(cli.config()?).await?;
            println!("exiting daemon");
        }
        Command::Info { command, monitor } => {
            if !cli.force_no_daemon {
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

                        if line.is_empty() {
                            continue;
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
            }

            // let state = match State::new(cli.config()?) {
            //     Ok(s) => s,
            //     Err(e) => {
            //         println!("{}", e);
            //         return Ok(());
            //     }
            // };
            // command
            //     .execute(
            //         InfoOutputStream::Stdout,
            //         Arc::new(Mutex::new(state)),
            //         monitor,
            //     )
            //     .await?;
            todo!("info command are currently only supported with the daemon running. run 'hyprkool daemon'");
        }
        cmd => {
            if !cli.force_no_daemon {
                if let Ok(sock) = UnixStream::connect(&sock_path).await {
                    let mut sock = BufWriter::new(sock);
                    sock.write_all(&Message::Command(cmd.clone()).msg()).await?;
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
            }

            let mut state = match State::new(cli.config()?).await {
                Ok(s) => s,
                Err(e) => {
                    println!("{}", e);
                    return Ok(());
                }
            };
            state.execute(cmd, None).await?;
        }
    }

    Ok(())
}
