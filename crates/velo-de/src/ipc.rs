//! Bridges `velo-de-ipc`'s protocol into the live compositor.
//!
//! [`spawn`] binds two Unix sockets and hands their accept loops to
//! background threads:
//!
//! - `velo_de_ipc::socket_path()`: the native request/response protocol.
//!   Each connection's requests are forwarded to the main loop (with a
//!   response channel) via [`IpcMessage::Request`]; a `Request::Subscribe`
//!   connection instead registers an event sender
//!   ([`IpcMessage::Subscribe`]) and streams [`Event`]s as the main loop
//!   produces them.
//! - `velo_de_ipc::hypr_event_socket_path()`: the Hyprland-event-socket
//!   compat path read verbatim by `Velo-shell/src/hypr.rs::subscribe()`.
//!   Newly-accepted connections are handed to the main loop, which writes
//!   `hypr_compat`-formatted lines to each as the Grid changes.
//!
//! Both accept loops only ever touch the sockets; all Grid access happens on
//! the main loop via [`crate::state::State::process_ipc`].

use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use velo_de_ipc::{hypr_event_socket_path, read_message, socket_path, write_message, Event, Request, Response};

/// A message from an IPC connection thread to the main loop.
pub enum IpcMessage {
    /// A request awaiting a response on `Sender<Response>`.
    Request(Request, Sender<Response>),
    /// A `Request::Subscribe` connection registering for the [`Event`] stream.
    Subscribe(Sender<Event>),
}

/// Background-thread channels for both IPC sockets, created by [`spawn`].
pub struct IpcChannels {
    pub requests: Receiver<IpcMessage>,
    pub hypr_clients: Receiver<UnixStream>,
}

/// Bind both sockets and spawn their accept threads.
pub fn spawn() -> std::io::Result<IpcChannels> {
    Ok(IpcChannels { requests: spawn_native_socket()?, hypr_clients: spawn_hypr_socket()? })
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
}

fn spawn_native_socket() -> std::io::Result<Receiver<IpcMessage>> {
    let listener = bind(&socket_path())?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || handle_native_client(stream, tx));
        }
    });
    Ok(rx)
}

/// Read [`Request`]s from `stream` and forward them to the main loop,
/// writing back whatever [`Response`] it produces. Switches to streaming
/// [`Event`]s for the lifetime of the connection on `Request::Subscribe`.
fn handle_native_client(stream: UnixStream, tx: Sender<IpcMessage>) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);

    loop {
        let req: Request = match read_message(&mut reader) {
            Ok(Some(req)) => req,
            _ => return,
        };

        if matches!(req, Request::Subscribe) {
            let (event_tx, event_rx) = mpsc::channel();
            if tx.send(IpcMessage::Subscribe(event_tx)).is_err() || write_message(&mut writer, &Response::Subscribed).is_err() {
                return;
            }
            for event in event_rx {
                if write_message(&mut writer, &event).is_err() {
                    return;
                }
            }
            return;
        }

        let (resp_tx, resp_rx) = mpsc::channel();
        if tx.send(IpcMessage::Request(req, resp_tx)).is_err() {
            return;
        }
        let Ok(resp) = resp_rx.recv() else { return };
        if write_message(&mut writer, &resp).is_err() {
            return;
        }
    }
}

fn spawn_hypr_socket() -> std::io::Result<Receiver<UnixStream>> {
    let listener = bind(&hypr_event_socket_path())?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if tx.send(stream).is_err() {
                return;
            }
        }
    });
    Ok(rx)
}
