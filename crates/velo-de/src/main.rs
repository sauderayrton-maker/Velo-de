//! `velo-de`: a Velo-styled Wayland desktop environment built around a 2D
//! grid of scrollable "Spaces" (see `velo-de-core`).

mod backend;
mod ipc;
mod render;
mod shell;
mod state;

use smithay::output::{Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::Display;

use state::State;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let config = velo_de_config::Config::load()?;
    let display: Display<State> = Display::new()?;

    let output = Output::new(
        "velo-de-0".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Velo".to_string(),
            model: "velo-de".to_string(),
        },
    );

    // SDDM (and other login managers) launch sessions with no
    // `WAYLAND_DISPLAY`/`DISPLAY` set; an existing graphical session sets
    // one or both. Mirrors how Hyprland/Sway pick a nested-vs-standalone
    // backend.
    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    if nested {
        tracing::info!("WAYLAND_DISPLAY/DISPLAY set; running nested winit backend");
        backend::winit::run(display, config, output)
    } else {
        tracing::info!("no parent Wayland/X11 session detected; running standalone udev backend");
        backend::udev::run(display, config, output)
    }
}
