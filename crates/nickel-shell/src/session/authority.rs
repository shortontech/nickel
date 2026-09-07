//! Typed, in-process access to compositor-owned session state.
//!
//! The datagram protocol remains the compatibility boundary for external
//! processes.  Code hosted by Nickel itself should use this interface instead:
//! possession of `&mut NickelSession` is the authority, so an internal caller
//! does not need to manufacture a process identity, capability token, socket
//! address, or request id.

use nickel_session_protocol::{Command, Query, ServerMessage};

use super::NickelSession;

/// A request made by a component hosted in the compositor process.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionAuthorityRequest {
    Query(Query),
    Command(Command),
}

impl From<Query> for SessionAuthorityRequest {
    fn from(value: Query) -> Self {
        Self::Query(value)
    }
}

impl From<Command> for SessionAuthorityRequest {
    fn from(value: Command) -> Self {
        Self::Command(value)
    }
}

/// Direct authority over compositor session policy.
///
/// This deliberately has no registration or subscription operations. Those
/// operations establish identity and delivery for out-of-process clients;
/// internal components are already identified by their typed state and will
/// receive events through the eventual in-process event host.
pub trait SessionAuthority {
    fn invoke(&mut self, request: SessionAuthorityRequest) -> ServerMessage;
}

impl SessionAuthority for NickelSession {
    fn invoke(&mut self, request: SessionAuthorityRequest) -> ServerMessage {
        self.handle_authority_request(request)
    }
}

#[cfg(test)]
mod tests {
    use nickel_session_protocol::{Command, Query, ServerMessage};

    use super::{SessionAuthority, SessionAuthorityRequest};

    struct RecordingAuthority {
        requests: Vec<SessionAuthorityRequest>,
    }

    impl SessionAuthority for RecordingAuthority {
        fn invoke(&mut self, request: SessionAuthorityRequest) -> ServerMessage {
            self.requests.push(request);
            ServerMessage::Ack
        }
    }

    #[test]
    fn direct_authority_preserves_typed_queries() {
        let mut authority = RecordingAuthority {
            requests: Vec::new(),
        };

        let response = authority.invoke(Query::Snapshot.into());

        assert_eq!(response, ServerMessage::Ack);
        assert_eq!(
            authority.requests,
            vec![SessionAuthorityRequest::Query(Query::Snapshot)]
        );
    }

    #[test]
    fn direct_authority_preserves_typed_commands_without_transport_identity() {
        let mut authority = RecordingAuthority {
            requests: Vec::new(),
        };

        authority.invoke(Command::ToggleLauncher.into());

        assert_eq!(
            authority.requests,
            vec![SessionAuthorityRequest::Command(Command::ToggleLauncher)]
        );
    }
}
