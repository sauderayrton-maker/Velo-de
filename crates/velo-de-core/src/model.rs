//! The "Spaces" model: a sparse 2D grid of Spaces, each holding a
//! horizontally-scrollable strip of columns. See the crate-level docs for
//! the overall idea.

use std::collections::{BTreeSet, HashMap};

use crate::anim::Animated;
use crate::geometry::{Rect, Size, Vec2};
use crate::layout::{compute_strip_layout, WindowLayout};

/// Default fraction of the viewport width a newly-created column occupies.
pub const DEFAULT_WIDTH_FRAC: f64 = 0.5;
const MIN_WIDTH_FRAC: f64 = 0.2;
const MAX_WIDTH_FRAC: f64 = 1.0;

/// How large an Overview tile is, as a fraction of the full viewport.
const OVERVIEW_TILE_SCALE: f64 = 0.28;
/// Gap between Overview tiles, as a fraction of the viewport dimension.
const OVERVIEW_GAP_FRAC: f64 = 0.06;
/// How many Spaces around the current/selected one are included (and thus
/// rendered, even if empty) while Overview is active.
const OVERVIEW_RADIUS: i32 = 1;

/// Opaque identifier for a mapped window/surface, allocated by the
/// compositor via [`IdGen`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// Monotonic [`WindowId`] allocator.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdGen(u64);

impl IdGen {
    pub fn next(&mut self) -> WindowId {
        self.0 += 1;
        WindowId(self.0)
    }
}

/// How the windows inside a [`Column`] share its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnLayout {
    /// Stacked vertically, each visible at once.
    Split,
    /// Only the active window is shown, full-column.
    Tabbed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn delta(self) -> (i32, i32) {
        match self {
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
        }
    }
}

/// A vertical slot in a [`Strip`], holding one or more stacked windows.
#[derive(Debug, Clone)]
pub struct Column {
    pub windows: Vec<WindowId>,
    pub active: usize,
    pub layout: ColumnLayout,
    pub width_frac: f64,
}

impl Column {
    fn new(window: WindowId) -> Self {
        Self { windows: vec![window], active: 0, layout: ColumnLayout::Split, width_frac: DEFAULT_WIDTH_FRAC }
    }

    pub fn active_window(&self) -> Option<WindowId> {
        self.windows.get(self.active).copied()
    }
}

/// A horizontally-scrollable sequence of [`Column`]s — one Space's content.
#[derive(Debug, Clone)]
pub struct Strip {
    pub columns: Vec<Column>,
    pub active: usize,
    pub scroll: Animated<f64>,
}

impl Strip {
    fn new() -> Self {
        Self { columns: Vec::new(), active: 0, scroll: Animated::new(0.0) }
    }

    pub fn active_column(&self) -> Option<&Column> {
        self.columns.get(self.active)
    }

    fn active_column_mut(&mut self) -> Option<&mut Column> {
        self.columns.get_mut(self.active)
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.active_column().and_then(Column::active_window)
    }

    /// Nudge the scroll target so the active column is fully on-screen,
    /// moving the minimum distance necessary (niri-style "follow focus").
    fn ensure_active_visible(&mut self, viewport: Size, gap: f64) {
        let Some(active) = self.columns.get(self.active) else { return };

        let mut local_x = gap;
        for column in &self.columns[..self.active] {
            local_x += viewport.w * column.width_frac + gap;
        }
        let col_w = viewport.w * active.width_frac;

        let scroll = self.scroll.target();
        let left = local_x - scroll;
        let right = left + col_w;

        if left < 0.0 {
            self.scroll.set_target(local_x);
        } else if right > viewport.w {
            self.scroll.set_target(local_x + col_w - viewport.w);
        }
    }
}

/// One cell of the Spaces grid.
#[derive(Debug, Clone)]
pub struct Space {
    pub strip: Strip,
    pub fullscreen: Option<WindowId>,
}

impl Space {
    fn new() -> Self {
        Self { strip: Strip::new(), fullscreen: None }
    }

    pub fn window_count(&self) -> usize {
        self.strip.columns.iter().map(|c| c.windows.len()).sum()
    }

    /// Lay out this Space's windows within a `viewport`-sized local
    /// coordinate space. If a window is fullscreened, everything else
    /// collapses to an empty/hidden rect.
    pub fn layout(&self, viewport: Size, gap: f64) -> Vec<WindowLayout> {
        if let Some(fullscreen_id) = self.fullscreen {
            return self
                .strip
                .columns
                .iter()
                .flat_map(|c| c.windows.iter().copied())
                .map(|id| WindowLayout {
                    id,
                    rect: if id == fullscreen_id { Rect::new(0.0, 0.0, viewport.w, viewport.h) } else { Rect::default() },
                    visible: id == fullscreen_id,
                })
                .collect();
        }

        compute_strip_layout(&self.strip, viewport, gap)
    }
}

/// State of the zoomed-out, spatial view of the whole Spaces grid.
#[derive(Debug, Clone)]
pub struct Overview {
    pub active: bool,
    pub selection: (i32, i32),
    pub zoom: Animated<f64>,
}

impl Overview {
    fn new(selection: (i32, i32)) -> Self {
        Self { active: false, selection, zoom: Animated::new(0.0) }
    }
}

/// Side effects of applying a [`Command`] that the compositor needs to act
/// on (focus a surface, send a close request, refresh the bar, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The window that should receive keyboard focus changed (or there is
    /// now no focused window).
    FocusChanged(Option<WindowId>),
    /// The active Space changed; carries its stable (Hyprland-shaped) id.
    SpaceChanged(i32),
    /// A Space was created/destroyed or a window count changed - bar
    /// modules (workspace pills) should refresh.
    SpacesChanged,
    /// The compositor should ask this window's client to close.
    CloseRequested(WindowId),
}

/// User-facing actions, normally produced by keybindings (see
/// `velo-de-config`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    FocusColumn(Direction),
    MoveColumn(Direction),
    SwitchSpace(Direction),
    MoveWindowToSpace(Direction),
    CycleWindow,
    ToggleColumnLayout,
    ToggleFullscreen,
    CloseFocused,
    /// Multiplies the focused column's width by this factor.
    ResizeColumn(NotNan),
    ToggleOverview,
    OverviewMove(Direction),
    OverviewConfirm,
    OverviewCancel,
    FocusSpaceById(i32),
}

/// A small `f64` wrapper that is `Eq`/`Hash` so [`Command`] can derive them
/// (needed to use `Command` as e.g. a config/keybind map value). The value
/// is always a finite resize factor (e.g. `1.1` or `1.0 / 1.1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotNan(f64);

impl NotNan {
    pub fn new(value: f64) -> Self {
        assert!(value.is_finite(), "resize factor must be finite");
        Self(value)
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl Eq for NotNan {}

impl std::hash::Hash for NotNan {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// The placement of one Space's content for the current frame, in
/// output-relative pixel coordinates. `windows` are laid out in the Space's
/// own `viewport`-sized local coordinate system (see
/// [`Space::layout`]); the compositor maps a window's local rect `w` into
/// `rect` via:
///
/// ```text
/// screen_x = rect.x + w.x * (rect.w / viewport.w)
/// screen_y = rect.y + w.y * (rect.h / viewport.h)
/// screen_w = w.w * (rect.w / viewport.w)
/// screen_h = w.h * (rect.h / viewport.h)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceFrame {
    pub coord: (i32, i32),
    /// `None` for Overview placeholder tiles that don't exist yet.
    pub space_id: Option<i32>,
    pub rect: Rect,
    pub windows: Vec<WindowLayout>,
    pub is_current: bool,
    pub is_overview_selection: bool,
}

/// The whole desktop: a sparse grid of [`Space`]s plus camera/Overview
/// animation state.
#[derive(Debug, Clone)]
pub struct Grid {
    spaces: HashMap<(i32, i32), Space>,
    current: (i32, i32),
    /// Camera position in *grid units* (i.e. `(1.0, 0.0)` means "centered
    /// on the Space to the right of the origin"). Converted to pixels via
    /// [`Grid::pan_pixels`].
    pan: Animated<Vec2>,
    overview: Overview,
    viewport: Size,
    gap: f64,
    next_id: i32,
    id_by_coord: HashMap<(i32, i32), i32>,
    coord_by_id: HashMap<i32, (i32, i32)>,
}

impl Grid {
    pub fn new(viewport: Size, gap: f64) -> Self {
        let mut grid = Self {
            spaces: HashMap::new(),
            current: (0, 0),
            pan: Animated::new(Vec2::ZERO),
            overview: Overview::new((0, 0)),
            viewport,
            gap,
            next_id: 1,
            id_by_coord: HashMap::new(),
            coord_by_id: HashMap::new(),
        };
        grid.ensure_space((0, 0));
        grid
    }

    // ---- accessors -----------------------------------------------------

    pub fn current_coord(&self) -> (i32, i32) {
        self.current
    }

    pub fn space(&self, coord: (i32, i32)) -> Option<&Space> {
        self.spaces.get(&coord)
    }

    pub fn current_space(&self) -> &Space {
        self.spaces.get(&self.current).expect("current space always exists")
    }

    fn current_space_mut(&mut self) -> &mut Space {
        self.spaces.get_mut(&self.current).expect("current space always exists")
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        let space = self.current_space();
        space.fullscreen.or_else(|| space.strip.focused_window())
    }

    pub fn space_id(&self, coord: (i32, i32)) -> Option<i32> {
        self.id_by_coord.get(&coord).copied()
    }

    pub fn coord_for_id(&self, id: i32) -> Option<(i32, i32)> {
        self.coord_by_id.get(&id).copied()
    }

    pub fn current_space_id(&self) -> i32 {
        self.id_by_coord[&self.current]
    }

    /// `(id, window_count)` for every existing Space - feeds the
    /// `hyprctl -j workspaces` compat response.
    pub fn space_infos(&self) -> Vec<(i32, usize)> {
        self.spaces.iter().filter_map(|(coord, space)| self.id_by_coord.get(coord).map(|&id| (id, space.window_count()))).collect()
    }

    pub fn is_settled(&self) -> bool {
        self.pan.is_settled() && self.overview.zoom.is_settled() && self.spaces.get(&self.current).is_none_or(|s| s.strip.scroll.is_settled())
    }

    pub fn overview_active(&self) -> bool {
        self.overview.active
    }

    pub fn viewport(&self) -> Size {
        self.viewport
    }

    pub fn gap(&self) -> f64 {
        self.gap
    }

    // ---- configuration ---------------------------------------------------

    pub fn set_viewport(&mut self, viewport: Size) {
        self.viewport = viewport;
        let gap = self.gap;
        if let Some(space) = self.spaces.get_mut(&self.current) {
            space.strip.ensure_active_visible(viewport, gap);
        }
    }

    pub fn set_gap(&mut self, gap: f64) {
        self.gap = gap;
        let viewport = self.viewport;
        if let Some(space) = self.spaces.get_mut(&self.current) {
            space.strip.ensure_active_visible(viewport, gap);
        }
    }

    // ---- animation --------------------------------------------------------

    pub fn tick(&mut self, dt: f64) {
        self.pan.tick(dt);
        self.overview.zoom.tick(dt);
        if let Some(space) = self.spaces.get_mut(&self.current) {
            space.strip.scroll.tick(dt);
        }
    }

    // ---- window lifecycle ---------------------------------------------

    /// Insert `id` as a new column immediately after the focused one in the
    /// current Space, and focus it.
    pub fn add_window(&mut self, id: WindowId) -> Vec<Event> {
        let viewport = self.viewport;
        let gap = self.gap;
        let space = self.current_space_mut();

        let insert_at = if space.strip.columns.is_empty() { 0 } else { space.strip.active + 1 };
        space.strip.columns.insert(insert_at, Column::new(id));
        space.strip.active = insert_at;
        space.strip.ensure_active_visible(viewport, gap);

        vec![Event::FocusChanged(Some(id)), Event::SpacesChanged]
    }

    /// Remove `id` from wherever it lives. Returns events for the
    /// compositor to refresh focus/bar state.
    pub fn remove_window(&mut self, id: WindowId) -> Vec<Event> {
        let Some((coord, col_idx, win_idx)) = self.locate(id) else { return Vec::new() };
        let was_current = coord == self.current;
        let viewport = self.viewport;
        let gap = self.gap;

        let space = self.spaces.get_mut(&coord).expect("located space exists");
        if space.fullscreen == Some(id) {
            space.fullscreen = None;
        }

        let column = &mut space.strip.columns[col_idx];
        column.windows.remove(win_idx);
        if !column.windows.is_empty() && column.active >= column.windows.len() {
            column.active = column.windows.len() - 1;
        }

        if column.windows.is_empty() {
            space.strip.columns.remove(col_idx);
            if !space.strip.columns.is_empty() && space.strip.active >= space.strip.columns.len() {
                space.strip.active = space.strip.columns.len() - 1;
            }
        }

        if was_current {
            space.strip.ensure_active_visible(viewport, gap);
        }

        let mut events = vec![Event::SpacesChanged];
        if was_current {
            events.push(Event::FocusChanged(self.focused_window()));
        }
        events
    }

    fn locate(&self, id: WindowId) -> Option<((i32, i32), usize, usize)> {
        for (&coord, space) in &self.spaces {
            for (col_idx, column) in space.strip.columns.iter().enumerate() {
                if let Some(win_idx) = column.windows.iter().position(|&w| w == id) {
                    return Some((coord, col_idx, win_idx));
                }
            }
        }
        None
    }

    // ---- commands -------------------------------------------------------

    pub fn apply(&mut self, cmd: Command) -> Vec<Event> {
        match cmd {
            Command::FocusColumn(dir) => self.focus_column(dir),
            Command::MoveColumn(dir) => self.move_column(dir),
            Command::SwitchSpace(dir) => self.switch_space(dir),
            Command::MoveWindowToSpace(dir) => self.move_window_to_space(dir),
            Command::CycleWindow => self.cycle_window(),
            Command::ToggleColumnLayout => self.toggle_column_layout(),
            Command::ToggleFullscreen => self.toggle_fullscreen(),
            Command::CloseFocused => self.close_focused(),
            Command::ResizeColumn(factor) => self.resize_column(factor.get()),
            Command::ToggleOverview => self.toggle_overview(),
            Command::OverviewMove(dir) => self.overview_move(dir),
            Command::OverviewConfirm => self.overview_confirm(),
            Command::OverviewCancel => self.overview_cancel(),
            Command::FocusSpaceById(id) => self.focus_space_by_id(id),
        }
    }

    fn focus_column(&mut self, dir: Direction) -> Vec<Event> {
        if !matches!(dir, Direction::Left | Direction::Right) {
            return Vec::new();
        }
        let viewport = self.viewport;
        let gap = self.gap;
        let space = self.current_space_mut();
        if space.strip.columns.is_empty() {
            return Vec::new();
        }
        let delta = if dir == Direction::Left { -1 } else { 1 };
        let new_active = (space.strip.active as i32 + delta).clamp(0, space.strip.columns.len() as i32 - 1) as usize;
        if new_active == space.strip.active {
            return Vec::new();
        }
        space.strip.active = new_active;
        space.strip.ensure_active_visible(viewport, gap);
        vec![Event::FocusChanged(self.focused_window())]
    }

    fn move_column(&mut self, dir: Direction) -> Vec<Event> {
        if !matches!(dir, Direction::Left | Direction::Right) {
            return Vec::new();
        }
        let viewport = self.viewport;
        let gap = self.gap;

        let space = self.current_space_mut();
        if space.strip.columns.is_empty() {
            return Vec::new();
        }
        let idx = space.strip.active;
        let delta = if dir == Direction::Left { -1 } else { 1 };
        let target_idx = idx as i32 + delta;

        if target_idx >= 0 && (target_idx as usize) < space.strip.columns.len() {
            space.strip.columns.swap(idx, target_idx as usize);
            space.strip.active = target_idx as usize;
            space.strip.ensure_active_visible(viewport, gap);
            return vec![Event::FocusChanged(self.focused_window())];
        }

        // At the edge of the strip: throw the column into the adjacent Space.
        let column = space.strip.columns.remove(idx);
        if !space.strip.columns.is_empty() && space.strip.active >= space.strip.columns.len() {
            space.strip.active = space.strip.columns.len() - 1;
        }
        space.strip.ensure_active_visible(viewport, gap);

        let (dc, dr) = dir.delta();
        let target_coord = (self.current.0 + dc, self.current.1 + dr);
        let is_new = !self.spaces.contains_key(&target_coord);
        self.ensure_space(target_coord);

        let target = self.spaces.get_mut(&target_coord).expect("just ensured");
        // Arriving from the left edge of `target` if we were thrown right,
        // and vice versa, so the column lands on the side closest to home.
        let insert_at = if dir == Direction::Right { 0 } else { target.strip.columns.len() };
        target.strip.columns.insert(insert_at, column);
        target.strip.active = insert_at;
        target.strip.ensure_active_visible(viewport, gap);

        self.go_to_space(target_coord);

        let mut events = vec![Event::SpacesChanged, Event::SpaceChanged(self.current_space_id())];
        if is_new {
            events.push(Event::SpacesChanged);
        }
        events.push(Event::FocusChanged(self.focused_window()));
        events
    }

    fn switch_space(&mut self, dir: Direction) -> Vec<Event> {
        let (dc, dr) = dir.delta();
        let target = (self.current.0 + dc, self.current.1 + dr);
        let is_new = !self.spaces.contains_key(&target);
        self.ensure_space(target);
        self.go_to_space(target);

        let mut events = vec![Event::SpaceChanged(self.current_space_id())];
        if is_new {
            events.push(Event::SpacesChanged);
        }
        events.push(Event::FocusChanged(self.focused_window()));
        events
    }

    fn move_window_to_space(&mut self, dir: Direction) -> Vec<Event> {
        let viewport = self.viewport;
        let gap = self.gap;

        let space = self.current_space_mut();
        if space.strip.columns.is_empty() {
            return Vec::new();
        }
        let idx = space.strip.active;
        let column = space.strip.columns.remove(idx);
        if !space.strip.columns.is_empty() && space.strip.active >= space.strip.columns.len() {
            space.strip.active = space.strip.columns.len() - 1;
        }
        if space.fullscreen.is_some_and(|f| column.windows.contains(&f)) {
            space.fullscreen = None;
        }
        space.strip.ensure_active_visible(viewport, gap);

        let (dc, dr) = dir.delta();
        let target_coord = (self.current.0 + dc, self.current.1 + dr);
        let is_new = !self.spaces.contains_key(&target_coord);
        self.ensure_space(target_coord);

        let target = self.spaces.get_mut(&target_coord).expect("just ensured");
        let insert_at = target.strip.columns.len();
        target.strip.columns.insert(insert_at, column);
        target.strip.active = insert_at;
        target.strip.ensure_active_visible(viewport, gap);

        self.go_to_space(target_coord);

        let mut events = vec![Event::SpacesChanged, Event::SpaceChanged(self.current_space_id())];
        if is_new {
            events.push(Event::SpacesChanged);
        }
        events.push(Event::FocusChanged(self.focused_window()));
        events
    }

    fn cycle_window(&mut self) -> Vec<Event> {
        let current_focus = self.focused_window();
        let viewport = self.viewport;
        let gap = self.gap;
        let space = self.current_space_mut();
        if space.fullscreen.is_some() {
            return Vec::new();
        }

        let mut flat = Vec::new();
        for (col_idx, column) in space.strip.columns.iter().enumerate() {
            for win_idx in 0..column.windows.len() {
                flat.push((col_idx, win_idx));
            }
        }
        if flat.len() < 2 {
            return Vec::new();
        }

        let current_pos = flat.iter().position(|&(ci, wi)| Some(space.strip.columns[ci].windows[wi]) == current_focus).unwrap_or(0);
        let (next_col, next_win) = flat[(current_pos + 1) % flat.len()];

        space.strip.active = next_col;
        space.strip.columns[next_col].active = next_win;
        space.strip.ensure_active_visible(viewport, gap);

        vec![Event::FocusChanged(self.focused_window())]
    }

    fn toggle_column_layout(&mut self) -> Vec<Event> {
        let space = self.current_space_mut();
        if let Some(column) = space.strip.active_column_mut() {
            column.layout = match column.layout {
                ColumnLayout::Split => ColumnLayout::Tabbed,
                ColumnLayout::Tabbed => ColumnLayout::Split,
            };
        }
        Vec::new()
    }

    fn toggle_fullscreen(&mut self) -> Vec<Event> {
        let focused = self.focused_window();
        let space = self.current_space_mut();
        match (space.fullscreen, focused) {
            (Some(_), _) => space.fullscreen = None,
            (None, Some(id)) => space.fullscreen = Some(id),
            (None, None) => {}
        }
        Vec::new()
    }

    fn close_focused(&mut self) -> Vec<Event> {
        match self.focused_window() {
            Some(id) => vec![Event::CloseRequested(id)],
            None => Vec::new(),
        }
    }

    fn resize_column(&mut self, factor: f64) -> Vec<Event> {
        let viewport = self.viewport;
        let gap = self.gap;
        let space = self.current_space_mut();
        if let Some(column) = space.strip.active_column_mut() {
            column.width_frac = (column.width_frac * factor).clamp(MIN_WIDTH_FRAC, MAX_WIDTH_FRAC);
        }
        space.strip.ensure_active_visible(viewport, gap);
        Vec::new()
    }

    fn toggle_overview(&mut self) -> Vec<Event> {
        self.overview.active = !self.overview.active;
        if self.overview.active {
            self.overview.selection = self.current;
            self.overview.zoom.set_target(1.0);
        } else {
            self.overview.zoom.set_target(0.0);
        }
        Vec::new()
    }

    fn overview_move(&mut self, dir: Direction) -> Vec<Event> {
        if !self.overview.active {
            return Vec::new();
        }
        let (dc, dr) = dir.delta();
        let target = (self.overview.selection.0 + dc, self.overview.selection.1 + dr);
        self.ensure_space(target);
        self.overview.selection = target;
        Vec::new()
    }

    fn overview_confirm(&mut self) -> Vec<Event> {
        if !self.overview.active {
            return Vec::new();
        }
        let target = self.overview.selection;
        self.overview.active = false;
        self.overview.zoom.set_target(0.0);

        if target == self.current {
            return Vec::new();
        }
        self.go_to_space(target);
        vec![Event::SpaceChanged(self.current_space_id()), Event::FocusChanged(self.focused_window())]
    }

    fn overview_cancel(&mut self) -> Vec<Event> {
        self.overview.active = false;
        self.overview.selection = self.current;
        self.overview.zoom.set_target(0.0);
        Vec::new()
    }

    fn focus_space_by_id(&mut self, id: i32) -> Vec<Event> {
        if let Some(coord) = self.coord_for_id(id) {
            if coord == self.current {
                return Vec::new();
            }
            self.go_to_space(coord);
            return vec![Event::SpaceChanged(self.current_space_id()), Event::FocusChanged(self.focused_window())];
        }

        // Unknown id: e.g. Velo-shell clicked one of its placeholder pills
        // for a Space that doesn't exist yet. Create one along row 0 and
        // register it under the requested id so the pill becomes real.
        let mut col = 0;
        while self.spaces.contains_key(&(col, 0)) {
            col += 1;
        }
        let coord = (col, 0);
        self.spaces.insert(coord, Space::new());
        self.id_by_coord.insert(coord, id);
        self.coord_by_id.insert(id, coord);
        self.next_id = self.next_id.max(id + 1);

        self.go_to_space(coord);
        vec![Event::SpacesChanged, Event::SpaceChanged(self.current_space_id()), Event::FocusChanged(self.focused_window())]
    }

    fn ensure_space(&mut self, coord: (i32, i32)) -> i32 {
        if !self.spaces.contains_key(&coord) {
            self.spaces.insert(coord, Space::new());
            let id = self.next_id;
            self.next_id += 1;
            self.id_by_coord.insert(coord, id);
            self.coord_by_id.insert(id, coord);
        }
        self.id_by_coord[&coord]
    }

    fn go_to_space(&mut self, coord: (i32, i32)) {
        self.current = coord;
        self.pan.set_target(Vec2::new(coord.0 as f64, coord.1 as f64));
    }

    // ---- rendering --------------------------------------------------------

    /// Camera offset in pixels (see [`Grid::pan`]).
    pub fn pan_pixels(&self) -> Vec2 {
        let p = self.pan.value();
        Vec2::new(p.x * (self.viewport.w + self.gap), p.y * (self.viewport.h + self.gap))
    }

    pub fn overview_zoom(&self) -> f64 {
        self.overview.zoom.value()
    }

    pub fn overview_selection(&self) -> (i32, i32) {
        self.overview.selection
    }

    /// A Space's placement when tiled normally (panned into view).
    fn tiled_rect(&self, coord: (i32, i32)) -> Rect {
        let pan = self.pan_pixels();
        let x = coord.0 as f64 * (self.viewport.w + self.gap) - pan.x;
        let y = coord.1 as f64 * (self.viewport.h + self.gap) - pan.y;
        Rect::new(x, y, self.viewport.w, self.viewport.h)
    }

    /// A Space's placement as an Overview tile, arranged spatially around
    /// the current selection.
    fn overview_rect(&self, coord: (i32, i32)) -> Rect {
        let tile_w = self.viewport.w * OVERVIEW_TILE_SCALE;
        let tile_h = self.viewport.h * OVERVIEW_TILE_SCALE;
        let gap_x = self.viewport.w * OVERVIEW_GAP_FRAC;
        let gap_y = self.viewport.h * OVERVIEW_GAP_FRAC;

        let center = self.overview.selection;
        let dx = (coord.0 - center.0) as f64;
        let dy = (coord.1 - center.1) as f64;

        let cx = self.viewport.w / 2.0 - tile_w / 2.0;
        let cy = self.viewport.h / 2.0 - tile_h / 2.0;

        Rect::new(cx + dx * (tile_w + gap_x), cy + dy * (tile_h + gap_y), tile_w, tile_h)
    }

    /// Compute every Space's placement for the current frame: the current
    /// Space, plus whatever else is visible mid-pan or in Overview.
    pub fn frame(&self) -> Vec<SpaceFrame> {
        let zoom = self.overview_zoom();
        let mut coords = BTreeSet::new();

        if zoom > 0.0 {
            for &coord in self.spaces.keys() {
                coords.insert(coord);
            }
            let center = self.overview.selection;
            for dc in -OVERVIEW_RADIUS..=OVERVIEW_RADIUS {
                for dr in -OVERVIEW_RADIUS..=OVERVIEW_RADIUS {
                    coords.insert((center.0 + dc, center.1 + dr));
                }
            }
        } else {
            let pan = self.pan.value();
            for &x in &[pan.x.floor() as i32, pan.x.ceil() as i32] {
                for &y in &[pan.y.floor() as i32, pan.y.ceil() as i32] {
                    coords.insert((x, y));
                }
            }
            coords.insert(self.current);
        }

        coords
            .into_iter()
            .map(|coord| {
                let tiled = self.tiled_rect(coord);
                let overview = self.overview_rect(coord);
                let rect = Rect::lerp(tiled, overview, zoom);
                let windows = self.spaces.get(&coord).map(|s| s.layout(self.viewport, self.gap)).unwrap_or_default();
                SpaceFrame {
                    coord,
                    space_id: self.space_id(coord),
                    rect,
                    windows,
                    is_current: coord == self.current,
                    is_overview_selection: self.overview.active && coord == self.overview.selection,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        Grid::new(Size::new(1000.0, 800.0), 10.0)
    }

    #[test]
    fn new_grid_has_one_space_with_id_1() {
        let g = grid();
        assert_eq!(g.current_coord(), (0, 0));
        assert_eq!(g.current_space_id(), 1);
        assert_eq!(g.space_infos(), vec![(1, 0)]);
    }

    #[test]
    fn add_window_focuses_it_and_appends_column() {
        let mut g = grid();
        let id = WindowId(1);
        let events = g.add_window(id);
        assert_eq!(events, vec![Event::FocusChanged(Some(id)), Event::SpacesChanged]);
        assert_eq!(g.focused_window(), Some(id));
        assert_eq!(g.current_space().strip.columns.len(), 1);
        assert_eq!(g.space_infos(), vec![(1, 1)]);
    }

    #[test]
    fn focus_column_moves_between_windows() {
        let mut g = grid();
        let a = WindowId(1);
        let b = WindowId(2);
        g.add_window(a);
        g.add_window(b);
        assert_eq!(g.focused_window(), Some(b));

        g.apply(Command::FocusColumn(Direction::Left));
        assert_eq!(g.focused_window(), Some(a));

        // clamped at the edge: no further change
        let events = g.apply(Command::FocusColumn(Direction::Left));
        assert!(events.is_empty());
        assert_eq!(g.focused_window(), Some(a));
    }

    #[test]
    fn switch_space_creates_and_navigates_grid() {
        let mut g = grid();
        let a = WindowId(1);
        g.add_window(a);

        let events = g.apply(Command::SwitchSpace(Direction::Right));
        assert_eq!(g.current_coord(), (1, 0));
        assert_eq!(g.current_space_id(), 2);
        assert!(events.contains(&Event::SpacesChanged));
        assert!(events.contains(&Event::SpaceChanged(2)));
        assert_eq!(g.focused_window(), None);

        // pan target should now point at the new space (1 grid unit right)
        let mut g2 = g.clone();
        for _ in 0..1000 {
            g2.tick(1.0 / 60.0);
            if g2.is_settled() {
                break;
            }
        }
        assert!((g2.pan_pixels().x - 1010.0).abs() < 1e-3); // viewport.w + gap
        assert_eq!(g2.pan_pixels().y, 0.0);

        // and back
        g.apply(Command::SwitchSpace(Direction::Left));
        assert_eq!(g.current_coord(), (0, 0));
        assert_eq!(g.focused_window(), Some(a));
    }

    #[test]
    fn move_column_throws_to_adjacent_space_at_edge() {
        let mut g = grid();
        let a = WindowId(1);
        g.add_window(a);

        // only one column, already at the edge -> thrown right
        let events = g.apply(Command::MoveColumn(Direction::Right));
        assert_eq!(g.current_coord(), (1, 0));
        assert_eq!(g.focused_window(), Some(a));
        assert!(events.contains(&Event::SpaceChanged(2)));

        // original space is now empty but still exists
        assert_eq!(g.space(( 0, 0)).unwrap().window_count(), 0);
        assert_eq!(g.current_space().window_count(), 1);
    }

    #[test]
    fn move_window_to_space_moves_and_follows() {
        let mut g = grid();
        let a = WindowId(1);
        let b = WindowId(2);
        g.add_window(a);
        g.add_window(b);
        // focus a
        g.apply(Command::FocusColumn(Direction::Left));
        assert_eq!(g.focused_window(), Some(a));

        g.apply(Command::MoveWindowToSpace(Direction::Down));
        assert_eq!(g.current_coord(), (0, 1));
        assert_eq!(g.focused_window(), Some(a));
        assert_eq!(g.current_space().window_count(), 1);

        g.apply(Command::SwitchSpace(Direction::Up));
        assert_eq!(g.current_coord(), (0, 0));
        assert_eq!(g.current_space().window_count(), 1);
        assert_eq!(g.focused_window(), Some(b));
    }

    #[test]
    fn cycle_window_wraps_across_columns() {
        let mut g = grid();
        let a = WindowId(1);
        let b = WindowId(2);
        let c = WindowId(3);
        g.add_window(a);
        g.add_window(b);
        g.add_window(c);
        assert_eq!(g.focused_window(), Some(c));

        g.apply(Command::CycleWindow);
        assert_eq!(g.focused_window(), Some(a));
        g.apply(Command::CycleWindow);
        assert_eq!(g.focused_window(), Some(b));
        g.apply(Command::CycleWindow);
        assert_eq!(g.focused_window(), Some(c));
    }

    #[test]
    fn toggle_fullscreen_overrides_layout() {
        let mut g = grid();
        let a = WindowId(1);
        let b = WindowId(2);
        g.add_window(a);
        g.add_window(b);

        g.apply(Command::ToggleFullscreen);
        let layouts = g.current_space().layout(Size::new(1000.0, 800.0), 10.0);
        let b_layout = layouts.iter().find(|w| w.id == b).unwrap();
        assert!(b_layout.visible);
        assert_eq!(b_layout.rect, Rect::new(0.0, 0.0, 1000.0, 800.0));

        let a_layout = layouts.iter().find(|w| w.id == a).unwrap();
        assert!(!a_layout.visible);

        g.apply(Command::ToggleFullscreen);
        let layouts = g.current_space().layout(Size::new(1000.0, 800.0), 10.0);
        assert!(layouts.iter().all(|w| w.rect.w < 1000.0 || w.rect.h < 800.0));
    }

    #[test]
    fn close_focused_requests_close_without_removing() {
        let mut g = grid();
        let a = WindowId(1);
        g.add_window(a);
        let events = g.apply(Command::CloseFocused);
        assert_eq!(events, vec![Event::CloseRequested(a)]);
        assert_eq!(g.current_space().window_count(), 1);
    }

    #[test]
    fn remove_window_clears_focus_when_empty() {
        let mut g = grid();
        let a = WindowId(1);
        g.add_window(a);
        let events = g.remove_window(a);
        assert!(events.contains(&Event::FocusChanged(None)));
        assert_eq!(g.current_space().window_count(), 0);
        assert_eq!(g.focused_window(), None);
    }

    #[test]
    fn overview_navigation_and_confirm() {
        let mut g = grid();
        let a = WindowId(1);
        g.add_window(a);

        g.apply(Command::ToggleOverview);
        assert!(g.overview_active());
        assert_eq!(g.overview_selection(), (0, 0));

        g.apply(Command::OverviewMove(Direction::Right));
        assert_eq!(g.overview_selection(), (1, 0));
        // selecting creates a placeholder so it can render as an empty tile
        assert!(g.space((1, 0)).is_some());

        g.apply(Command::OverviewConfirm);
        assert!(!g.overview_active());
        assert_eq!(g.current_coord(), (1, 0));
    }

    #[test]
    fn overview_cancel_keeps_current_space() {
        let mut g = grid();
        g.apply(Command::ToggleOverview);
        g.apply(Command::OverviewMove(Direction::Down));
        g.apply(Command::OverviewCancel);
        assert!(!g.overview_active());
        assert_eq!(g.current_coord(), (0, 0));
    }

    #[test]
    fn focus_space_by_id_jumps_to_existing() {
        let mut g = grid();
        g.apply(Command::SwitchSpace(Direction::Right)); // creates id 2 at (1,0)
        g.apply(Command::SwitchSpace(Direction::Left)); // back to (0,0)/id 1

        g.apply(Command::FocusSpaceById(2));
        assert_eq!(g.current_coord(), (1, 0));
    }

    #[test]
    fn focus_space_by_id_creates_placeholder_for_unknown_id() {
        let mut g = grid();
        // velo-shell shows pills 1..=10 by default; clicking pill 5 when
        // only space 1 exists should create+register space 5.
        g.apply(Command::FocusSpaceById(5));
        assert_eq!(g.space_id(g.current_coord()), Some(5));
        assert_eq!(g.coord_for_id(5), Some(g.current_coord()));
    }

    #[test]
    fn strip_layout_splits_columns_with_gaps() {
        let mut g = grid();
        g.add_window(WindowId(1));
        g.add_window(WindowId(2));

        let layouts = g.current_space().layout(Size::new(1000.0, 800.0), 10.0);
        assert_eq!(layouts.len(), 2);

        // each column is 50% of 1000 = 500px wide; first starts at gap=10
        let first = layouts.iter().find(|w| w.id == WindowId(1)).unwrap();
        assert_eq!(first.rect.x, 10.0);
        assert_eq!(first.rect.w, 500.0);
        assert_eq!(first.rect.y, 10.0);
        assert_eq!(first.rect.h, 780.0);

        let second = layouts.iter().find(|w| w.id == WindowId(2)).unwrap();
        assert_eq!(second.rect.x, 10.0 + 500.0 + 10.0);
        assert_eq!(second.rect.w, 500.0);
    }

    #[test]
    fn frame_current_space_is_full_viewport_when_settled() {
        let mut g = grid();
        g.add_window(WindowId(1));

        let frame = g.frame();
        let current = frame.iter().find(|f| f.is_current).unwrap();
        assert_eq!(current.coord, (0, 0));
        assert_eq!(current.rect, Rect::new(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(current.space_id, Some(1));
    }

    #[test]
    fn frame_includes_overview_tiles_when_active() {
        let mut g = grid();
        g.apply(Command::ToggleOverview);
        // settle the zoom animation
        for _ in 0..1000 {
            g.tick(1.0 / 60.0);
            if g.is_settled() {
                break;
            }
        }

        let frame = g.frame();
        // 3x3 ring around the selection, all at the same (small) tile size
        assert!(frame.len() >= 9);
        let current = frame.iter().find(|f| f.is_current).unwrap();
        assert!((current.rect.w - 1000.0 * OVERVIEW_TILE_SCALE).abs() < 1e-6);
        assert!(current.is_overview_selection);
    }

    #[test]
    fn resize_column_clamped() {
        let mut g = grid();
        g.add_window(WindowId(1));

        for _ in 0..50 {
            g.apply(Command::ResizeColumn(NotNan::new(1.5)));
        }
        assert!((g.current_space().strip.columns[0].width_frac - MAX_WIDTH_FRAC).abs() < 1e-9);

        for _ in 0..50 {
            g.apply(Command::ResizeColumn(NotNan::new(1.0 / 1.5)));
        }
        assert!((g.current_space().strip.columns[0].width_frac - MIN_WIDTH_FRAC).abs() < 1e-9);
    }
}
