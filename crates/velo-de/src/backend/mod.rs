//! Compositor backends. [`winit`] runs `velo-de` as a window inside an
//! existing Wayland/X11 session (nested development/testing). [`udev`] runs
//! it standalone on a bare VT via DRM/KMS, GBM/EGL and libinput — used when
//! launched from a login manager like SDDM. [`input`] holds keybind-dispatch
//! logic shared by both.

pub mod input;
pub mod udev;
pub mod winit;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{with_surface_tree_downward, SurfaceAttributes, TraversalAction};

/// Prepare the environment inherited by spawned children (`velo-shell`,
/// `velo-launcher`, and Velo-Browser/Files/Player):
///
/// - Set `HYPRLAND_INSTANCE_SIGNATURE` and prepend the directory containing
///   the `hyprctl` shim (built by `velo-hyprctl`, alongside this binary) to
///   `PATH`, so `velo-shell` resolves `hyprctl` to the shim and
///   `Velo-shell/src/hypr.rs::event_socket_path()` to our compat socket.
/// - Default `GSK_RENDERER=gl`: GTK4's default (Vulkan/NGL) renderer fails
///   with `VK_ERROR_SURFACE_LOST_KHR` under `velo-de`, which doesn't yet
///   advertise `zwp_linux_dmabuf_v1`. Left alone if already set.
pub(crate) fn prepare_child_env() {
    std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", velo_de_ipc::HYPRLAND_INSTANCE_SIGNATURE);

    if std::env::var_os("GSK_RENDERER").is_none() {
        std::env::set_var("GSK_RENDERER", "gl");
    }

    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    if !dir.join("hyprctl").is_file() {
        return;
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path));
    if let Ok(new_path) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", new_path);
    }
}

/// Run every pending `wl_surface.frame` callback on `surface` and its
/// subsurfaces, marking them done at `time` (milliseconds since startup).
pub(crate) fn send_frame_callbacks(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_, states, &()| {
            for callback in states.cached_state.get::<SurfaceAttributes>().current().frame_callbacks.drain(..) {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}
