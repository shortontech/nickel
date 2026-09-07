//! Typed boundary between reusable shell state and the session that owns it.
//!
//! The production adapter still uses the platform transport today. A compositor
//! hosted shell can instead provide an implementation that applies commands
//! directly, without teaching shell state about Smithay or socket details.

use crate::platform::{self, SessionRequestError, ShellCommand};

#[cfg(target_os = "linux")]
use crate::session::{NickelSession, SessionAuthority, SessionAuthorityRequest};

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
}

#[cfg(target_os = "linux")]
impl SessionHost for InProcessSessionHost {
    fn dispatch(&self, command: ShellCommand) -> Result<(), SessionRequestError> {
        self.sender
            .send(platform::shell_command_payload(command).into())
            .map_err(|_| SessionRequestError::Send)
    }
}

/// Install the in-process authority bridge into the compositor event loop.
///
/// The returned host is intended to be injected into `LiveShell` once the
/// shell is compositor-owned. Until then `PlatformSessionHost` remains the
/// default for the supervised external shell role.
#[cfg(target_os = "linux")]
pub(crate) fn install_in_process_session_host(
    loop_handle: &smithay::reexports::calloop::LoopHandle<'static, NickelSession>,
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
    Ok(InProcessSessionHost { sender })
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use nickel_session_protocol::{Command, ShellRole};
    use smithay::reexports::calloop::channel::channel;

    use super::{InProcessSessionHost, SessionHost};
    use crate::{platform::ShellCommand, session::SessionAuthorityRequest};

    #[test]
    fn in_process_host_emits_typed_authority_commands_without_transport_identity() {
        let (sender, receiver) = channel();
        let host = InProcessSessionHost { sender };

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
        let host = InProcessSessionHost { sender };

        assert!(host.dispatch(ShellCommand::Show).is_err());
    }
}
