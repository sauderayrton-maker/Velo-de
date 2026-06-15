//! The top-level `velo-de` configuration, loaded from
//! `~/.config/velo-de/config.toml` (created with Velo-styled defaults on
//! first run).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::keybind::{Action, Dir, Keybind};
use crate::theme::Theme;

/// Default gap (in logical pixels) between tiled windows, and between the
/// Spaces grid's edge and the output edge.
pub const DEFAULT_GAP: f64 = 12.0;

/// The full `velo-de` configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
    /// Gap between tiled windows and around the Spaces grid, in logical pixels.
    pub gap: f64,
    /// Command spawned by [`Action::SpawnTerminal`] (`Super+Return`).
    pub terminal: String,
    /// Commands run once at compositor startup (e.g. `velo-shell`).
    pub autostart: Vec<String>,
    /// Action triggered by tapping `Super` alone (no other key).
    pub super_tap_action: Action,
    pub keybinds: Vec<Keybind>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            gap: DEFAULT_GAP,
            terminal: "kitty".into(),
            autostart: vec!["velo-shell".into()],
            super_tap_action: Action::ToggleOverview,
            keybinds: default_keybinds(),
        }
    }
}

/// The default keybinding table described in the Velo-de design.
pub fn default_keybinds() -> Vec<Keybind> {
    vec![
        Keybind::new("Super+Space", Action::Spawn("velo-launcher".into())),
        Keybind::new("Super+Return", Action::SpawnTerminal),
        Keybind::new("Super+B", Action::Spawn("velo".into())),
        Keybind::new("Super+E", Action::Spawn("velo-files".into())),
        Keybind::new("Super+M", Action::Spawn("velo-player".into())),
        Keybind::new("Super+Left", Action::FocusColumn(Dir::Left)),
        Keybind::new("Super+Right", Action::FocusColumn(Dir::Right)),
        Keybind::new("Super+Shift+Left", Action::MoveColumn(Dir::Left)),
        Keybind::new("Super+Shift+Right", Action::MoveColumn(Dir::Right)),
        Keybind::new("Super+Up", Action::SwitchSpace(Dir::Up)),
        Keybind::new("Super+Down", Action::SwitchSpace(Dir::Down)),
        Keybind::new("Super+Ctrl+Left", Action::SwitchSpace(Dir::Left)),
        Keybind::new("Super+Ctrl+Right", Action::SwitchSpace(Dir::Right)),
        Keybind::new("Super+Shift+Up", Action::MoveWindowToSpace(Dir::Up)),
        Keybind::new("Super+Shift+Down", Action::MoveWindowToSpace(Dir::Down)),
        Keybind::new("Super+Tab", Action::CycleWindow),
        Keybind::new("Super+R", Action::ToggleColumnLayout),
        Keybind::new("Super+F", Action::ToggleFullscreen),
        Keybind::new("Super+Q", Action::CloseWindow),
        Keybind::new("Super+Equal", Action::ResizeColumn(1.1)),
        Keybind::new("Super+Minus", Action::ResizeColumn(1.0 / 1.1)),
    ]
}

impl Config {
    /// `~/.config/velo-de/config.toml`, or `None` if no config directory
    /// could be determined.
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("velo-de").join("config.toml"))
    }

    /// Load the config from disk, writing out Velo-styled defaults first if
    /// no config file exists yet. Falls back to in-memory defaults if the
    /// config directory can't be determined.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };

        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }

        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {path:?}: {e}"))?;
        toml::from_str(&text).map_err(|e| format!("parsing {path:?}: {e}"))
    }

    /// Write this config to `~/.config/velo-de/config.toml`, creating the
    /// directory if needed.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::path() else {
            return Err("could not determine config directory".into());
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
        }

        let text = toml::to_string_pretty(self).map_err(|e| format!("serializing config: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("writing {path:?}: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(config, back);
    }

    #[test]
    fn default_keybinds_all_parse() {
        for kb in default_keybinds() {
            kb.combo().unwrap_or_else(|e| panic!("bad default keybind {kb:?}: {e}"));
        }
    }

    #[test]
    fn default_gap_and_terminal() {
        let config = Config::default();
        assert_eq!(config.gap, DEFAULT_GAP);
        assert_eq!(config.terminal, "kitty");
        assert_eq!(config.autostart, vec!["velo-shell".to_string()]);
        assert_eq!(config.super_tap_action, Action::ToggleOverview);
    }
}
