//! Phase 1 backend: a nested `winit` window. Pumps `winit` input/resize
//! events, accepts Wayland clients on a `ListeningSocket`, ticks the Spaces
//! grid's animations, and renders+presents each frame.

use std::sync::Arc;
use std::time::Instant;

use smithay::backend::input::{Event as _, InputEvent, KeyState, KeyboardKeyEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::input::keyboard::FilterResult;
use smithay::output::{Mode, Output, Scale as OutputScale};
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Rectangle, Serial, Transform};
use smithay::wayland::compositor::{with_surface_tree_downward, SurfaceAttributes, TraversalAction};

use velo_de_config::{to_lower_keysym, Action, Config, KeyCombo, Modifiers};
use velo_de_core::Size;

use crate::ipc;
use crate::render::render_frame;
use crate::state::{spawn_shell, ClientState, State};

const SOCKET_NAME: &str = "wayland-velo-de";

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
fn prepare_child_env() {
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

pub fn run(display: Display<State>, config: Config, output: Output) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = display;
    let dh = display.handle();

    let (mut backend, mut winit_loop) = winit::init::<GlesRenderer>()?;

    let size = backend.window_size();
    let mode = Mode { size, refresh: 60_000 };
    output.change_current_state(Some(mode), Some(Transform::Normal), Some(OutputScale::Integer(1)), Some((0, 0).into()));
    output.set_preferred(mode);
    let _output_global = output.create_global::<State>(&dh);

    let viewport = Size::new(size.w as f64, size.h as f64);
    let ipc = ipc::spawn()?;
    let mut state = State::new(dh, config, output, viewport, ipc);

    let listener = ListeningSocket::bind(SOCKET_NAME)?;
    std::env::set_var("WAYLAND_DISPLAY", SOCKET_NAME);
    tracing::info!(socket = SOCKET_NAME, "velo-de listening for Wayland clients");

    prepare_child_env();
    for cmd in state.config.autostart.clone() {
        spawn_shell(&cmd);
    }

    let keymap: Vec<(KeyCombo, Action)> =
        state.config.keybinds.iter().filter_map(|kb| kb.combo().ok().map(|combo| (combo, kb.action.clone()))).collect();

    let mut last_frame = Instant::now();

    while state.running {
        let mut pending_action: Option<Action> = None;

        let status = winit_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                state.output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.arrange_layers();
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                let Some(keyboard) = state.seat.get_keyboard() else { return };
                let key_state = event.state();
                let action = keyboard.input::<Action, _>(
                    &mut state,
                    event.key_code(),
                    key_state,
                    Serial::from(0),
                    event.time_msec(),
                    |_, mods, keysym| {
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
                    },
                );
                pending_action = action;
            }
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { .. }) => {
                if let Some(id) = state.grid.focused_window() {
                    if let Some(surface) = state.toplevel_for(id) {
                        let surface = surface.wl_surface().clone();
                        if let Some(keyboard) = state.seat.get_keyboard() {
                            keyboard.set_focus(&mut state, Some(surface), Serial::from(0));
                        }
                    }
                }
            }
            _ => {}
        });

        if let PumpStatus::Exit(_) = status {
            break;
        }

        if let Some(action) = pending_action {
            state.apply_action(&action);
        }

        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f64();
        last_frame = now;
        state.tick(dt);
        state.process_ipc();

        if let Ok(Some(stream)) = listener.accept() {
            let _ = display.handle().insert_client(stream, Arc::new(ClientState::default()));
        }

        let size = backend.window_size();
        {
            let (renderer, mut framebuffer) = backend.bind()?;
            render_frame(&mut state, renderer, &mut framebuffer, size)?;
        }

        let time = state.start_time.elapsed().as_millis() as u32;
        for (_, surface) in state.windows() {
            send_frame_callbacks(surface.wl_surface(), time);
        }

        let damage = Rectangle::from_size(size);
        backend.submit(Some(&[damage]))?;

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }

    Ok(())
}

fn send_frame_callbacks(surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, time: u32) {
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
