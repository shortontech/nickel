//! Bounded local request transport for ordinary session clients.

use crate::{ClientEnvelope, MAX_FRAME_BYTES, Request, ServerEnvelope, ServerMessage};
use std::{
    io,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
struct ReplyPath(PathBuf);
impl Drop for ReplyPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Absence of a session is distinct from an unavailable configured authority.
/// Never fall back to an unrelated host session or expose its capability in diagnostics.
pub fn request_from_environment(
    request: Request,
    timeout: Duration,
) -> io::Result<Option<ServerMessage>> {
    let Some(server) = std::env::var_os("NICKEL_SESSION_CONTROL") else {
        return Ok(None);
    };
    let token = std::env::var("NICKEL_SESSION_TOKEN").map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "session capability unavailable",
        )
    })?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "session runtime unavailable"))?;
    request_at(
        Path::new(&server),
        Path::new(&runtime),
        token,
        request,
        timeout,
    )
    .map(Some)
}

fn request_at(
    server: &Path,
    runtime: &Path,
    token: String,
    request: Request,
    timeout: Duration,
) -> io::Result<ServerMessage> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let path = runtime.join(format!("ni-{:x}-{id:x}.sock", std::process::id()));
    // Binding is exclusive; do not unlink an existing path to acquire it.
    let socket = UnixDatagram::bind(&path)?;
    let _cleanup = ReplyPath(path);
    socket.connect(server)?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    let frame = crate::encode(&ClientEnvelope {
        token,
        request_id: id,
        request,
    })
    .map_err(io::Error::other)?;
    socket.send(&frame)?;
    let mut buffer = vec![0; MAX_FRAME_BYTES];
    let count = socket.recv(&mut buffer)?;
    let envelope = crate::decode::<ServerEnvelope>(&buffer[..count]).map_err(io::Error::other)?;
    if envelope.request_id != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session response correlation mismatch",
        ));
    }
    Ok(envelope.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlated_reply_and_failure_both_remove_the_reply_socket() {
        let root = std::env::temp_dir().join(format!(
            "nickel-protocol-{}-{}",
            std::process::id(),
            NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for mismatch in [false, true] {
            let path = root.join("server");
            let socket = UnixDatagram::bind(&path).unwrap();
            let server = std::thread::spawn(move || {
                let mut buffer = vec![0; MAX_FRAME_BYTES];
                let (length, peer) = socket.recv_from(&mut buffer).unwrap();
                let request: ClientEnvelope = crate::decode(&buffer[..length]).unwrap();
                let reply = ServerEnvelope {
                    request_id: request.request_id + u64::from(mismatch),
                    message: ServerMessage::OnScreenKeyboard(Default::default()),
                };
                socket
                    .send_to(&crate::encode(&reply).unwrap(), peer.as_pathname().unwrap())
                    .unwrap();
            });
            let result = request_at(
                &path,
                &root,
                "test".into(),
                Request::Query(crate::Query::OnScreenKeyboard),
                Duration::from_secs(1),
            );
            assert_eq!(result.is_err(), mismatch);
            server.join().unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        }
        std::fs::remove_dir(root).unwrap();
    }
}
