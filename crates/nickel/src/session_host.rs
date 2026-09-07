//! Typed boundary between reusable shell state and the session that owns it.
//!
//! The production adapter still uses the platform transport today. A compositor
//! hosted shell can instead provide an implementation that applies commands
//! directly, without teaching shell state about Smithay or socket details.

use crate::platform::{self, SessionRequestError, ShellCommand};

pub trait SessionHost: Send + Sync {
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError>;
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
}
