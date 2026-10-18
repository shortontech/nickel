//! Typed boundary between reusable shell state and the session that owns it.
//!
//! Standalone shells use the platform transport. The compositor runtime implements
//! this interface with queued native commands in `session::in_process_host`,
//! without teaching reusable shell state about Smithay or socket details.

use crate::platform::{self, SecureStorageState, SessionRequestError, ShellCommand};

/// Copy effects and admission failures may coexist in a batch. A rejected cut
/// never revokes a successful copy/cut that already supplied replacement text.
#[cfg(target_os = "linux")]
pub fn record_clipboard_outcome(
    slot: &mut Option<Result<String, String>>,
    outcome: &mut twinkle_ui::HostEventOutcome,
) {
    for failure in outcome
        .failures
        .iter()
        .filter(|failure| failure.stage == twinkle_ui::HostFailureStage::Clipboard)
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

pub enum DesktopCapturePoll {
    #[cfg(target_os = "linux")]
    Pending,
    Ready(Result<platform::DesktopCapture, String>),
}

// Windows remote controls use WindowsDesktopAuthority; the reusable shell host
// keeps the compositor methods for Linux and standalone shell fixtures.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub trait SessionHost: Send + Sync {
    fn feature_preference_writes_allowed(&self) -> bool {
        true
    }
    fn optional_features_committed(
        &self,
        _codex_generation: u64,
        _keyboard_generation: u64,
    ) -> Result<(), String> {
        // The native Windows shell already observes preferences in its system subscription.
        #[cfg(target_os = "linux")]
        {
            platform::refresh_optional_features()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
    }
    fn stages_effects(&self) -> bool {
        false
    }
    fn copy_image(&self, image: image::RgbaImage) -> Result<(), String> {
        platform::copy_image_to_clipboard(image)
    }
    fn copy_image_path(&self, image: &image::RgbaImage) -> Result<std::path::PathBuf, String> {
        platform::copy_temp_image_path(image)
    }
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError>;
    fn remote_pending_leases(&self) -> Vec<nickel_session_protocol::RemotePendingLease> {
        Vec::new()
    }
    fn decide_remote_lease(
        &self,
        pending: &nickel_session_protocol::RemotePendingLease,
        allow: bool,
    ) -> Result<(), SessionRequestError> {
        let _ = (pending, allow);
        Err(SessionRequestError::Send)
    }
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

// Unit tests replace this native adapter with TestSessionHost.
#[cfg_attr(test, allow(dead_code))]
#[derive(Default)]
pub struct PlatformSessionHost;

impl SessionHost for PlatformSessionHost {
    #[cfg(target_os = "linux")]
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        platform::on_screen_keyboard_snapshot()
    }
    #[cfg(target_os = "windows")]
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        platform::on_screen_keyboard_snapshot().map_err(|error| {
            tracing::warn!(%error, "Windows keyboard recipient observation failed");
            SessionRequestError::Receive
        })
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
    #[cfg(target_os = "windows")]
    fn keyboard_input(
        &self,
        epoch: u64,
        input: nickel_session_protocol::OnScreenKeyboardInput,
    ) -> Result<(), SessionRequestError> {
        platform::deliver_on_screen_keyboard_input(epoch, input).map_err(|error| {
            tracing::warn!(%error, "Windows keyboard input rejected");
            SessionRequestError::Send
        })
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

/// A bounded effect collector installed only during one remote shell reduction.
/// The owner executes its commands with fresh authority before acknowledging.
#[cfg(any(test, target_os = "linux"))]
pub struct StagedSessionHost {
    // Windows fixtures construct the collector; only the Linux owner reads
    // through to the original session host during guarded effect delivery.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    original: std::sync::Arc<dyn SessionHost>,
    commands: std::sync::Mutex<Vec<ShellCommand>>,
}
#[cfg(any(test, target_os = "linux"))]
impl StagedSessionHost {
    pub fn new(original: std::sync::Arc<dyn SessionHost>) -> Self {
        Self {
            original,
            commands: Default::default(),
        }
    }
    pub(crate) fn take_commands(&self) -> Vec<ShellCommand> {
        std::mem::take(&mut *self.commands.lock().unwrap())
    }
}
#[cfg(any(test, target_os = "linux"))]
impl SessionHost for StagedSessionHost {
    fn feature_preference_writes_allowed(&self) -> bool {
        false
    }
    fn optional_features_committed(&self, _: u64, _: u64) -> Result<(), String> {
        Err("staged feature writes are unavailable".into())
    }
    fn stages_effects(&self) -> bool {
        true
    }
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError> {
        let mut commands = self
            .commands
            .lock()
            .map_err(|_| SessionRequestError::Send)?;
        if commands.len() >= 8 {
            return Err(SessionRequestError::Send);
        }
        commands.push(command);
        Ok(())
    }
    fn copy_image(&self, _: image::RgbaImage) -> Result<(), String> {
        Err("remote image effect unavailable".into())
    }
    fn copy_image_path(&self, _: &image::RgbaImage) -> Result<std::path::PathBuf, String> {
        Err("remote image effect unavailable".into())
    }
    fn consumer_control(&self, _: nickel_session_protocol::ConsumerControl) -> bool {
        false
    }
    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        Err(SessionRequestError::Send)
    }
    fn capture_desktop(&self, _: Option<&str>) -> DesktopCapturePoll {
        DesktopCapturePoll::Ready(Err("remote capture effect unavailable".into()))
    }
    fn secure_storage_state(&self) -> Result<SecureStorageState, SessionRequestError> {
        self.original.secure_storage_state()
    }
    fn keyboard_snapshot(
        &self,
    ) -> Result<nickel_session_protocol::OnScreenKeyboardSnapshot, SessionRequestError> {
        self.original.keyboard_snapshot()
    }
    fn projection_outputs(&self) -> Result<Vec<nickel_session_protocol::OutputSnapshot>, String> {
        self.original.projection_outputs()
    }
}
