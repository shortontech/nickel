//! Ordinary-client controller admission while the shell keyboard retains text focus.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ControllerFence {
    pub blocked: bool,
    pub barrier_unix_ms: u64,
}

impl ControllerFence {
    pub fn admits(&self, produced_at: SystemTime) -> bool {
        !self.blocked
            && (self.barrier_unix_ms == 0
                || produced_at
                    .duration_since(UNIX_EPOCH)
                    .is_ok_and(|age| age.as_millis() > u128::from(self.barrier_unix_ms)))
    }
}

pub(crate) fn controller_fence() -> ControllerFence {
    #[cfg(target_os = "linux")]
    {
        use nickel_session_protocol::{Query, Request, ServerMessage};
        match nickel_session_protocol::client::request_from_environment(
            Request::Query(Query::OnScreenKeyboard),
            std::time::Duration::from_millis(25),
        ) {
            Ok(None) => ControllerFence::default(),
            Ok(Some(ServerMessage::OnScreenKeyboard(snapshot))) => ControllerFence {
                blocked: snapshot.visible,
                barrier_unix_ms: snapshot.controller_barrier_unix_ms,
            },
            // A configured session with unknown ownership must not dispatch to two consumers.
            _ => ControllerFence {
                blocked: true,
                ..Default::default()
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    ControllerFence::default()
}

pub(crate) fn request_text_entry() {
    #[cfg(target_os = "linux")]
    {
        use nickel_session_protocol::{Command, Request};
        let _ = nickel_session_protocol::client::request_from_environment(
            Request::Command(Command::RequestOnScreenKeyboard),
            std::time::Duration::from_millis(25),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn closing_button_and_queued_events_do_not_escape_to_the_recipient() {
        let fence = ControllerFence {
            blocked: false,
            barrier_unix_ms: 100,
        };
        for millis in [80, 99, 100] {
            assert!(!fence.admits(UNIX_EPOCH + Duration::from_millis(millis)));
        }
        assert!(fence.admits(UNIX_EPOCH + Duration::from_millis(101)));
        assert!(
            !ControllerFence {
                blocked: true,
                ..fence
            }
            .admits(SystemTime::now())
        );
    }
}
