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

    backend::winit::run(display, config, output)
}
