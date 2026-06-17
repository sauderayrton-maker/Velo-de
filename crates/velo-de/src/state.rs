//! Compositor state: the smithay protocol handler glue, plus the live
//! [`Grid`] (Spaces model) and the mapping between [`WindowId`]s and
//! `xdg_toplevel` surfaces.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::io::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use smithay::input::pointer::CursorImageStatus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_buffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::utils::{Logical, Point, Rectangle, Serial, Size as SmithaySize, SERIAL_COUNTER};
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{with_states, CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::selection::primary_selection::{set_primary_focus, PrimarySelectionHandler, PrimarySelectionState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::dmabuf::{DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::presentation::PresentationState;
use smithay::wayland::single_pixel_buffer::SinglePixelBufferState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceCachedState, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgPopupSurfaceData, XdgShellHandler, XdgShellState, XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::{
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_dmabuf, delegate_layer_shell,
    delegate_output, delegate_presentation, delegate_primary_selection, delegate_seat, delegate_shm,
    delegate_single_pixel_buffer, delegate_viewporter, delegate_xdg_decoration, delegate_xdg_shell,
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::backend::allocator::dmabuf::Dmabuf;

use crate::ipc::{IpcChannels, IpcMessage};
use crate::shell::layer;
use velo_de_config::{Action, Config};
use velo_de_core::{place_window, Command, Event, Grid, IdGen, Size, WindowId};
use velo_de_ipc::{hypr_compat, Event as IpcEvent, Request as IpcRequest, Response as IpcResponse, SpaceInfo, WindowInfo};

/// Per-client compositor bookkeeping required by [`CompositorHandler`].
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// The whole compositor's state, threaded through every protocol handler.
pub struct State {
    #[allow(dead_code)]
    pub display_handle: DisplayHandle,
    pub start_time: Instant,
    pub running: bool,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    #[allow(dead_code)]
    pub output_manager_state: OutputManagerState,
    pub layer_shell_state: WlrLayerShellState,

    pub dmabuf_state: DmabufState,
    #[allow(dead_code)]
    pub viewporter_state: ViewporterState,
    #[allow(dead_code)]
    pub cursor_shape_state: CursorShapeManagerState,
    #[allow(dead_code)]
    pub single_pixel_buffer_state: SinglePixelBufferState,
    #[allow(dead_code)]
    pub presentation_state: PresentationState,
    #[allow(dead_code)]
    pub xdg_decoration_state: XdgDecorationState,

    pub seat: Seat<Self>,
    pub output: Output,

    pub config: Config,
    pub grid: Grid,
    id_gen: IdGen,
    windows: HashMap<WindowId, ToplevelSurface>,
    configured_sizes: HashMap<WindowId, (i32, i32)>,

    /// `(surface, layer, on-screen geometry)` for every live layer-shell
    /// surface, recomputed by [`Self::arrange_layers`].
    layer_entries: Vec<(LayerSurface, Layer, Rectangle<i32, Logical>)>,
    /// The [`Layer`] each live layer-shell surface was created on (from
    /// [`WlrLayerShellHandler::new_layer_surface`]'s `layer` argument).
    layer_kinds: HashMap<ObjectId, Layer>,
    configured_layer_sizes: HashMap<ObjectId, (i32, i32)>,
    /// The output area left over after subtracting layer-shell exclusive
    /// zones (e.g. a Velo-shell top bar) — this becomes the Spaces grid's
    /// viewport, offset by `usable_area.loc`.
    usable_area: Rectangle<i32, Logical>,
    /// The layer surface currently holding exclusive keyboard focus (e.g.
    /// an open Velo Launcher), if any.
    exclusive_keyboard_layer: Option<WlSurface>,

    /// Pending requests from `velo-de-ipc` clients, drained by
    /// [`Self::process_ipc`].
    ipc_requests: Receiver<IpcMessage>,
    /// Senders for connections that issued `Request::Subscribe`; each
    /// [`IpcEvent`] is pushed to all of these as the Grid changes.
    ipc_subscribers: Vec<Sender<IpcEvent>>,
    /// Newly-accepted Hyprland-event-socket-compat connections, drained by
    /// [`Self::process_ipc`].
    hypr_new_clients: Receiver<UnixStream>,
    /// Connected Hyprland-event-socket-compat clients (e.g. `Velo-shell`),
    /// written to as the Grid changes.
    hypr_clients: Vec<UnixStream>,
    /// The [`WindowInfo`] last broadcast as `IpcEvent::ActiveWindow`, so a
    /// later commit that fills in a title/app-id set after the toplevel was
    /// first mapped (and focused with an empty title) triggers a refresh.
    last_active_window: Option<WindowInfo>,

    /// Current pointer position in logical output coordinates, updated by
    /// the active backend on every pointer motion event.
    pub cursor_pos: Point<f64, Logical>,
    /// The cursor image currently set by the focused client (or the default
    /// arrow cursor when no client has called `wl_pointer.set_cursor`).
    pub cursor_status: CursorImageStatus,
    /// The loaded xcursor theme frames used to render the default arrow when
    /// no client has set a cursor surface.
    pub cursor_frames: crate::cursor::CursorFrames,

    /// Live `xdg_popup` surfaces (menus, tooltips, ...), kept so they can be
    /// configured, rendered and pruned each frame.
    popup_surfaces: Vec<PopupSurface>,
}

impl State {
    pub fn new(display_handle: DisplayHandle, config: Config, output: Output, viewport: Size, ipc: IpcChannels) -> Self {
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, Vec::new());
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&display_handle);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&display_handle);

        let dmabuf_state = DmabufState::new();
        let viewporter_state = ViewporterState::new::<Self>(&display_handle);
        let cursor_shape_state = CursorShapeManagerState::new::<Self>(&display_handle);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&display_handle);
        // CLOCK_MONOTONIC (1 on Linux) — the clock our frame-callback /
        // presentation timestamps are expressed against.
        const CLOCK_MONOTONIC: u32 = 1;
        let presentation_state = PresentationState::new::<Self>(&display_handle, CLOCK_MONOTONIC);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&display_handle);

        let mut seat = seat_state.new_wl_seat(&display_handle, "velo-de");
        let _ = seat.add_keyboard(Default::default(), config.key_repeat_delay_ms as i32, config.key_repeat_rate as i32);
        let _ = seat.add_pointer();

        let gap = config.gap;
        let usable_area = Rectangle::from_size(SmithaySize::from((viewport.w as i32, viewport.h as i32)));

        let mut state = Self {
            display_handle,
            start_time: Instant::now(),
            running: true,

            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            output_manager_state,
            layer_shell_state,

            dmabuf_state,
            viewporter_state,
            cursor_shape_state,
            single_pixel_buffer_state,
            presentation_state,
            xdg_decoration_state,

            seat,
            output,

            config,
            grid: Grid::new(viewport, gap),
            id_gen: IdGen::default(),
            windows: HashMap::new(),
            configured_sizes: HashMap::new(),

            layer_entries: Vec::new(),
            layer_kinds: HashMap::new(),
            configured_layer_sizes: HashMap::new(),
            usable_area,
            exclusive_keyboard_layer: None,

            ipc_requests: ipc.requests,
            ipc_subscribers: Vec::new(),
            hypr_new_clients: ipc.hypr_clients,
            hypr_clients: Vec::new(),
            last_active_window: None,

            cursor_pos: Point::from((0.0, 0.0)),
            cursor_status: CursorImageStatus::default_named(),
            cursor_frames: crate::cursor::CursorFrames::load_default(),

            popup_surfaces: Vec::new(),
        };
        state.arrange_layers();
        state
    }

    /// A newly-created `xdg_toplevel` becomes a column in the current
    /// Space's strip and is given keyboard focus.
    pub fn map_toplevel(&mut self, surface: ToplevelSurface) {
        let id = self.id_gen.next();
        self.windows.insert(id, surface.clone());

        surface.with_pending_state(|state| {
            state.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
        });

        self.run_grid(|grid| grid.add_window(id));
    }

    /// An `xdg_toplevel` was destroyed: drop it from the Spaces grid and
    /// re-flow the remaining columns.
    pub fn unmap_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some(id) = self.window_id_for(surface) else { return };
        self.windows.remove(&id);
        self.configured_sizes.remove(&id);

        self.run_grid(|grid| grid.remove_window(id));
    }

    fn window_id_for(&self, surface: &ToplevelSurface) -> Option<WindowId> {
        self.window_id_for_wl_surface(surface.wl_surface())
    }

    pub fn window_id_for_wl_surface(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows.iter().find(|(_, s)| s.wl_surface() == surface).map(|(id, _)| *id)
    }

    pub fn toplevel_for(&self, id: WindowId) -> Option<&ToplevelSurface> {
        self.windows.get(&id)
    }

    pub fn windows(&self) -> impl Iterator<Item = (&WindowId, &ToplevelSurface)> {
        self.windows.iter()
    }

    /// Live `xdg_popup` surfaces, for rendering and frame-callback delivery.
    pub fn popup_surfaces(&self) -> &[PopupSurface] {
        &self.popup_surfaces
    }

    /// Drop any `xdg_popup` surfaces whose client has destroyed them.
    pub fn prune_dead_popups(&mut self) {
        self.popup_surfaces.retain(|p| p.alive());
    }

    /// Find the Wayland surface under `pos` (output-relative logical coords)
    /// and its surface-local position. Searches: Top/Overlay layers → tiled
    /// windows (current space) → Bottom/Background layers. In this tiling WM
    /// the root surface is the hit target (subsurfaces stay within bounds).
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let usable = self.usable_area();
        let offset = (usable.loc.x as f64, usable.loc.y as f64);
        let viewport = self.grid.viewport();

        // Top/Overlay layers first.
        for (surface, layer, geometry) in self.layer_entries().iter().rev() {
            if !matches!(layer, Layer::Top | Layer::Overlay) {
                continue;
            }
            let geo = geometry.to_f64();
            if geo.contains(pos) {
                return Some((surface.wl_surface().clone(), pos - geo.loc));
            }
        }

        // Tiled windows in the current space.
        for frame in self.grid.frame() {
            if !frame.is_current {
                continue;
            }
            for w in &frame.windows {
                if !w.visible {
                    continue;
                }
                let screen = place_window(frame.rect, viewport, w.rect);
                let win_x = screen.x + offset.0;
                let win_y = screen.y + offset.1;
                if pos.x >= win_x && pos.x < win_x + screen.w && pos.y >= win_y && pos.y < win_y + screen.h {
                    let Some(toplevel) = self.toplevel_for(w.id) else { continue };
                    return Some((toplevel.wl_surface().clone(), Point::from((pos.x - win_x, pos.y - win_y))));
                }
            }
        }

        // Background/Bottom layers last.
        for (surface, layer, geometry) in self.layer_entries().iter().rev() {
            if !matches!(layer, Layer::Background | Layer::Bottom) {
                continue;
            }
            let geo = geometry.to_f64();
            if geo.contains(pos) {
                return Some((surface.wl_surface().clone(), pos - geo.loc));
            }
        }

        None
    }

    /// Output-relative logical position of a mapped surface (tiled toplevel
    /// or layer surface), used to place popups relative to their parent.
    pub fn surface_screen_loc(&self, surface: &WlSurface) -> Point<i32, Logical> {
        let offset = self.usable_area().loc;
        let viewport = self.grid.viewport();

        if let Some(id) = self.window_id_for_wl_surface(surface) {
            for frame in self.grid.frame() {
                if !frame.is_current {
                    continue;
                }
                for w in &frame.windows {
                    if w.id == id {
                        let screen = place_window(frame.rect, viewport, w.rect);
                        return Point::from(((screen.x as f64 + offset.x as f64) as i32, (screen.y as f64 + offset.y as f64) as i32));
                    }
                }
            }
        }

        for (ls, _, geometry) in self.layer_entries() {
            if ls.wl_surface() == surface {
                return geometry.loc;
            }
        }

        Point::from((0, 0))
    }

    /// Output-relative logical position at which a popup should be rendered:
    /// its parent's on-screen position plus the positioner geometry offset.
    pub fn popup_screen_loc(&self, popup: &PopupSurface) -> Point<i32, Logical> {
        let geometry = popup.with_pending_state(|state| state.geometry);
        let parent = popup
            .get_parent_surface()
            .or_else(|| with_states(popup.wl_surface(), |states| states.data_map.get::<XdgPopupSurfaceData>().and_then(|d| d.lock().ok()).and_then(|d| d.parent.clone())));

        match parent {
            Some(parent) => {
                // The parent may itself be a popup; walk up to an anchored
                // surface, accumulating each popup's offset.
                let parent_loc = if let Some(parent_popup) = self.popup_surfaces.iter().find(|p| p.wl_surface() == &parent) {
                    self.popup_screen_loc(parent_popup)
                } else {
                    self.surface_screen_loc(&parent)
                };
                Point::from((parent_loc.x + geometry.loc.x, parent_loc.y + geometry.loc.y))
            }
            None => geometry.loc,
        }
    }

    /// Apply a [`Command`] to the live Grid, re-flow layout, and act on the
    /// resulting [`Event`]s.
    pub fn apply_command(&mut self, command: Command) {
        self.run_grid(|grid| grid.apply(command));
    }

    /// Run a Grid mutation, re-flow layout, act on the resulting [`Event`]s,
    /// and emit `IpcEvent::CreateWorkspace` for any Space the mutation
    /// brought into existence (e.g. panning into an empty area of the grid).
    fn run_grid(&mut self, f: impl FnOnce(&mut Grid) -> Vec<Event>) {
        let before: HashSet<i32> = self.grid.space_infos().into_iter().map(|(id, _)| id).collect();
        let events = f(&mut self.grid);
        self.sync_layout();
        self.handle_events(events);
        for (id, _) in self.grid.space_infos() {
            if !before.contains(&id) {
                self.emit_ipc_event(IpcEvent::CreateWorkspace(id));
            }
        }
    }

    /// Advance Grid animations by `dt` seconds.
    pub fn tick(&mut self, dt: f64) {
        self.grid.tick(dt);
    }

    /// Run a keybinding's [`Action`]: either spawn a process / quit
    /// directly, or apply the corresponding Grid [`Command`].
    pub fn apply_action(&mut self, action: &Action) {
        match action {
            Action::Spawn(cmd) => spawn_shell(cmd),
            Action::SpawnTerminal => {
                let cmd = self.config.terminal.clone();
                spawn_shell(&cmd);
            }
            Action::Quit => self.running = false,
            _ => {
                if let Some(command) = action.to_command() {
                    self.apply_command(command);
                }
            }
        }
    }

    /// Tell all mapped windows their target size, sending an `xdg_toplevel`
    /// configure only when the size actually changed. Covers every Space so
    /// that a screen resize propagates to background spaces immediately, not
    /// only when the user visits them.
    pub fn sync_layout(&mut self) {
        let viewport = self.grid.viewport();
        let gap = self.grid.gap();
        let all_coords: Vec<(i32, i32)> = self.grid.space_coords().collect();
        for coord in all_coords {
            let layouts = match self.grid.space(coord) {
                Some(space) => space.layout(viewport, gap),
                None => continue,
            };
            for layout in layouts {
                let Some(surface) = self.windows.get(&layout.id) else { continue };
                let size = (layout.rect.w.round() as i32, layout.rect.h.round() as i32);
                if self.configured_sizes.get(&layout.id) == Some(&size) {
                    continue;
                }
                surface.with_pending_state(|state| {
                    state.size = Some(SmithaySize::from(size));
                });
                surface.send_configure();
                self.configured_sizes.insert(layout.id, size);
            }
        }
    }

    /// Recompute every layer-shell surface's on-screen geometry and the
    /// `usable_area` left over after subtracting exclusive zones (e.g. a
    /// Velo-shell top bar), feed `usable_area.size` into the Spaces grid's
    /// viewport, and send configure events to layer surfaces whose target
    /// size changed.
    ///
    /// Called on output resize and on every layer-surface commit.
    pub fn arrange_layers(&mut self) {
        let output_size = self.output.current_mode().map(|m| m.size.to_logical(1)).unwrap_or_else(|| SmithaySize::from((0, 0)));
        let output_rect = Rectangle::from_size(output_size);

        let mut usable = output_rect;
        let mut entries = Vec::new();

        for &target_layer in &[Layer::Background, Layer::Bottom, Layer::Top, Layer::Overlay] {
            for surface in self.layer_shell_state.layer_surfaces() {
                if !surface.alive() {
                    continue;
                }
                let id = surface.wl_surface().id();
                if self.layer_kinds.get(&id) != Some(&target_layer) {
                    continue;
                }

                let cached = with_states(surface.wl_surface(), |states| *states.cached_state.get::<LayerSurfaceCachedState>().current());

                let geometry = layer::arrange(output_rect, cached.anchor, cached.size, cached.margin);

                let size = (geometry.size.w, geometry.size.h);
                if self.configured_layer_sizes.get(&id) != Some(&size) {
                    surface.with_pending_state(|pending| {
                        pending.size = Some(SmithaySize::from(size));
                    });
                    surface.send_configure();
                    self.configured_layer_sizes.insert(id, size);
                }

                usable = layer::shrink_by_exclusive_zone(usable, cached.anchor, cached.exclusive_zone);
                entries.push((surface.clone(), target_layer, geometry));
            }
        }

        self.layer_entries = entries;
        self.usable_area = usable;
        self.grid.set_viewport(Size::new(usable.size.w as f64, usable.size.h as f64));
        self.sync_layout();
        self.sync_layer_focus();
    }

    /// Every live layer-shell surface with its [`Layer`] and on-screen
    /// geometry, as last computed by [`Self::arrange_layers`].
    pub fn layer_entries(&self) -> &[(LayerSurface, Layer, Rectangle<i32, Logical>)] {
        &self.layer_entries
    }

    /// The output area left over after subtracting layer-shell exclusive
    /// zones; Space/window content is offset by `usable_area.loc` and sized
    /// to `usable_area.size`.
    pub fn usable_area(&self) -> Rectangle<i32, Logical> {
        self.usable_area
    }

    /// Give keyboard focus to the topmost [`KeyboardInteractivity::Exclusive`]
    /// layer surface (e.g. an open Velo Launcher), or restore focus to the
    /// Grid's focused window once no layer surface claims it.
    fn sync_layer_focus(&mut self) {
        let exclusive = self.layer_entries.iter().rev().find_map(|(surface, _, _)| {
            let interactivity = with_states(surface.wl_surface(), |states| states.cached_state.get::<LayerSurfaceCachedState>().current().keyboard_interactivity);
            (interactivity == KeyboardInteractivity::Exclusive).then(|| surface.wl_surface().clone())
        });

        if exclusive == self.exclusive_keyboard_layer {
            return;
        }

        let target = exclusive.clone().or_else(|| self.grid.focused_window().and_then(|id| self.windows.get(&id)).map(|s| s.wl_surface().clone()));

        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
        }
        self.exclusive_keyboard_layer = exclusive;
    }

    /// React to side effects of Grid commands: move keyboard focus, ask
    /// clients to close, and notify `velo-de-ipc` subscribers (and thus
    /// `Velo-shell`'s Hyprland-event-compat socket) of bar-facing changes.
    fn handle_events(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::FocusChanged(id) => {
                    let surface = id.and_then(|id| self.windows.get(&id)).map(|s| s.wl_surface().clone());
                    let info = surface.as_ref().map(window_info).unwrap_or_else(|| WindowInfo { title: String::new(), class: String::new() });
                    if let Some(keyboard) = self.seat.get_keyboard() {
                        keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
                    }
                    self.last_active_window = Some(info.clone());
                    self.emit_ipc_event(IpcEvent::ActiveWindow(info));
                }
                Event::CloseRequested(id) => {
                    if let Some(surface) = self.windows.get(&id) {
                        surface.send_close();
                    }
                }
                Event::SpaceChanged(id) => self.emit_ipc_event(IpcEvent::Workspace(id)),
                Event::SpacesChanged => self.emit_ipc_event(IpcEvent::SpacesChanged),
            }
        }
    }

    /// Drain pending `velo-de-ipc` requests (answering them against the live
    /// Grid) and newly-accepted Hyprland-event-compat connections. Called
    /// once per frame from the backend's main loop.
    pub fn process_ipc(&mut self) {
        self.prune_dead_popups();
        while let Ok(client) = self.hypr_new_clients.try_recv() {
            self.hypr_clients.push(client);
        }
        while let Ok(msg) = self.ipc_requests.try_recv() {
            match msg {
                IpcMessage::Request(req, resp_tx) => {
                    let resp = self.handle_ipc_request(req);
                    let _ = resp_tx.send(resp);
                }
                IpcMessage::Subscribe(tx) => self.ipc_subscribers.push(tx),
            }
        }
    }

    fn handle_ipc_request(&mut self, req: IpcRequest) -> IpcResponse {
        match req {
            IpcRequest::GetSpaces => IpcResponse::Spaces(self.space_infos()),
            IpcRequest::GetActiveSpace => {
                let current = self.grid.current_space_id();
                let info = self.space_infos().into_iter().find(|s| s.id == current).expect("current space is always in space_infos");
                IpcResponse::ActiveSpace(info)
            }
            IpcRequest::GetActiveWindow => IpcResponse::ActiveWindow(self.active_window_info()),
            IpcRequest::Dispatch(cmd) => {
                self.apply_command(cmd.into());
                IpcResponse::Ok
            }
            IpcRequest::Subscribe => IpcResponse::Subscribed,
        }
    }

    /// Every Space in Hyprland-`workspaces`-compatible shape.
    fn space_infos(&self) -> Vec<SpaceInfo> {
        let current = self.grid.current_space_id();
        self.grid
            .space_infos()
            .into_iter()
            .map(|(id, windows)| SpaceInfo { id, coord: self.grid.coord_for_id(id).unwrap_or_default(), windows, focused: id == current })
            .collect()
    }

    fn active_window_info(&self) -> Option<WindowInfo> {
        let id = self.grid.focused_window()?;
        Some(window_info(self.windows.get(&id)?.wl_surface()))
    }

    /// Push `event` to every `velo-de-ipc` subscriber, and to every
    /// Hyprland-event-compat client if it has a plain-text equivalent (see
    /// [`hypr_compat::format_event`]).
    fn emit_ipc_event(&mut self, event: IpcEvent) {
        self.ipc_subscribers.retain(|tx| tx.send(event.clone()).is_ok());
        if let Some(line) = hypr_compat::format_event(&event) {
            self.hypr_clients.retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
        }
    }
}

/// This toplevel's title/app-id, in Hyprland-`activewindow`-compatible
/// shape (empty strings if unset, matching Hyprland's `activewindow>>,`
/// for "no active window").
fn window_info(surface: &WlSurface) -> WindowInfo {
    with_states(surface, |states| {
        let attrs = states.data_map.get::<XdgToplevelSurfaceData>().expect("xdg_toplevel surface has XdgToplevelSurfaceData").lock().unwrap();
        WindowInfo { title: attrs.title.clone().unwrap_or_default(), class: attrs.app_id.clone().unwrap_or_default() }
    })
}

// ---- protocol handler glue -------------------------------------------------

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

        let is_layer_surface = with_states(surface, |states| states.data_map.get::<LayerSurfaceData>().is_some());
        if is_layer_surface {
            self.arrange_layers();
            return;
        }

        // A toplevel may call `set_title`/`set_app_id` after it was first
        // mapped (and focused with an empty title); if this commit belongs
        // to the focused window and its info changed, refresh subscribers.
        if let Some(id) = self.window_id_for_wl_surface(surface) {
            if Some(id) == self.grid.focused_window() {
                let info = window_info(surface);
                if Some(&info) != self.last_active_window.as_ref() {
                    self.last_active_window = Some(info.clone());
                    self.emit_ipc_event(IpcEvent::ActiveWindow(info));
                }
            }
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| s.client());
        set_primary_focus(&self.display_handle, seat, client);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

// Required by `delegate_cursor_shape!` (it also covers tablet cursor shapes);
// velo-de has no tablet support, so the default no-op is sufficient.
impl smithay::wayland::tablet_manager::TabletSeatHandler for State {}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &smithay::wayland::dmabuf::DmabufGlobal, _dmabuf: Dmabuf, notifier: ImportNotifier) {
        // The GlesRenderer imports the dmabuf lazily during rendering; just
        // acknowledge the buffer as accepted here.
        let _ = notifier.successful::<State>();
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        // velo-de draws no server-side decorations; tell clients to use CSD.
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ClientSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ClientSide);
        });
        toplevel.send_configure();
    }

    fn unset_mode(&mut self, _toplevel: ToplevelSurface) {}
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.send_configure();
        self.map_toplevel(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let geometry = positioner.get_geometry();
        surface.with_pending_state(|state| {
            state.geometry = geometry;
        });
        let _ = surface.send_configure();
        self.popup_surfaces.push(surface);
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        self.popup_surfaces.retain(|p| p.alive() && p.wl_surface() != surface.wl_surface());
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {}

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
        });
        surface.send_repositioned(token);
        let _ = surface.send_configure();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.unmap_toplevel(&surface);
    }
}

impl OutputHandler for State {}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: LayerSurface, _output: Option<WlOutput>, layer: Layer, namespace: String) {
        tracing::info!(namespace, ?layer, "new layer surface");
        self.layer_kinds.insert(surface.wl_surface().id(), layer);
        self.arrange_layers();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let id = surface.wl_surface().id();
        self.layer_kinds.remove(&id);
        self.configured_layer_sizes.remove(&id);
        if self.exclusive_keyboard_layer.as_ref() == Some(surface.wl_surface()) {
            self.exclusive_keyboard_layer = None;
        }
        self.arrange_layers();
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);
delegate_xdg_shell!(State);
delegate_output!(State);
delegate_layer_shell!(State);
delegate_dmabuf!(State);
delegate_viewporter!(State);
delegate_cursor_shape!(State);
delegate_single_pixel_buffer!(State);
delegate_presentation!(State);
delegate_xdg_decoration!(State);

/// Run `cmd` via `sh -c`, inheriting the compositor's environment (notably
/// `WAYLAND_DISPLAY`) so spawned apps connect to this compositor.
pub fn spawn_shell(cmd: &str) {
    if let Err(err) = std::process::Command::new("sh").arg("-c").arg(cmd).spawn() {
        tracing::warn!("failed to spawn `{cmd}`: {err}");
    }
}
