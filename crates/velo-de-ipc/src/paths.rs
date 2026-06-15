//! Well-known socket paths, rooted at `$XDG_RUNTIME_DIR` (falling back to
//! `/tmp` if unset).

use std::path::PathBuf;

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// `velo-de`'s native IPC socket.
pub fn socket_path() -> PathBuf {
    runtime_dir().join("velo-de").join("velo-de.sock")
}

/// The `HYPRLAND_INSTANCE_SIGNATURE` value `velo-de` sets for spawned
/// children, so `Velo-shell/src/hypr.rs::event_socket_path()` resolves to
/// [`hypr_event_socket_path`].
pub const HYPRLAND_INSTANCE_SIGNATURE: &str = "velo-de";

/// The Hyprland-event-socket-compatible path
/// (`$XDG_RUNTIME_DIR/hypr/velo-de/.socket2.sock`), read verbatim by
/// `Velo-shell/src/hypr.rs::subscribe()`.
pub fn hypr_event_socket_path() -> PathBuf {
    runtime_dir().join("hypr").join(HYPRLAND_INSTANCE_SIGNATURE).join(".socket2.sock")
}
