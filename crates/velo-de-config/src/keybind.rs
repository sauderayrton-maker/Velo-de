//! Parsing keybinding strings (e.g. `"Super+Shift+Left"`) into modifier +
//! keysym combos, and the [`Action`]s a key combo can trigger.

use serde::{Deserialize, Serialize};
use velo_de_core::{Command, Direction, NotNan};
use xkbcommon::xkb;

/// Modifier keys tracked for keybindings. "Super" is the Windows/Command
/// key (xkb `Mod4`/`Logo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub logo: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// A parsed keybinding: modifiers plus a single keysym.
///
/// The keysym is normalized to its unshifted/lowercase form (via
/// [`to_lower_keysym`]) at parse time. The compositor must apply the same
/// normalization to keysyms it reads from the keyboard before comparing,
/// so that e.g. `"Super+Shift+B"` matches pressing Super+Shift+b (which
/// produces the keysym for `B`, not `b`) while still requiring the Shift
/// modifier to be held (checked separately via [`Modifiers`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCombo {
    pub modifiers: Modifiers,
    pub keysym: xkb::Keysym,
}

/// Lowercase ASCII `A..=Z` keysyms to their `a..=z` equivalents; all other
/// keysyms (named keys like `Left`/`Return`, digits, function keys, ...)
/// are returned unchanged. This is the simple ASCII subset of what
/// `xkb_keysym_to_lower` does in full libxkbcommon, which is sufficient for
/// the Latin keybinding names used here.
pub fn to_lower_keysym(sym: xkb::Keysym) -> xkb::Keysym {
    const A: u32 = 0x41; // 'A'
    const Z: u32 = 0x5a; // 'Z'
    const CASE_OFFSET: u32 = 0x61 - 0x41; // 'a' - 'A'

    let raw = sym.raw();
    if (A..=Z).contains(&raw) {
        xkb::Keysym::new(raw + CASE_OFFSET)
    } else {
        sym
    }
}

impl KeyCombo {
    /// Parse a combo like `"Super+Shift+Left"` or `"Super+Return"`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
        let Some((key, mods)) = parts.split_last() else {
            return Err(format!("empty key combo: {s:?}"));
        };

        let mut modifiers = Modifiers::default();
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "super" | "logo" | "mod4" => modifiers.logo = true,
                "shift" => modifiers.shift = true,
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "mod1" => modifiers.alt = true,
                other => return Err(format!("unknown modifier {other:?} in {s:?}")),
            }
        }

        let keysym = xkb::keysym_from_name(key, xkb::KEYSYM_CASE_INSENSITIVE);
        if keysym == xkb::Keysym::new(xkb::keysyms::KEY_NoSymbol) {
            return Err(format!("unknown key {key:?} in {s:?}"));
        }

        Ok(Self { modifiers, keysym: to_lower_keysym(keysym) })
    }
}

/// The grid-relative direction of a [`Action`] variant. Mirrors
/// [`velo_de_core::Direction`] with `serde` support for config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl From<Dir> for Direction {
    fn from(dir: Dir) -> Direction {
        match dir {
            Dir::Left => Direction::Left,
            Dir::Right => Direction::Right,
            Dir::Up => Direction::Up,
            Dir::Down => Direction::Down,
        }
    }
}

/// Everything a keybinding (or the IPC `dispatch` command) can trigger.
/// Most variants map 1:1 onto [`velo_de_core::Command`] via
/// [`Action::to_command`]; [`Action::Spawn`] and [`Action::Quit`] are
/// handled by the compositor directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    FocusColumn(Dir),
    MoveColumn(Dir),
    SwitchSpace(Dir),
    MoveWindowToSpace(Dir),
    CycleWindow,
    ToggleColumnLayout,
    ToggleFullscreen,
    CloseWindow,
    /// Multiply the focused column's width by this factor (e.g. `1.1`).
    ResizeColumn(f64),
    ToggleOverview,
    OverviewMove(Dir),
    OverviewConfirm,
    OverviewCancel,
    /// Run a shell command (e.g. to launch Velo-Browser/Files/Player).
    Spawn(String),
    /// Run [`Config::terminal`](crate::Config::terminal).
    SpawnTerminal,
    /// Cleanly shut down the compositor.
    Quit,
}

impl Action {
    /// The [`Command`] to apply to the focused output's [`velo_de_core::Grid`],
    /// or `None` for actions the compositor itself must handle.
    pub fn to_command(&self) -> Option<Command> {
        Some(match self {
            Action::FocusColumn(dir) => Command::FocusColumn((*dir).into()),
            Action::MoveColumn(dir) => Command::MoveColumn((*dir).into()),
            Action::SwitchSpace(dir) => Command::SwitchSpace((*dir).into()),
            Action::MoveWindowToSpace(dir) => Command::MoveWindowToSpace((*dir).into()),
            Action::CycleWindow => Command::CycleWindow,
            Action::ToggleColumnLayout => Command::ToggleColumnLayout,
            Action::ToggleFullscreen => Command::ToggleFullscreen,
            Action::CloseWindow => Command::CloseFocused,
            Action::ResizeColumn(factor) => Command::ResizeColumn(NotNan::new(*factor)),
            Action::ToggleOverview => Command::ToggleOverview,
            Action::OverviewMove(dir) => Command::OverviewMove((*dir).into()),
            Action::OverviewConfirm => Command::OverviewConfirm,
            Action::OverviewCancel => Command::OverviewCancel,
            Action::Spawn(_) | Action::SpawnTerminal | Action::Quit => return None,
        })
    }
}

/// One entry in `config.toml`'s `keybinds` list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keybind {
    pub key: String,
    pub action: Action,
}

impl Keybind {
    pub fn new(key: impl Into<String>, action: Action) -> Self {
        Self { key: key.into(), action }
    }

    pub fn combo(&self) -> Result<KeyCombo, String> {
        KeyCombo::parse(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_combo() {
        let combo = KeyCombo::parse("Super+Return").unwrap();
        assert!(combo.modifiers.logo);
        assert!(!combo.modifiers.shift);
        assert_eq!(combo.keysym, xkb::Keysym::new(xkb::keysyms::KEY_Return));
    }

    #[test]
    fn parses_multi_modifier_combo() {
        let combo = KeyCombo::parse("Super+Shift+Left").unwrap();
        assert!(combo.modifiers.logo);
        assert!(combo.modifiers.shift);
        assert_eq!(combo.keysym, xkb::Keysym::new(xkb::keysyms::KEY_Left));
    }

    #[test]
    fn normalizes_letter_case() {
        let lower = KeyCombo::parse("Super+B").unwrap();
        let upper = KeyCombo::parse("Super+Shift+B").unwrap();
        // Both should reference the lowercase 'b' keysym; Shift is tracked
        // separately via `modifiers.shift`.
        assert_eq!(lower.keysym, xkb::Keysym::new(xkb::keysyms::KEY_b));
        assert_eq!(upper.keysym, xkb::Keysym::new(xkb::keysyms::KEY_b));
        assert!(!lower.modifiers.shift);
        assert!(upper.modifiers.shift);
    }

    #[test]
    fn rejects_unknown_key_and_modifier() {
        assert!(KeyCombo::parse("Super+NotAKey").is_err());
        assert!(KeyCombo::parse("Hyper+Return").is_err());
    }

    #[test]
    fn action_to_command_roundtrip() {
        assert_eq!(Action::ToggleOverview.to_command(), Some(Command::ToggleOverview));
        assert_eq!(Action::FocusColumn(Dir::Left).to_command(), Some(Command::FocusColumn(Direction::Left)));
        assert_eq!(Action::Spawn("kitty".into()).to_command(), None);
    }

    #[test]
    fn serde_round_trip_for_keybind() {
        let kb = Keybind::new("Super+Return", Action::Spawn("kitty".into()));
        let toml = toml::to_string(&kb).unwrap();
        let back: Keybind = toml::from_str(&toml).unwrap();
        assert_eq!(kb, back);
    }
}
