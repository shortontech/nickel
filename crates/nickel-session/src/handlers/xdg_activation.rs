use smithay::{
    delegate_xdg_activation,
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
};

use crate::NickelSession;

impl XdgActivationHandler for NickelSession {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let target_client = self
            .display_handle
            .get_client(surface.id())
            .ok()
            .map(|client| client.id());
        let same_client = target_client.is_some() && token_data.client_id == target_client
            || token_data
                .surface
                .as_ref()
                .is_some_and(|requester| requester.id().same_client_as(&surface.id()));
        if token_data.timestamp.elapsed().as_secs() < 10
            && same_client
            && let Some(id) = self.surface_windows.get(&surface.id()).copied()
        {
            self.activate_window(id);
        }
        self.activation_state.remove_token(&token);
    }
}

delegate_xdg_activation!(NickelSession);
