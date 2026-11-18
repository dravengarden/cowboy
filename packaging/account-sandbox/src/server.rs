//! The Unix socket server: one thread per connection, sequential
//! newline-delimited requests, `SCM_RIGHTS` replies and stop-on-close unit
//! ownership.

use std::io::{self, BufRead, BufReader, IoSlice, Write as _};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

use crate::daemon::Daemon;
use crate::protocol::{self, Code, Request, fail};

/// Accept connections forever.
pub fn serve(daemon: &Arc<Daemon>, listener: &UnixListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let daemon = Arc::clone(daemon);
                std::thread::spawn(move || handle_connection(&daemon, &stream));
            }
            Err(error) => eprintln!("cowboy-account-sandbox: accept: {error}"),
        }
    }
}

/// Serve one connection until it closes, then stop the units it owns.
pub fn handle_connection(daemon: &Arc<Daemon>, stream: &UnixStream) {
    let peer_uid = match rustix::net::sockopt::socket_peercred(stream) {
        Ok(credentials) => credentials.uid.as_raw(),
        Err(error) => {
            eprintln!("cowboy-account-sandbox: peer credentials: {error}");
            return;
        }
    };
    let conn = daemon.connect(peer_uid);
    let mut reader = BufReader::new(stream);
    loop {
        let line = match read_line(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                let _ = send(
                    stream,
                    &protocol::error_line(&fail(Code::InvalidRequest, error.to_string())),
                    &[],
                );
                break;
            }
        };
        let (bytes, fds, close) = match protocol::parse_request(&line) {
            Ok(request) => {
                let is_register = matches!(request, Request::RegisterRuntime { .. });
                match daemon.handle(&conn, request, &mut reader) {
                    Ok(reply) => (protocol::ok_line(&reply.value), reply.fds, false),
                    Err(failure) => (protocol::error_line(&failure), Vec::new(), is_register),
                }
            }
            Err(failure) => (protocol::error_line(&failure), Vec::new(), false),
        };
        if send(stream, &bytes, &fds).is_err() || close {
            break;
        }
    }
    daemon.disconnect(&conn);
}

/// One line without its newline, at most [`protocol::MAX_LINE_BYTES`];
/// `None` at a clean end of stream.
fn read_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::other("unterminated request line"))
            };
        }
        if let Some(position) = available.iter().position(|&b| b == b'\n') {
            line.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            if line.len() > protocol::MAX_LINE_BYTES {
                return Err(io::Error::other("request line too long"));
            }
            return Ok(Some(line));
        }
        let length = available.len();
        line.extend_from_slice(available);
        reader.consume(length);
        if line.len() > protocol::MAX_LINE_BYTES {
            return Err(io::Error::other("request line too long"));
        }
    }
}

/// Write `bytes`, attaching `fds` to the first byte.
fn send(stream: &UnixStream, bytes: &[u8], fds: &[OwnedFd]) -> io::Result<()> {
    let mut writer = stream;
    if fds.is_empty() {
        return writer.write_all(bytes);
    }
    let borrowed: Vec<BorrowedFd<'_>> = fds.iter().map(OwnedFd::as_fd).collect();
    let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(8))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !control.push(SendAncillaryMessage::ScmRights(&borrowed)) {
        return Err(io::Error::other("too many descriptors"));
    }
    let sent = rustix::net::sendmsg(
        stream,
        &[IoSlice::new(bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )?;
    writer.write_all(&bytes[sent..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_bounded() {
        let mut ok = BufReader::new(&b"{\"a\":1}\nrest"[..]);
        assert_eq!(read_line(&mut ok).unwrap().unwrap(), b"{\"a\":1}");
        assert!(read_line(&mut ok).is_err());
        let long = vec![b'x'; protocol::MAX_LINE_BYTES + 2];
        assert!(read_line(&mut BufReader::new(&long[..])).is_err());
        assert!(read_line(&mut BufReader::new(&b""[..])).unwrap().is_none());
    }
}
