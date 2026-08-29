use crate::{
    NickelSession,
    grabs::resize_grab,
    state::{ClientState, SurfaceBufferCommit},
};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface,
        },
        seat::WaylandFocus,
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

fn commit_is_render_visible(synchronized_subsurface: bool) -> bool {
    !synchronized_subsurface
}

impl CompositorHandler for NickelSession {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(client) = client.get_data::<ClientState>() {
            &client.compositor_state
        } else {
            &client
                .get_data::<smithay::xwayland::XWaylandClientData>()
                .expect("all compositor clients have compositor state")
                .compositor_state
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        let render_visible = commit_is_render_visible(is_sync_subsurface(surface));
        if render_visible {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(&root))
            {
                window.on_commit();
            }
        };

        xdg_shell::handle_commit(&mut self.popups, &self.space, surface);
        resize_grab::handle_commit(&mut self.space, surface);
        if let Some(sender) = &self.buffer_commit_tx {
            let _ = sender.send(SurfaceBufferCommit {
                surface: surface.clone(),
                render_visible,
            });
        }
        if render_visible {
            self.request_output_redraw();
        }
    }
}

impl BufferHandler for NickelSession {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for NickelSession {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

#[cfg(test)]
mod tests {
    use super::commit_is_render_visible;

    #[test]
    fn commit_visibility_matches_subsurface_synchronization() {
        for (synchronized_subsurface, expected_visible) in [(true, false), (false, true)] {
            assert_eq!(
                commit_is_render_visible(synchronized_subsurface),
                expected_visible,
                "synchronized={synchronized_subsurface}"
            );
        }
    }
}
