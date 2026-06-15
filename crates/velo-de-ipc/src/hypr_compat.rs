//! Plain-text formatting for Hyprland's `.socket2.sock` event protocol
//! (`event>>data\n`), as consumed by `Velo-shell/src/hypr.rs::subscribe()`.
//!
//! Pure string formatting, no I/O: `velo-de` formats each [`Event`] it
//! emits and appends the result to its `.socket2.sock` clients.

use crate::protocol::{Event, WindowInfo};

/// Format an [`Event`] as a Hyprland event-socket line (including the
/// trailing `\n`), or `None` for events with no Hyprland equivalent.
pub fn format_event(event: &Event) -> Option<String> {
    match event {
        Event::Workspace(id) => Some(format!("workspace>>{id}\n")),
        Event::CreateWorkspace(id) => Some(format!("createworkspace>>{id}\n")),
        Event::DestroyWorkspace(id) => Some(format!("destroyworkspace>>{id}\n")),
        Event::ActiveWindow(WindowInfo { class, title }) => Some(format!("activewindow>>{class},{title}\n")),
        // No mapped events change Hyprland's per-workspace window counts in
        // a way Velo-shell needs a dedicated line for; it re-queries
        // `hyprctl -j workspaces` on `workspace>>`/`createworkspace>>` etc.
        Event::SpacesChanged => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_workspace_event() {
        assert_eq!(format_event(&Event::Workspace(3)).unwrap(), "workspace>>3\n");
    }

    #[test]
    fn formats_active_window_event() {
        let win = WindowInfo { class: "velo-browser".into(), title: "New Tab".into() };
        assert_eq!(format_event(&Event::ActiveWindow(win)).unwrap(), "activewindow>>velo-browser,New Tab\n");
    }

    #[test]
    fn spaces_changed_has_no_hyprland_equivalent() {
        assert_eq!(format_event(&Event::SpacesChanged), None);
    }
}
