use std::collections::HashMap;
use std::path::PathBuf;

use linicon::IconPath;
use serde::{Deserialize, Serialize};

use crate::config::Config;

pub struct InfoCommandContext {
    pub config: Config,

    /// (theme, size, class)
    pub icons: HashMap<(Option<String>, u16, String), Option<PathBuf>>,
}

impl InfoCommandContext {
    pub fn get_icon_path(
        &mut self,
        class: &str,
        theme: Option<&String>,
        window_icon_try_min_size: Option<u16>,
    ) -> Option<PathBuf> {
        let theme = theme.cloned().or(self.config.icon_theme.clone());
        let icon_min_size = window_icon_try_min_size
            .or(self.config.window_icon_try_min_size)
            .unwrap_or(0);

        if let Some(icon) = self
            .icons
            .get(&(theme.clone(), icon_min_size, class.to_string()))
        {
            return icon.clone();
        }

        let mut icons = linicon::lookup_icon(class);

        if let Some(theme) = &theme {
            icons = icons.from_theme(theme);
        }

        let mut icon = None;
        let mut alt = None;
        for next in icons {
            let Some(next) = next.ok() else {
                continue;
            };
            if next.min_size >= icon_min_size
                && next.min_size
                    < icon
                        .as_ref()
                        .map(|i: &IconPath| i.min_size)
                        .unwrap_or(u16::MAX)
            {
                icon = Some(next);
            } else if next.min_size
                > alt
                    .as_ref()
                    .map(|i: &IconPath| i.min_size)
                    .unwrap_or(u16::MIN)
            {
                alt = Some(next);
            }
        }

        let mut icon = icon.or(alt).map(|i| i.path);

        if icon.is_none() {
            if class == "wayland" {
                return None;
            }
            icon = self.get_icon_path("wayland", theme.as_ref(), window_icon_try_min_size);
        }

        self.icons
            .insert((theme, icon_min_size, class.to_string()), icon.clone());

        icon
    }
}

#[derive(Clone, Debug)]
pub enum KInfoEvent {
    Submap(SubmapStatus),
    Monitors(Vec<MonitorStatus>),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct SubmapStatus {
    pub submap: String,
}

#[derive(Serialize, Debug, Clone)]
pub struct MonitorStatus {
    pub name: String,
    pub id: i64,
    pub focused: bool,
    pub activities: Vec<ActivityStatus>,
    pub scale: f32,
}

#[derive(Serialize, Debug, Clone)]
pub struct ActivityStatus {
    pub name: String,
    pub focused: bool,
    pub workspaces: Vec<Vec<WorkspaceStatus>>,
}

#[derive(Serialize, Debug, Clone)]
pub struct WorkspaceStatus {
    pub name: String,
    pub focused: bool,
    pub named_focus: Vec<String>,
    pub windows: Vec<WindowStatus>,
}

#[derive(Serialize, Debug, Clone)]
pub struct WindowStatus {
    pub title: String,
    pub class: String,
    pub initial_title: String,
    pub icon: Option<PathBuf>,
    pub address: String,
    pub focused: bool,
    pub focus_history_id: i32,
}
