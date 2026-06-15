//! Newline-delimited-JSON framing and a small blocking client, used by
//! `velo-msg` and `velo-hyprctl` to talk to `velo-de`'s IPC socket.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::paths::socket_path;
use crate::protocol::{Request, Response};

/// Write a single newline-delimited JSON message.
pub fn write_message<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let mut json = serde_json::to_string(msg).map_err(io::Error::other)?;
    json.push('\n');
    w.write_all(json.as_bytes())
}

/// Read a single newline-delimited JSON message, or `Ok(None)` at EOF.
pub fn read_message<R: BufRead, T: DeserializeOwned>(r: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(line.trim_end()).map(Some).map_err(io::Error::other)
}

/// A blocking connection to `velo-de`'s IPC socket at [`socket_path`].
pub struct Client {
    reader: BufReader<UnixStream>,
}

impl Client {
    pub fn connect() -> io::Result<Self> {
        let stream = UnixStream::connect(socket_path())?;
        Ok(Self { reader: BufReader::new(stream) })
    }

    /// Send a [`Request`] and block for its [`Response`].
    pub fn request(&mut self, req: &Request) -> io::Result<Response> {
        write_message(self.reader.get_mut(), req)?;
        read_message(&mut self.reader)?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "velo-de closed the connection"))
    }
}
