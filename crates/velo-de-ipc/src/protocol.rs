//! Wire types for `velo-de`'s native IPC protocol: newline-delimited JSON
//! [`Request`]/[`Response`] pairs over the socket at [`crate::socket_path`],
//! plus an out-of-band [`Event`] stream for `Request::Subscribe`.
//!
//! These mirror (but are independent of) `velo-de-core`'s `Command`/
//! `Direction`/`Event` types, via `From` conversions, so the wire format
//! stays stable even if the internal model changes.

use serde::{Deserialize, Serialize};

/// A grid-relative direction, as in `velo_de_core::Direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl From<Direction> for velo_de_core::Direction {
    fn from(d: Direction) -> Self {
        match d {
            Direction::Left => velo_de_core::Direction::Left,
            Direction::Right => velo_de_core::Direction::Right,
            Direction::Up => velo_de_core::Direction::Up,
            Direction::Down => velo_de_core::Direction::Down,
        }
    }
}

impl From<velo_de_core::Direction> for Direction {
    fn from(d: velo_de_core::Direction) -> Self {
        match d {
            velo_de_core::Direction::Left => Direction::Left,
            velo_de_core::Direction::Right => Direction::Right,
            velo_de_core::Direction::Up => Direction::Up,
            velo_de_core::Direction::Down => Direction::Down,
        }
    }
}

/// A command to apply to the focused output's Spaces grid, as in
/// `velo_de_core::Command`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    FocusColumn(Direction),
    MoveColumn(Direction),
    SwitchSpace(Direction),
    MoveWindowToSpace(Direction),
    CycleWindow,
    ToggleColumnLayout,
    ToggleFullscreen,
    CloseFocused,
    /// Multiply the focused column's width by this factor.
    ResizeColumn(f64),
    ToggleOverview,
    OverviewMove(Direction),
    OverviewConfirm,
    OverviewCancel,
    /// Jump to (or create) the Space with this Hyprland-shaped id.
    FocusSpaceById(i32),
}

impl From<Command> for velo_de_core::Command {
    fn from(c: Command) -> Self {
        use velo_de_core::Command as C;
        match c {
            Command::FocusColumn(d) => C::FocusColumn(d.into()),
            Command::MoveColumn(d) => C::MoveColumn(d.into()),
            Command::SwitchSpace(d) => C::SwitchSpace(d.into()),
            Command::MoveWindowToSpace(d) => C::MoveWindowToSpace(d.into()),
            Command::CycleWindow => C::CycleWindow,
            Command::ToggleColumnLayout => C::ToggleColumnLayout,
            Command::ToggleFullscreen => C::ToggleFullscreen,
            Command::CloseFocused => C::CloseFocused,
            Command::ResizeColumn(factor) => C::ResizeColumn(velo_de_core::NotNan::new(factor)),
            Command::ToggleOverview => C::ToggleOverview,
            Command::OverviewMove(d) => C::OverviewMove(d.into()),
            Command::OverviewConfirm => C::OverviewConfirm,
            Command::OverviewCancel => C::OverviewCancel,
            Command::FocusSpaceById(id) => C::FocusSpaceById(id),
        }
    }
}

/// One Space, described in Hyprland-`workspaces`-compatible shape (see
/// `velo-hyprctl`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceInfo {
    /// Stable, sequential, 1-based id (Hyprland-shaped "workspace id").
    pub id: i32,
    /// This Space's position in the Spaces grid.
    pub coord: (i32, i32),
    /// Number of mapped windows in this Space.
    pub windows: usize,
    /// Whether this is the currently-focused Space.
    pub focused: bool,
}

/// The currently-focused window, in Hyprland-`activewindow`-compatible
/// shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub class: String,
}

/// A request sent to `velo-de`'s IPC socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// All Spaces in the current grid (Hyprland `-j workspaces`).
    GetSpaces,
    /// The currently-focused Space (Hyprland `-j activeworkspace`).
    GetActiveSpace,
    /// The currently-focused window (Hyprland `-j activewindow`).
    GetActiveWindow,
    /// Apply a [`Command`] to the focused output's grid.
    Dispatch(Command),
    /// Subscribe to the [`Event`] stream; the server switches this
    /// connection to emit one JSON [`Event`] per line until it closes.
    Subscribe,
}

/// A response to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Spaces(Vec<SpaceInfo>),
    ActiveSpace(SpaceInfo),
    ActiveWindow(Option<WindowInfo>),
    Ok,
    Err(String),
    /// Sent once in reply to [`Request::Subscribe`] before the connection
    /// switches to streaming [`Event`]s.
    Subscribed,
}

/// An asynchronous notification, streamed (one JSON value per line) after
/// [`Request::Subscribe`]. Mirrors the Hyprland-shaped events
/// `Velo-shell` listens for; see `velo-de-ipc::hypr_compat` for the
/// plain-text translation used by the Hyprland event-socket shim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// The focused Space changed to this (Hyprland-shaped) id.
    Workspace(i32),
    /// A Space was created (Hyprland-shaped id).
    CreateWorkspace(i32),
    /// A Space was destroyed (Hyprland-shaped id).
    DestroyWorkspace(i32),
    /// The focused window changed.
    ActiveWindow(WindowInfo),
    /// Spaces or their window counts changed; bar modules should refresh.
    SpacesChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trips_through_core() {
        let cmd = Command::ResizeColumn(1.1);
        let core: velo_de_core::Command = cmd.into();
        assert_eq!(core, velo_de_core::Command::ResizeColumn(velo_de_core::NotNan::new(1.1)));
    }

    #[test]
    fn request_serde_round_trip() {
        let req = Request::Dispatch(Command::FocusColumn(Direction::Left));
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn response_serde_round_trip() {
        let resp = Response::Spaces(vec![SpaceInfo { id: 1, coord: (0, 0), windows: 2, focused: true }]);
        let json = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, back);
    }
}
