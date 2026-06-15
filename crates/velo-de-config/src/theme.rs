//! The shared Velo "glass" palette, also used by Velo-shell, Velo-launcher,
//! Velo-Browser, Velo-Files and Velo-player (see their `src/style.css`).

use serde::{Deserialize, Serialize};

/// An RGBA color, stored as `0.0..=1.0` floats (ready for a GLES uniform)
/// but configured/serialized as a `#rrggbb` or `#rrggbbaa` hex string.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba8(r, g, b, 255)
    }

    pub fn from_hex(s: &str) -> Result<Self, String> {
        let s = s.trim().trim_start_matches('#');
        let component = |range: std::ops::Range<usize>| -> Result<u8, String> {
            let chunk = s.get(range.clone()).ok_or_else(|| format!("invalid color: {s:?}"))?;
            u8::from_str_radix(chunk, 16).map_err(|_| format!("invalid color: {s:?}"))
        };

        match s.len() {
            6 => Ok(Self::rgb8(component(0..2)?, component(2..4)?, component(4..6)?)),
            8 => Ok(Self::rgba8(component(0..2)?, component(2..4)?, component(4..6)?, component(6..8)?)),
            _ => Err(format!("invalid color: {s:?} (expected #rrggbb or #rrggbbaa)")),
        }
    }

    pub fn to_hex(self) -> String {
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", to_u8(self.r), to_u8(self.g), to_u8(self.b))
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", to_u8(self.r), to_u8(self.g), to_u8(self.b), to_u8(self.a))
        }
    }

    /// RGBA as `0.0..=1.0` floats, ready for a renderer uniform.
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl TryFrom<String> for Color {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Color::from_hex(&s)
    }
}

impl From<Color> for String {
    fn from(c: Color) -> String {
        c.to_hex()
    }
}

/// The shared "Lexus Cockpit / Hyprland Glass" palette. Defaults are pulled
/// verbatim from `Velo-shell/src/style.css` so the compositor's gaps,
/// borders and Overview chrome sit naturally alongside the Velo apps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Shows through the gaps between tiled windows (`#09090c`).
    pub background: Color,
    /// Used behind Overview tiles for Spaces that don't exist yet (`#06060a`).
    pub view_background: Color,
    /// Focused-window border / Overview selection (`#8ab4d4`).
    pub accent: Color,
    /// Secondary accent, e.g. inactive-but-occupied indicators (`#4d8fb8`).
    pub accent_strong: Color,
    /// Soft glow drawn just outside the focused window's border.
    pub accent_glow: Color,
    pub border_width: f64,
    pub corner_radius: f64,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::rgb8(0x09, 0x09, 0x0c),
            view_background: Color::rgb8(0x06, 0x06, 0x0a),
            accent: Color::rgb8(0x8a, 0xb4, 0xd4),
            accent_strong: Color::rgb8(0x4d, 0x8f, 0xb8),
            accent_glow: Color::rgba8(0x8a, 0xb4, 0xd4, 0x14),
            border_width: 2.0,
            corner_radius: 10.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Color::from_hex("#8ab4d4").unwrap();
        assert_eq!(c.to_hex(), "#8ab4d4");
        assert!((c.r - 0x8a as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn hex_round_trip_with_alpha() {
        let c = Color::from_hex("#8ab4d414").unwrap();
        assert_eq!(c.to_hex(), "#8ab4d414");
    }

    #[test]
    fn rejects_bad_hex() {
        assert!(Color::from_hex("#zzzzzz").is_err());
        assert!(Color::from_hex("#fff").is_err());
    }

    #[test]
    fn defaults_match_velo_palette() {
        let theme = Theme::default();
        assert_eq!(theme.background.to_hex(), "#09090c");
        assert_eq!(theme.accent.to_hex(), "#8ab4d4");
        assert_eq!(theme.accent_strong.to_hex(), "#4d8fb8");
    }
}
