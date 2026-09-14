//! Nonblocking named-pipe adapter for a Windows-owned local admission executor.
//!
//! Pipe creation deliberately requires caller-supplied security attributes. This module will not
//! silently substitute the process default DACL for a current-user-only ACL.

use crate::{ClientEnvelope, MAX_FRAME_BYTES, ServerEnvelope};
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
        },
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_NOWAIT,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PeekNamedPipe,
        },
    },
    core::PCWSTR,
};

const PIPE_INSTANCES: u32 = 1;

/// One current-user-secured pipe instance, polled only by its owning admission executor.
pub struct WindowsPipeServer {
    handle: HANDLE,
    capability: String,
    connected: bool,
    input: Vec<u8>,
}

impl WindowsPipeServer {
    /// Creates a local byte-mode pipe. `security` must contain a current-user-only DACL prepared by
    /// the owning process; its descriptor must remain alive for this call.
    pub fn bind(
        endpoint: &OsStr,
        capability: String,
        security: &SECURITY_ATTRIBUTES,
    ) -> io::Result<Self> {
        if capability.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "empty session capability",
            ));
        }
        let endpoint = wide(endpoint);
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(endpoint.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_INSTANCES,
                MAX_FRAME_BYTES as u32,
                MAX_FRAME_BYTES as u32,
                0,
                Some(security),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            handle,
            capability,
            connected: false,
            input: Vec::with_capacity(crate::FRAME_HEADER_BYTES),
        })
    }

    /// Reads at most one complete authenticated request without blocking the admission executor.
    pub fn poll(&mut self) -> io::Result<Option<ClientEnvelope>> {
        if !self.connected {
            match unsafe { ConnectNamedPipe(self.handle, None) } {
                Ok(()) => self.connected = true,
                Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => {
                    self.connected = true
                }
                Err(_) => return Ok(None),
            }
        }

        let mut available = 0;
        if unsafe { PeekNamedPipe(self.handle, None, 0, None, Some(&mut available), None) }.is_err()
        {
            self.reset()?;
            return Ok(None);
        }
        if available == 0 {
            return Ok(None);
        }
        let mut chunk = [0_u8; 4096];
        let mut read = 0;
        match unsafe { ReadFile(self.handle, Some(&mut chunk), Some(&mut read), None) } {
            Ok(()) if read > 0 => self.input.extend_from_slice(&chunk[..read as usize]),
            Ok(()) => return Ok(None),
            Err(_) => return Ok(None),
        }
        if self.input.len() > MAX_FRAME_BYTES {
            self.reset()?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "named-pipe request exceeds frame limit",
            ));
        }
        if self.input.len() < crate::FRAME_HEADER_BYTES {
            return Ok(None);
        }
        let payload =
            u32::from_le_bytes(self.input[6..10].try_into().expect("fixed protocol header"))
                as usize;
        let total = crate::FRAME_HEADER_BYTES.saturating_add(payload);
        if total > MAX_FRAME_BYTES {
            self.reset()?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "named-pipe request declares oversized frame",
            ));
        }
        if self.input.len() < total {
            return Ok(None);
        }
        if self.input.len() != total {
            self.reset()?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "multiple outstanding pipe requests are forbidden",
            ));
        }
        let envelope = match crate::decode::<ClientEnvelope>(&self.input) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.reset()?;
                return Err(io::Error::other(error));
            }
        };
        self.input.clear();
        if envelope.token != self.capability {
            self.reset()?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid session capability",
            ));
        }
        Ok(Some(envelope))
    }

    /// Writes one correlated bounded response. A partial/non-ready write disconnects the client;
    /// responses are never queued without a bound.
    pub fn respond(&mut self, response: &ServerEnvelope) -> io::Result<()> {
        if !self.connected {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "pipe is not connected",
            ));
        }
        let frame = crate::encode(response).map_err(io::Error::other)?;
        let mut written = 0;
        let result = unsafe { WriteFile(self.handle, Some(&frame), Some(&mut written), None) };
        if result.is_err() || written as usize != frame.len() {
            self.reset()?;
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "bounded named-pipe response was not accepted",
            ));
        }
        Ok(())
    }

    pub fn reset(&mut self) -> io::Result<()> {
        self.input.clear();
        self.connected = false;
        unsafe { DisconnectNamedPipe(self.handle) }
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

impl Drop for WindowsPipeServer {
    fn drop(&mut self) {
        if self.connected {
            let _ = unsafe { DisconnectNamedPipe(self.handle) };
        }
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
