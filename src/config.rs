use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{arg, command, Parser};
use serde::Deserialize;

use crate::command::Command;

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    // TODO: maybe
    // pub enable: bool,
    /// how long to wait for ipc responses before executing the command in ms
    // pub ipc_timeout: u64,
    pub fallback_commands: bool,

    /// remember what workspace was last focused on an activity
    pub remember_activity_focus: bool,

    /// move monitors to a valid hyprkool activity on daemon init
    /// also moves newly added monitors to a valid hyprkool activity
    pub move_monitors_to_hyprkool_activity: bool,

    /// when input:follow_mouse != 1, but you want the focus to change when monitor changes
    pub focus_last_window_on_monitor_change: bool,

    pub mouse: MouseConfig,
}
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            focus_last_window_on_monitor_change: false,
            move_monitors_to_hyprkool_activity: true,
            remember_activity_focus: true,
            fallback_commands: true,
            mouse: Default::default(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct MouseConfig {
    pub switch_workspace_on_edge: bool,
    /// mouse polling rate in ms
    pub polling_rate: u64,
    /// number of pixels to consider as edge
    pub edge_width: u64,
    /// push cursor inside margin when it loops
    pub edge_margin: u64,
}
impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            switch_workspace_on_edge: true,
            polling_rate: 300,
            edge_width: 0,
            edge_margin: 2,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub activities: Vec<String>,
    /// number of workspaces in x and y dimensions
    pub workspaces: (i32, i32),
    pub named_focii: HashMap<String, String>,
    pub daemon: DaemonConfig,

    pub icon_theme: Option<String>,
    pub window_icon_try_min_size: Option<u16>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            activities: vec!["default".into()],
            workspaces: (2, 2),
            named_focii: Default::default(),
            daemon: Default::default(),
            icon_theme: None,
            window_icon_try_min_size: None,
        }
    }
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
pub struct Cli {
    /// Specify a custom config directory
    #[arg(short, long)]
    pub config_dir: Option<String>,

    #[command(subcommand)]
    pub command: Command,

    /// don't use daemon for this command even if one is active (mainly useful for debugging)
    #[arg(long)]
    pub force_no_daemon: bool,
}

impl Cli {
    pub fn config(&self) -> Result<Config> {
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
