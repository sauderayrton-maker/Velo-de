//! Phase 1 backend: a nested `winit` window. Pumps `winit` input/resize
//! events, accepts Wayland clients on a `ListeningSocket`, ticks the Spaces
//! grid's animations, and renders+presents each frame.

use std::sync::Arc;
use std::time::Instant;

use smithay::backend::input::{Event as _, InputEvent, KeyboardKeyEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::output::{Mode, Output, Scale as OutputScale};
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Rectangle, Serial, Transform};

use velo_de_config::{Action, Config};
use velo_de_core::Size;

use crate::backend::input::{build_keymap, handle_keyboard};
use crate::backend::{prepare_child_env, send_frame_callbacks};
use crate::ipc;
use crate::render::render_frame;
use crate::state::{spawn_shell, ClientState, State};

const SOCKET_NAME: &str = "wayland-velo-de";

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

    let keymap = build_keymap(&state);

    let mut last_frame = Instant::now();

    while state.running {
        let mut pending_action: Option<Action> = None;

        let status = winit_loop.dispatch_new_events(|event| match event {
            WinitEvent::Resized { size, .. } => {
                state.output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.arrange_layers();
            }
            WinitEvent::Input(InputEvent::Keyboard { event }) => {
                pending_action = handle_keyboard(&mut state, &keymap, event.key_code(), event.state(), event.time_msec());
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
