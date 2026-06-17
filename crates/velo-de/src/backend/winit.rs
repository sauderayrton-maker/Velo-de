//! Phase 1 backend: a nested `winit` window. Pumps `winit` input/resize
//! events, accepts Wayland clients on a `ListeningSocket`, ticks the Spaces
//! grid's animations, and renders+presents each frame.

use std::sync::Arc;
use std::time::Instant;

use smithay::backend::input::{AbsolutePositionEvent, ButtonState, Event as _, InputEvent, KeyboardKeyEvent, PointerButtonEvent};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::input::pointer::{ButtonEvent, MotionEvent};
use smithay::output::{Mode, Output, Scale as OutputScale};
use smithay::reexports::wayland_server::{Display, ListeningSocket};
use smithay::reexports::winit::platform::pump_events::PumpStatus;
use smithay::utils::{Point, Rectangle, Transform, SERIAL_COUNTER};

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
    let mut state = State::new(dh.clone(), config, output, viewport, ipc);

    // Advertise `zwp_linux_dmabuf_v1` with the renderer's supported import
    // formats so GPU clients (Firefox, Chromium, ...) can share buffers.
    let dmabuf_formats: Vec<_> = backend.renderer().egl_context().dmabuf_render_formats().iter().copied().collect();
    let _dmabuf_global = state.dmabuf_state.create_global::<State>(&dh, dmabuf_formats);

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
            WinitEvent::Input(InputEvent::PointerMotionAbsolute { event }) => {
                let output_size = state
                    .output
                    .current_mode()
                    .map(|m| m.size)
                    .unwrap_or_default();
                let x = event.x_transformed(output_size.w);
                let y = event.y_transformed(output_size.h);
                let location = Point::from((x, y));
                state.cursor_pos = location;

                let focus = state.surface_under(location);
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.motion(
                        &mut state,
                        focus,
                        &MotionEvent {
                            location,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(&mut state);
                }
            }
            WinitEvent::Input(InputEvent::PointerButton { event }) => {
                let pos = state.cursor_pos;
                if event.state() == ButtonState::Pressed {
                    focus_under_cursor(&mut state, pos);
                }
                if let Some(pointer) = state.seat.get_pointer() {
                    pointer.button(
                        &mut state,
                        &ButtonEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time_msec(),
                            button: event.button_code(),
                            state: event.state(),
                        },
                    );
                    pointer.frame(&mut state);
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
            render_frame(&mut state, renderer, &mut framebuffer, size, smithay::utils::Transform::Flipped180)?;
        }

        let time = state.start_time.elapsed().as_millis() as u32;
        for (_, surface) in state.windows() {
            send_frame_callbacks(surface.wl_surface(), time);
        }
        for (surface, _, _) in state.layer_entries() {
            send_frame_callbacks(surface.wl_surface(), time);
        }
        for popup in state.popup_surfaces() {
            send_frame_callbacks(popup.wl_surface(), time);
        }

        let damage = Rectangle::from_size(size);
        backend.submit(Some(&[damage]))?;

        display.dispatch_clients(&mut state)?;
        display.flush_clients()?;
    }

    Ok(())
}

/// Click-to-focus: if the window under the cursor isn't the currently focused
/// one, make its column active in the grid (which moves keyboard focus too).
fn focus_under_cursor(state: &mut State, pos: Point<f64, smithay::utils::Logical>) {
    if let Some((surface, _)) = state.surface_under(pos) {
        if let Some(id) = state.window_id_for_wl_surface(&surface) {
            if state.grid.focused_window() != Some(id) {
                state.apply_command(velo_de_core::Command::FocusWindowById(id));
            }
        }
    }
}
