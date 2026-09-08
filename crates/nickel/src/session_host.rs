//! Typed boundary between reusable shell state and the session that owns it.
//!
//! The production adapter still uses the platform transport today. A compositor
//! hosted shell can instead provide an implementation that applies commands
//! directly, without teaching shell state about Smithay or socket details.

use crate::platform::{self, SecureStorageState, SessionRequestError, ShellCommand};

pub(crate) enum DesktopCapturePoll {
    Pending,
    Ready(Result<platform::DesktopCapture, String>),
}

#[cfg(target_os = "linux")]
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

#[cfg(target_os = "linux")]
use crate::session::{NickelSession, SessionAuthority, SessionAuthorityRequest};

pub trait SessionHost: Send + Sync {
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError>;
    /// Enqueue a consumer action without blocking the compositor on backend I/O.
    /// Success means accepted for delivery, not a confirmed mixer/player change.
    fn consumer_control(&self, control: nickel_session_protocol::ConsumerControl) -> bool {
        platform::handle_consumer_control(control)
    }
    fn secure_storage_state(&self) -> Result<SecureStorageState, SessionRequestError> {
        Ok(SecureStorageState::Ready)
    }
    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        Ok(())
    }
    fn projection_outputs(&self) -> Result<Vec<nickel_session_protocol::OutputSnapshot>, String> {
        #[cfg(target_os = "linux")]
        return platform::projection_outputs();
        #[cfg(not(target_os = "linux"))]
        Err("display projection is unavailable on this platform".into())
    }
    fn capture_desktop(&self) -> DesktopCapturePoll {
        DesktopCapturePoll::Ready(platform::capture_desktop())
    }
}

#[derive(Default)]
pub struct PlatformSessionHost;

impl SessionHost for PlatformSessionHost {
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError> {
        #[cfg(target_os = "linux")]
        {
            platform::send_shell_command(command)
        }
        #[cfg(not(target_os = "linux"))]
        {
            if platform::send_shell_command(command) {
                Ok(())
            } else {
                Err(SessionRequestError::Send)
            }
        }
    }

    fn secure_storage_state(&self) -> Result<SecureStorageState, SessionRequestError> {
        #[cfg(target_os = "linux")]
        return platform::secure_storage_state();
        #[cfg(not(target_os = "linux"))]
        Ok(SecureStorageState::Ready)
    }

    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        #[cfg(target_os = "linux")]
        return platform::request_secure_storage_retry();
        #[cfg(not(target_os = "linux"))]
        Ok(())
    }
}

/// A shell-to-session command path for UI hosted by the compositor process.
///
/// Dispatch only enqueues typed work. The calloop source applies that work on
/// the compositor thread, where `NickelSession` is already exclusively owned.
/// There is deliberately no token, PID, request id, encoding, or socket in
/// this path.
#[cfg(target_os = "linux")]
#[derive(Clone)]
pub(crate) struct InProcessSessionHost {
    sender: smithay::reexports::calloop::channel::Sender<SessionAuthorityRequest>,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    projection_outputs: Arc<RwLock<Vec<nickel_session_protocol::OutputSnapshot>>>,
    capture: Arc<Mutex<crate::session::InternalCaptureState>>,
}

#[cfg(target_os = "linux")]
impl SessionHost for InProcessSessionHost {
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError> {
        self.sender
            .send(platform::shell_command_payload(command).into())
            .map_err(|_| SessionRequestError::Send)
    }

    fn secure_storage_state(&self) -> Result<SecureStorageState, SessionRequestError> {
        use crate::session::login_services::SecureStorageState as SessionState;

        Ok(
            match SessionState::from_u8(self.secure_storage_state.load(Ordering::Acquire)) {
                SessionState::Starting => SecureStorageState::Starting,
                SessionState::Locked => SecureStorageState::Locked,
                SessionState::PromptRequired => SecureStorageState::PromptRequired,
                SessionState::Ready => SecureStorageState::Ready,
                SessionState::Unavailable => {
                    crate::session::login_services::secure_storage_unavailable_reason().map_or(
                        SecureStorageState::Unavailable,
                        SecureStorageState::UnavailableReason,
                    )
                }
            },
        )
    }

    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        self.secure_storage_retry.store(true, Ordering::Release);
        Ok(())
    }

    fn projection_outputs(&self) -> Result<Vec<nickel_session_protocol::OutputSnapshot>, String> {
        Ok(self.projection_outputs.read().unwrap().clone())
    }

    fn capture_desktop(&self) -> DesktopCapturePoll {
        use crate::session::InternalCaptureState;
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut capture = self.capture.lock().unwrap();
        match std::mem::replace(&mut *capture, InternalCaptureState::Idle) {
            InternalCaptureState::Idle => {
                let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path =
                    std::env::temp_dir().join(format!("nickel-internal-capture-{sequence}.png"));
                *capture = InternalCaptureState::Pending(path.clone());
                let request = nickel_session_protocol::Command::CaptureOutput {
                    path: path.to_string_lossy().into_owned(),
                    output: None,
                };
                if self.sender.send(request.into()).is_err() {
                    *capture = InternalCaptureState::Idle;
                    return DesktopCapturePoll::Ready(Err(
                        "internal capture authority is unavailable".into(),
                    ));
                }
                DesktopCapturePoll::Pending
            }
            InternalCaptureState::Pending(path) => {
                *capture = InternalCaptureState::Pending(path);
                DesktopCapturePoll::Pending
            }
            InternalCaptureState::Complete(path, result) => {
                let answer = match result {
                    nickel_session_protocol::CaptureResult::Saved { .. } => image::open(&path)
                        .map(|image| platform::DesktopCapture {
                            image: image.into_rgba8(),
                        })
                        .map_err(|error| {
                            format!("could not read captured desktop pixels: {error}")
                        }),
                    nickel_session_protocol::CaptureResult::Failed { message } => Err(message),
                };
                let _ = std::fs::remove_file(path);
                DesktopCapturePoll::Ready(answer)
            }
        }
    }
}

/// Install the in-process authority bridge into the compositor event loop.
///
/// Linux injects the returned host into its compositor-owned `LiveShell`.
/// `PlatformSessionHost` remains the standalone adapter for non-compositor
/// platforms and compatibility clients.
#[cfg(target_os = "linux")]
pub(crate) fn install_in_process_session_host(
    loop_handle: &smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
    secure_storage_state: Arc<AtomicU8>,
    secure_storage_retry: Arc<AtomicBool>,
    projection_outputs: Arc<RwLock<Vec<nickel_session_protocol::OutputSnapshot>>>,
    capture: Arc<Mutex<crate::session::InternalCaptureState>>,
) -> Result<
    InProcessSessionHost,
    smithay::reexports::calloop::InsertError<
        smithay::reexports::calloop::channel::Channel<SessionAuthorityRequest>,
    >,
> {
    let (sender, receiver) = smithay::reexports::calloop::channel::channel();
    loop_handle.insert_source(receiver, |event, _, session| {
        if let smithay::reexports::calloop::channel::Event::Msg(request) = event {
            let _ = session.invoke(request);
        }
    })?;
    Ok(InProcessSessionHost {
        sender,
        secure_storage_state,
        secure_storage_retry,
        projection_outputs,
        capture,
    })
}

pub(crate) fn default_session_host() -> std::sync::Arc<dyn SessionHost> {
    #[cfg(test)]
    {
        std::sync::Arc::new(TestSessionHost)
    }
    #[cfg(not(test))]
    {
        std::sync::Arc::new(PlatformSessionHost)
    }
}

#[cfg(test)]
struct TestSessionHost;

#[cfg(test)]
impl SessionHost for TestSessionHost {
    fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
        Ok(())
    }

    fn secure_storage_state(&self) -> Result<SecureStorageState, SessionRequestError> {
        Ok(SecureStorageState::Ready)
    }

    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    };

    use nickel_session_protocol::{Command, ShellRole};
    use smithay::reexports::calloop::channel::channel;

    use super::{DesktopCapturePoll, InProcessSessionHost, SessionHost};
    use crate::{platform::ShellCommand, session::SessionAuthorityRequest};

    fn host(
        sender: smithay::reexports::calloop::channel::Sender<SessionAuthorityRequest>,
    ) -> InProcessSessionHost {
        InProcessSessionHost {
            sender,
            secure_storage_state: Arc::new(AtomicU8::new(
                crate::session::login_services::SecureStorageState::Ready as u8,
            )),
            secure_storage_retry: Arc::new(AtomicBool::new(false)),
            projection_outputs: Arc::new(RwLock::new(Vec::new())),
            capture: Arc::new(Mutex::new(crate::session::InternalCaptureState::Idle)),
        }
    }

    #[test]
    fn in_process_host_emits_typed_authority_commands_without_transport_identity() {
        let (sender, receiver) = channel();
        let host = host(sender);

        host.dispatch(ShellCommand::SetShellRoleVisible {
            role: ShellRole::Notification,
            visible: true,
        })
        .expect("enqueue direct session command");

        assert_eq!(
            receiver.try_recv().expect("typed authority request"),
            SessionAuthorityRequest::Command(Command::SetShellRoleVisible {
                role: ShellRole::Notification,
                visible: true,
            })
        );
    }

    #[test]
    fn in_process_host_reports_closed_authority_channel() {
        let (sender, receiver) = channel();
        drop(receiver);
        let host = host(sender);

        assert!(host.dispatch(ShellCommand::Show).is_err());
    }

    #[test]
    fn in_process_secure_storage_access_uses_shared_authority_not_environment_transport() {
        let (sender, _receiver) = channel();
        let host = host(sender);

        // No control socket or capability token participates in either operation.
        assert_eq!(
            host.secure_storage_state().unwrap(),
            crate::platform::SecureStorageState::Ready
        );
        host.request_secure_storage_retry().unwrap();
        assert!(host.secure_storage_retry.load(Ordering::Acquire));
    }

    #[test]
    fn in_process_projection_query_reads_shared_compositor_state() {
        let (sender, _receiver) = channel();
        let host = host(sender);
        host.projection_outputs
            .write()
            .unwrap()
            .push(nickel_session_protocol::OutputSnapshot {
                name: "DP-1".into(),
                model: "Test".into(),
                geometry: nickel_session_protocol::Geometry {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                work_area: nickel_session_protocol::Geometry {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1040,
                },
                scale_120: 120,
                transform: nickel_session_protocol::OutputTransform::Normal,
                physical_width_mm: 0,
                physical_height_mm: 0,
                primary: true,
                enabled: true,
            });
        assert_eq!(host.projection_outputs().unwrap()[0].name, "DP-1");
    }

    #[test]
    fn in_process_capture_enqueues_typed_request_without_a_reply_socket() {
        let (sender, receiver) = channel();
        let host = host(sender);

        assert!(matches!(
            host.capture_desktop(),
            DesktopCapturePoll::Pending
        ));
        let request = receiver.try_recv().expect("typed capture request");
        assert!(matches!(
            request,
            SessionAuthorityRequest::Command(Command::CaptureOutput { output: None, .. })
        ));
        assert!(matches!(
            host.capture_desktop(),
            DesktopCapturePoll::Pending
        ));
    }
}
