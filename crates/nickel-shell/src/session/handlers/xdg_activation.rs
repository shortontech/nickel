use std::time::Duration;

use smithay::{
    input::Seat,
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    wayland::seat::WaylandFocus,
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};

use crate::NickelSession;

const ACTIVATION_TOKEN_MAX_AGE: Duration = Duration::from_secs(10);
const MAX_PENDING_ACTIVATION_TOKENS: usize = 256;

fn activation_allowed(age: Duration, requester_has_focus: bool, serial_is_for_seat: bool) -> bool {
    age < ACTIVATION_TOKEN_MAX_AGE && requester_has_focus && serial_is_for_seat
}

fn pending_token_allowed(current: usize) -> bool {
    current < MAX_PENDING_ACTIVATION_TOKENS
}

impl XdgActivationHandler for NickelSession {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData) -> bool {
        self.activation_state
            .retain_tokens(|_, data| data.timestamp.elapsed() < ACTIVATION_TOKEN_MAX_AGE);
        pending_token_allowed(self.activation_state.tokens().count())
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let focused = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| focus.wl_surface().map(std::borrow::Cow::into_owned));
        let requester_has_focus = focused.as_ref().is_some_and(|focused| {
            token_data
                .surface
                .as_ref()
                .is_some_and(|requester| requester == focused)
                || token_data.client_id.as_ref().is_some_and(|requester| {
                    self.display_handle
                        .get_client(focused.id())
                        .is_ok_and(|client| client.id() == *requester)
                })
        });
        let serial_is_for_seat = token_data.serial.as_ref().is_some_and(|(serial, seat)| {
            Seat::<Self>::from_resource(seat).as_ref() == Some(&self.seat)
                && self
                    .seat
                    .get_keyboard()
                    .and_then(|keyboard| keyboard.last_enter())
                    .is_some_and(|focus_serial| serial.is_no_older_than(&focus_serial))
        });
        let age = token_data.timestamp.elapsed();
        let target = self.surface_windows.get(&surface.id()).copied();
        let allowed = activation_allowed(age, requester_has_focus, serial_is_for_seat);
        tracing::debug!(
            ?age,
            requester_has_focus,
            serial_is_for_seat,
            ?target,
            allowed,
            "processed XDG activation request"
        );
        if allowed && let Some(id) = target {
            self.activate_window(id);
        }
        self.activation_state.remove_token(&token);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        ACTIVATION_TOKEN_MAX_AGE, MAX_PENDING_ACTIVATION_TOKENS, activation_allowed,
        pending_token_allowed,
    };

    #[test]
    fn activation_requires_a_recent_focused_request_with_a_seat_serial() {
        assert!(activation_allowed(Duration::ZERO, true, true));
        assert!(!activation_allowed(Duration::ZERO, false, true));
        assert!(!activation_allowed(Duration::ZERO, true, false));
        assert!(!activation_allowed(ACTIVATION_TOKEN_MAX_AGE, true, true));
    }

    #[test]
    fn activation_token_pool_has_a_hard_limit() {
        assert!(pending_token_allowed(MAX_PENDING_ACTIVATION_TOKENS - 1));
        assert!(!pending_token_allowed(MAX_PENDING_ACTIVATION_TOKENS));
    }
}
