//! `velo-de`'s native IPC protocol (newline-delimited JSON over a unix
//! socket) plus a blocking client and the Hyprland-event-socket-compatible
//! formatting used so `Velo-shell` works unmodified.

pub mod client;
pub mod hypr_compat;
pub mod paths;
pub mod protocol;

pub use client::{read_message, write_message, Client};
pub use paths::{hypr_event_socket_path, socket_path, HYPRLAND_INSTANCE_SIGNATURE};
pub use protocol::{Command, Direction, Event, Request, Response, SpaceInfo, WindowInfo};
