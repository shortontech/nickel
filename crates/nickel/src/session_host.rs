//! Typed boundary between reusable shell state and the session that owns it.
//!
//! The production adapter still uses the platform transport today. A compositor
//! hosted shell can instead provide an implementation that applies commands
//! directly, without teaching shell state about Smithay or socket details.

use crate::platform::{self, SecureStorageState, SessionRequestError, ShellCommand};

/// Copy effects and admission failures may coexist in a batch. A rejected cut
/// never revokes a successful copy/cut that already supplied replacement text.
pub(crate) fn record_clipboard_outcome(
    slot: &mut Option<Result<String, String>>,
    outcome: &mut nickel_ui::HostEventOutcome,
) {
    for failure in outcome
        .failures
        .iter()
        .filter(|failure| failure.stage == nickel_ui::HostFailureStage::Clipboard)
    {
        tracing::warn!(detail = failure.detail, "host clipboard operation rejected");
        if !matches!(slot, Some(Ok(_))) {
            *slot = Some(Err(failure.detail.clone()));
        }
    }
    if let Some(text) = outcome.clipboard_text.take() {
        *slot = Some(Ok(text));
    }
}

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
    fn copy_image(&self, image: image::RgbaImage) -> Result<(), String> {
        platform::copy_image_to_clipboard(image)
    }
    fn copy_image_path(&self, image: &image::RgbaImage) -> Result<std::path::PathBuf, String> {
        platform::copy_temp_image_path(image)
    }
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError>;
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        Err(SessionRequestError::Send)
    }
    fn configure_keyboard(
        &self,
        enabled: bool,
        visible: bool,
        generation: u64,
        environment_override: bool,
        dock_top: bool,
        height: u32,
    ) -> Result<(), SessionRequestError> {
        let _ = (
            enabled,
            visible,
            generation,
            environment_override,
            dock_top,
            height,
        );
        Err(SessionRequestError::Send)
    }
    fn keyboard_input(
        &self,
        epoch: u64,
        input: nickel_session_protocol::OnScreenKeyboardInput,
    ) -> Result<(), SessionRequestError> {
        let _ = (epoch, input);
        Err(SessionRequestError::Send)
    }
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
    fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
        #[cfg(target_os = "linux")]
        return DesktopCapturePoll::Ready(match output {
            Some(_) => platform::capture_output(output),
            None => platform::capture_desktop(),
        });
        #[cfg(not(target_os = "linux"))]
        {
            let _ = output;
            DesktopCapturePoll::Ready(platform::capture_desktop())
        }
    }
}

#[derive(Default)]
pub struct PlatformSessionHost;

impl SessionHost for PlatformSessionHost {
    #[cfg(target_os = "linux")]
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        platform::on_screen_keyboard_snapshot()
    }
    #[cfg(target_os = "linux")]
    fn configure_keyboard(
        &self,
        enabled: bool,
        visible: bool,
        generation: u64,
        environment_override: bool,
        dock_top: bool,
        height: u32,
    ) -> Result<(), SessionRequestError> {
        platform::configure_on_screen_keyboard(
            enabled,
            visible,
            generation,
            environment_override,
            dock_top,
            height,
        )
    }
    #[cfg(target_os = "linux")]
    fn keyboard_input(
        &self,
        epoch: u64,
        input: nickel_session_protocol::OnScreenKeyboardInput,
    ) -> Result<(), SessionRequestError> {
        platform::deliver_on_screen_keyboard_input(epoch, input)
    }
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
    keyboard: Arc<RwLock<Option<nickel_session_protocol::OnScreenKeyboardSnapshot>>>,
}

#[cfg(target_os = "linux")]
impl SessionHost for InProcessSessionHost {
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        self.keyboard
            .read()
            .map_err(|_| SessionRequestError::Receive)?
            .clone()
            .ok_or(SessionRequestError::Receive)
    }
    fn configure_keyboard(
        &self,
        enabled: bool,
        visible: bool,
        generation: u64,
        environment_override: bool,
        dock_top: bool,
        height: u32,
    ) -> Result<(), SessionRequestError> {
        self.sender
            .send(
                nickel_session_protocol::Command::ConfigureOnScreenKeyboard {
                    enabled,
                    visible,
                    generation,
                    environment_override,
                    dock_top,
                    height,
                }
                .into(),
            )
            .map_err(|_| SessionRequestError::Send)
    }
    fn keyboard_input(
        &self,
        epoch: u64,
        input: nickel_session_protocol::OnScreenKeyboardInput,
    ) -> Result<(), SessionRequestError> {
        self.sender
            .send(nickel_session_protocol::Command::OnScreenKeyboardInput { epoch, input }.into())
            .map_err(|_| SessionRequestError::Send)
    }
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

    fn copy_image(&self, image: image::RgbaImage) -> Result<(), String> {
        use image::ImageEncoder;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        )
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| error.to_string())?;
        self.sender
            .send(SessionAuthorityRequest::PublishClipboardImage(Arc::new(
                png,
            )))
            .map_err(|_| "native clipboard authority is unavailable".into())
    }

    fn copy_image_path(&self, image: &image::RgbaImage) -> Result<std::path::PathBuf, String> {
        let path = platform::save_temp_image(image)?;
        if self
            .sender
            .send(SessionAuthorityRequest::PublishClipboardText(
                path.to_string_lossy().into_owned(),
            ))
            .is_err()
        {
            let _ = std::fs::remove_file(&path);
            return Err("native clipboard authority is unavailable".into());
        }
        Ok(path)
    }

    fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
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
                    output: output.map(str::to_owned),
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
    keyboard: Arc<RwLock<Option<nickel_session_protocol::OnScreenKeyboardSnapshot>>>,
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
            session.publish_internal_keyboard_snapshot();
        }
    })?;
    Ok(InProcessSessionHost {
        sender,
        secure_storage_state,
        secure_storage_retry,
        projection_outputs,
        capture,
        keyboard,
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
            keyboard: Arc::new(RwLock::new(None)),
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
    fn native_keyboard_host_reads_current_snapshot_and_enqueues_epoch_checked_input() {
        use nickel_session_protocol::{OnScreenKeyboardInput, OnScreenKeyboardSnapshot, WindowId};
        let (sender, receiver) = channel();
        let host = host(sender);
        assert!(host.keyboard_snapshot().is_err());
        let snapshot = OnScreenKeyboardSnapshot {
            height: 320,
            generation: 1,
            epoch: 19,
            recipient: Some(WindowId(7)),
            enabled: true,
            visible: true,
            ..Default::default()
        };
        *host.keyboard.write().unwrap() = Some(snapshot.clone());
        assert_eq!(host.keyboard_snapshot().unwrap(), snapshot);
        host.configure_keyboard(true, true, 1, false, false, 320)
            .unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            SessionAuthorityRequest::Command(Command::ConfigureOnScreenKeyboard {
                enabled: true,
                visible: true,
                generation: 1,
                environment_override: false,
                dock_top: false,
                height: 320,
            })
        );
        let input = OnScreenKeyboardInput::Text { text: "a".into() };
        host.keyboard_input(19, input.clone()).unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            SessionAuthorityRequest::Command(Command::OnScreenKeyboardInput { epoch: 19, input })
        );
        // Enqueueing does not manufacture an acknowledgement or mutate the recipient.
        assert_eq!(host.keyboard_snapshot().unwrap(), snapshot);
        let next = OnScreenKeyboardSnapshot {
            epoch: 20,
            recipient: None,
            ..snapshot
        };
        *host.keyboard.write().unwrap() = Some(next.clone());
        assert_eq!(host.keyboard_snapshot().unwrap(), next);
        drop(receiver);
        assert!(
            host.configure_keyboard(false, false, 1, false, false, 320)
                .is_err()
        );
        assert!(
            host.keyboard_input(20, OnScreenKeyboardInput::Text { text: "b".into() })
                .is_err()
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
    fn native_keyboard_configuration_is_applied_by_the_session_before_snapshot_acknowledgement() {
        use smithay::reexports::{calloop::EventLoop, wayland_server::Display};
        let mut event_loop = EventLoop::try_new().unwrap();
        let mut session =
            crate::session::NickelSession::new(&mut event_loop, Display::new().unwrap(), true);
        let host = super::install_in_process_session_host(
            &event_loop.handle(),
            session.secure_storage_state_handle(),
            session.secure_storage_retry_handle(),
            Arc::clone(&session.internal_projection_outputs),
            Arc::clone(&session.internal_capture),
            Arc::clone(&session.internal_keyboard_snapshot),
        )
        .unwrap();
        session.publish_internal_keyboard_snapshot();
        assert!(!host.keyboard_snapshot().unwrap().enabled);
        host.configure_keyboard(true, false, 37, false, true, 350)
            .unwrap();
        assert!(
            !host.keyboard_snapshot().unwrap().enabled,
            "enqueue is not an acknowledgement"
        );
        event_loop
            .dispatch(std::time::Duration::ZERO, &mut session)
            .unwrap();
        let acknowledged = host.keyboard_snapshot().unwrap();
        assert!(acknowledged.enabled);
        assert!(!acknowledged.visible);
        assert_eq!(acknowledged.generation, 37);
        assert_eq!(acknowledged.height, 350);
        assert!(acknowledged.dock_top);
        assert_eq!(acknowledged, session.on_screen_keyboard_snapshot());
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
    fn in_process_screenshot_copy_publishes_image_and_path_through_native_authority() {
        let (sender, receiver) = channel();
        let host = host(sender);
        let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([23, 45, 67, 255]));
        host.copy_image(image.clone()).unwrap();
        let SessionAuthorityRequest::PublishClipboardImage(png) = receiver.try_recv().unwrap()
        else {
            panic!("image publication")
        };
        assert_eq!(image::load_from_memory(&png).unwrap().into_rgba8(), image);
        let path = host.copy_image_path(&image).unwrap();
        assert_eq!(
            receiver.try_recv().unwrap(),
            SessionAuthorityRequest::PublishClipboardText(path.to_string_lossy().into_owned())
        );
        assert_eq!(image::open(&path).unwrap().into_rgba8(), image);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn in_process_capture_enqueues_typed_request_without_a_reply_socket() {
        let (sender, receiver) = channel();
        let host = host(sender);

        assert!(matches!(
            host.capture_desktop(Some("secondary")),
            DesktopCapturePoll::Pending
        ));
        let request = receiver.try_recv().expect("typed capture request");
        assert!(matches!(
            request,
            SessionAuthorityRequest::Command(Command::CaptureOutput { output: Some(ref output), .. }) if output == "secondary"
        ));
        assert!(matches!(
            host.capture_desktop(Some("secondary")),
            DesktopCapturePoll::Pending
        ));
    }
}
