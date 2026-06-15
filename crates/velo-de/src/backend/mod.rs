//! Compositor backends. Phase 1 is the nested `winit` backend ([`winit`]),
//! which runs `velo-de` as a window inside an existing Wayland/X11 session —
//! exactly how Smithay's own `anvil` is normally developed against. A bare
//! metal `udev`/DRM backend (for a real login-manager session) is future
//! work and intentionally not stubbed here yet.

pub mod winit;
