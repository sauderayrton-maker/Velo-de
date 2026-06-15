//! Configuration for `velo-de`: the shared Velo "glass" theme, keybindings,
//! and the top-level [`Config`] loaded from `~/.config/velo-de/config.toml`.

pub mod config;
pub mod keybind;
pub mod theme;

pub use config::Config;
pub use keybind::{to_lower_keysym, Action, Dir, KeyCombo, Keybind, Modifiers};
pub use theme::{Color, Theme};
