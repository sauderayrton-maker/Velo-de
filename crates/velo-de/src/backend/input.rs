//! Shared keybind dispatch, used by both the [`super::winit`] and
//! [`super::udev`] backends so each only has to translate its own
//! `InputEvent::Keyboard` event into `(keycode, key_state, time_msec)`.

use smithay::backend::input::{KeyState, Keycode};
use smithay::input::keyboard::FilterResult;
use smithay::utils::Serial;

use velo_de_config::{to_lower_keysym, Action, KeyCombo, Modifiers};

use crate::state::State;

/// Build the `(KeyCombo, Action)` lookup table from `state.config.keybinds`,
/// skipping any that fail to parse.
pub fn build_keymap(state: &State) -> Vec<(KeyCombo, Action)> {
    state.config.keybinds.iter().filter_map(|kb| kb.combo().ok().map(|combo| (combo, kb.action.clone()))).collect()
}

/// Feed a keyboard key event to `state.seat`'s keyboard, returning the
/// matched [`Action`] if the combo is bound and the key was pressed.
pub fn handle_keyboard(state: &mut State, keymap: &[(KeyCombo, Action)], keycode: Keycode, key_state: KeyState, time_msec: u32) -> Option<Action> {
    let keyboard = state.seat.get_keyboard()?;
    keyboard.input::<Action, _>(state, keycode, key_state, Serial::from(0), time_msec, |_, mods, keysym| {
        if key_state != KeyState::Pressed {
            return FilterResult::Forward;
        }
        let combo = KeyCombo {
            modifiers: Modifiers { logo: mods.logo, shift: mods.shift, ctrl: mods.ctrl, alt: mods.alt },
            keysym: to_lower_keysym(keysym.modified_sym()),
        };
        match keymap.iter().find(|(c, _)| *c == combo) {
            Some((_, action)) => FilterResult::Intercept(action.clone()),
            None => FilterResult::Forward,
        }
    })
}
