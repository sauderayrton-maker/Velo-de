//! Phase 2 backend: a standalone session driven by udev/DRM/KMS, GBM, EGL
//! and libinput, with device access brokered by libseat. Used when `velo-de`
//! is launched with no parent Wayland/X11 session (e.g. from SDDM), as
//! opposed to [`super::winit`]'s nested window.

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::input::{
    Axis as InputAxis, Event as _, InputEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::Bind;
use smithay::backend::session::{libseat::LibSeatSession, Event as SessionEvent, Session};
use smithay::backend::udev::{primary_gpu, UdevBackend, UdevEvent};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::output::{Mode as OutputMode, Output, Scale as OutputScale};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::drm::control::{connector, Device as _};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::{Display, DisplayHandle, ListeningSocket};
use smithay::utils::{DeviceFd, Physical, Point, Rectangle, Size as SmithaySize, Transform, SERIAL_COUNTER};

use velo_de_config::{Action, Config, KeyCombo};
use velo_de_core::Size;

use crate::backend::input::{build_keymap, handle_keyboard};
use crate::backend::{prepare_child_env, send_frame_callbacks};
use crate::ipc;
use crate::render::render_frame;
use crate::state::{spawn_shell, ClientState, State};

const SOCKET_NAME: &str = "wayland-velo-de";

/// Per-output GPU/render state, distinct from [`State`] (which is shared
/// with [`super::winit`]).
struct UdevData {
    /// Kept alive for its `Drop` impl, which releases the seat/DRM master.
    #[allow(dead_code)]
    session: LibSeatSession,
    primary_gpu_devnum: u64,
    renderer: GlesRenderer,
    gbm_surface: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>,
    output_size: SmithaySize<i32, Physical>,
    pointer_location: Point<f64, smithay::utils::Logical>,
}

/// The single `&mut` value calloop hands to every event-source callback.
struct CalloopData {
    state: State,
    udev: UdevData,
    keymap: Vec<(KeyCombo, Action)>,
    listener: ListeningSocket,
    display_handle: DisplayHandle,
    last_frame: Instant,
}

pub fn run(display: Display<State>, config: Config, output: Output) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, session_notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    let gpu_path = primary_gpu(&seat_name)?
        .or_else(|| smithay::backend::udev::all_gpus(&seat_name).ok().and_then(|gpus| gpus.into_iter().next()))
        .ok_or("velo-de: no GPU found")?;
    let primary_gpu_devnum = std::fs::metadata(&gpu_path)?.rdev();

    let device_fd = DrmDeviceFd::new(DeviceFd::from(session.open(&gpu_path, OFlags::RDWR | OFlags::CLOEXEC)?));
    let (mut drm, drm_notifier) = DrmDevice::new(device_fd.clone(), true)?;

    let gbm = GbmDevice::new(device_fd.clone())?;

    // SAFETY: `gbm` (cloned below) outlives the `EGLDisplay`/`EGLContext`,
    // and we don't call any other code that would race with smithay's
    // internal `eglGetPlatformDisplay` deduplication.
    let egl_display = unsafe { EGLDisplay::new(gbm.clone())? };
    let egl_context = EGLContext::new(&egl_display)?;
    let render_formats = egl_context.dmabuf_render_formats().clone();
    // SAFETY: `egl_context` was just created above and is not shared.
    let renderer = unsafe { GlesRenderer::new(egl_context)? };

    // Pick the first connected connector, its preferred mode, and a CRTC for it.
    let resources = drm.resource_handles()?;
    let connector_info = resources
        .connectors()
        .iter()
        .filter_map(|&conn| drm.get_connector(conn, false).ok())
        .find(|info| info.state() == connector::State::Connected)
        .ok_or("velo-de: no connected display found")?;
    let mode = *connector_info.modes().first().ok_or("velo-de: connector has no modes")?;
    let crtc = connector_info
        .current_encoder()
        .and_then(|enc| drm.get_encoder(enc).ok())
        .and_then(|enc| enc.crtc())
        .or_else(|| resources.crtcs().first().copied())
        .ok_or("velo-de: no CRTC available")?;

    let drm_surface = drm.create_surface(crtc, mode, &[connector_info.handle()])?;
    let gbm_allocator = GbmAllocator::new(gbm, GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let gbm_surface = GbmBufferedSurface::new(drm_surface, gbm_allocator, &[Fourcc::Argb8888, Fourcc::Xrgb8888], render_formats)?;

    // Build the real Output from the chosen DRM mode.
    let (mode_w, mode_h) = mode.size();
    let output_size = SmithaySize::<i32, Physical>::from((mode_w as i32, mode_h as i32));
    let output_mode = OutputMode { size: (mode_w as i32, mode_h as i32).into(), refresh: mode.vrefresh() as i32 * 1000 };
    output.change_current_state(Some(output_mode), Some(Transform::Normal), Some(OutputScale::Integer(1)), Some((0, 0).into()));
    output.set_preferred(output_mode);
    let dh = display.handle();
    let _output_global = output.create_global::<State>(&dh);

    let viewport = Size::new(output_size.w as f64, output_size.h as f64);
    let ipc = ipc::spawn()?;
    let mut state = State::new(dh.clone(), config, output, viewport, ipc);

    let listener = ListeningSocket::bind(SOCKET_NAME)?;
    std::env::set_var("WAYLAND_DISPLAY", SOCKET_NAME);
    tracing::info!(socket = SOCKET_NAME, seat = %seat_name, "velo-de: listening for Wayland clients (standalone udev backend)");

    prepare_child_env();
    for cmd in state.config.autostart.clone() {
        spawn_shell(&cmd);
    }

    let keymap = build_keymap(&state);

    // Input: libinput on the same seat as the DRM device.
    let mut libinput_context = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    libinput_context.udev_assign_seat(&seat_name).map_err(|()| "velo-de: failed to assign libinput seat")?;
    let libinput_backend = LibinputInputBackend::new(libinput_context);

    // Hotplug monitoring for the primary GPU and other devices.
    let udev_backend = UdevBackend::new(&seat_name)?;

    state.running = true;
    let mut data = CalloopData {
        state,
        listener,
        display_handle: dh,
        keymap,
        last_frame: Instant::now(),
        udev: UdevData { session, primary_gpu_devnum, renderer, gbm_surface, output_size, pointer_location: (0.0, 0.0).into() },
    };

    // Wayland display socket: dispatch/flush on every readable event.
    loop_handle.insert_source(Generic::new(display, Interest::READ, CalloopMode::Level), |_, display, data: &mut CalloopData| {
        // SAFETY: `dispatch_clients`/`flush_clients` don't drop the
        // display's underlying I/O resources.
        let display = unsafe { display.get_mut() };
        display.dispatch_clients(&mut data.state)?;
        display.flush_clients()?;
        Ok(PostAction::Continue)
    })?;

    // Session pause/resume (VT switch away/back). v1: log only.
    loop_handle.insert_source(session_notifier, |event, _, _data| match event {
        SessionEvent::PauseSession => tracing::info!("velo-de: session paused"),
        SessionEvent::ActivateSession => tracing::info!("velo-de: session resumed"),
    })?;

    // udev hotplug: exit if the primary GPU disappears.
    loop_handle.insert_source(udev_backend, |event, _, data: &mut CalloopData| match event {
        UdevEvent::Added { device_id, path } => tracing::debug!(?device_id, ?path, "velo-de: udev device added"),
        UdevEvent::Changed { device_id } => tracing::debug!(?device_id, "velo-de: udev device changed"),
        UdevEvent::Removed { device_id } => {
            if device_id == data.udev.primary_gpu_devnum {
                tracing::error!("velo-de: primary GPU removed, exiting");
                data.state.running = false;
            }
        }
    })?;

    // DRM vblank: free the previous buffer and queue the next frame.
    loop_handle.insert_source(drm_notifier, |event, _, data: &mut CalloopData| match event {
        DrmEvent::VBlank(_crtc) => {
            let _ = data.udev.gbm_surface.frame_submitted();
            if let Err(err) = render(data) {
                tracing::error!(?err, "velo-de: render error");
            }
        }
        DrmEvent::Error(err) => tracing::error!(?err, "velo-de: drm error"),
    })?;

    // libinput: keyboard keybinds + basic pointer forwarding.
    loop_handle.insert_source(libinput_backend, |event, _, data: &mut CalloopData| match event {
        InputEvent::Keyboard { event } => {
            if let Some(action) = handle_keyboard(&mut data.state, &data.keymap, event.key_code(), event.state(), event.time_msec()) {
                data.state.apply_action(&action);
            }
        }
        InputEvent::PointerMotion { event } => {
            let delta = event.delta();
            let mut loc = data.udev.pointer_location + delta;
            loc.x = loc.x.clamp(0.0, data.udev.output_size.w as f64);
            loc.y = loc.y.clamp(0.0, data.udev.output_size.h as f64);
            data.udev.pointer_location = loc;
            data.state.cursor_pos = loc;

            if let Some(pointer) = data.state.seat.get_pointer() {
                let focus = focused_surface(&data.state).map(|surface| (surface, Point::from((0.0, 0.0))));
                pointer.motion(&mut data.state, focus, &MotionEvent { location: loc, serial: SERIAL_COUNTER.next_serial(), time: event.time_msec() });
                pointer.frame(&mut data.state);
            }
            refresh_keyboard_focus(&mut data.state);
        }
        InputEvent::PointerButton { event } => {
            if let Some(pointer) = data.state.seat.get_pointer() {
                pointer.button(&mut data.state, &ButtonEvent { serial: SERIAL_COUNTER.next_serial(), time: event.time_msec(), button: event.button_code(), state: event.state() });
                pointer.frame(&mut data.state);
            }
        }
        InputEvent::PointerAxis { event } => {
            if let Some(pointer) = data.state.seat.get_pointer() {
                let mut frame = AxisFrame::new(event.time_msec()).source(event.source());
                for axis in [InputAxis::Horizontal, InputAxis::Vertical] {
                    if let Some(value) = event.amount(axis) {
                        frame = frame.value(axis, value);
                    }
                    if let Some(v120) = event.amount_v120(axis) {
                        frame = frame.v120(axis, v120 as i32);
                    }
                }
                pointer.axis(&mut data.state, frame);
                pointer.frame(&mut data.state);
            }
        }
        _ => {}
    })?;

    // Kick off the vblank-driven render cycle with a first frame.
    render(&mut data)?;

    let signal = event_loop.get_signal();
    event_loop.run(Some(Duration::from_millis(16)), &mut data, |data| {
        let now = Instant::now();
        let dt = (now - data.last_frame).as_secs_f64();
        data.last_frame = now;
        data.state.tick(dt);
        data.state.process_ipc();
        refresh_keyboard_focus(&mut data.state);

        if let Ok(Some(stream)) = data.listener.accept() {
            let _ = data.display_handle.insert_client(stream, Arc::new(ClientState::default()));
        }

        if !data.state.running {
            signal.stop();
        }
    })?;

    Ok(())
}

/// The Spaces grid's currently-focused window's surface, if any — used as
/// the pointer-event target. Full pointer-to-surface hit-testing (e.g. for
/// click-to-focus) and cursor-sprite rendering are not implemented in v1.
fn focused_surface(state: &State) -> Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface> {
    let id = state.grid.focused_window()?;
    state.toplevel_for(id).map(|s| s.wl_surface().clone())
}

/// Point keyboard focus at the grid's current focused window. Called
/// periodically so typing always reaches the right client even if the
/// compositor hasn't received a pointer event recently. smithay skips
/// sending enter/leave when the focus hasn't changed.
fn refresh_keyboard_focus(state: &mut State) {
    let surface = focused_surface(state);
    if let Some(keyboard) = state.seat.get_keyboard() {
        keyboard.set_focus(state, surface, SERIAL_COUNTER.next_serial());
    }
}

/// Render one frame into the next GBM buffer and queue it for scanout.
fn render(data: &mut CalloopData) -> Result<(), Box<dyn std::error::Error>> {
    let (mut dmabuf, _age) = data.udev.gbm_surface.next_buffer()?;
    let mut framebuffer = data.udev.renderer.bind(&mut dmabuf)?;
    render_frame(&mut data.state, &mut data.udev.renderer, &mut framebuffer, data.udev.output_size, smithay::utils::Transform::Normal)?;

    let time = data.state.start_time.elapsed().as_millis() as u32;
    for (_, surface) in data.state.windows() {
        send_frame_callbacks(surface.wl_surface(), time);
    }

    let damage = Rectangle::from_size(data.udev.output_size);
    data.udev.gbm_surface.queue_buffer(None, Some(vec![damage]), ())?;
    Ok(())
}
