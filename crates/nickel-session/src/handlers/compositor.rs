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
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, get_parent, is_sync_subsurface, with_states,
        },
        seat::WaylandFocus,
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

fn commit_is_render_visible(synchronized_subsurface: bool) -> bool {
    !synchronized_subsurface
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BufferTransition {
    Attached,
    Removed,
    Unchanged,
}

#[cfg(test)]
fn mapped_after_transition(mapped: bool, transition: BufferTransition) -> bool {
    match transition {
        BufferTransition::Attached => true,
        BufferTransition::Removed => false,
        BufferTransition::Unchanged => mapped,
    }
}

fn buffer_transition(surface: &WlSurface) -> BufferTransition {
    with_states(surface, |states| {
        let mut attributes = states.cached_state.get::<SurfaceAttributes>();
        match attributes.current().buffer.as_ref() {
            Some(BufferAssignment::NewBuffer(_)) => BufferTransition::Attached,
            Some(BufferAssignment::Removed) => BufferTransition::Removed,
            None => BufferTransition::Unchanged,
        }
    })
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
        let transition = buffer_transition(surface);
        on_commit_buffer_handler::<Self>(surface);
        let render_visible = commit_is_render_visible(is_sync_subsurface(surface));
        if render_visible {
            self.invalidate_preview_for_surface(surface);
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if root == *surface {
                match transition {
                    BufferTransition::Attached => {
                        self.map_xdg_toplevel(&root);
                    }
                    BufferTransition::Removed => {
                        self.unmap_xdg_toplevel(&root);
                    }
                    BufferTransition::Unchanged => {}
                }
            }
            let committed_window = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(&root))
                .cloned();
            if let Some(window) = committed_window {
                window.on_commit();
                self.relayout_committed_shell_window(&window);
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
    use super::{BufferTransition, commit_is_render_visible, mapped_after_transition};

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

    #[test]
    fn xdg_buffer_lifecycle_maps_unmaps_and_remaps() {
        let mapped = mapped_after_transition(false, BufferTransition::Unchanged);
        assert!(!mapped, "the initial bufferless configure stays unmapped");
        let mapped = mapped_after_transition(mapped, BufferTransition::Attached);
        assert!(mapped, "the first real buffer maps the toplevel");
        let mapped = mapped_after_transition(mapped, BufferTransition::Unchanged);
        assert!(mapped, "metadata-only commits preserve mapping");
        let mapped = mapped_after_transition(mapped, BufferTransition::Removed);
        assert!(!mapped, "an explicit null buffer unmaps the toplevel");
        let mapped = mapped_after_transition(mapped, BufferTransition::Attached);
        assert!(mapped, "a later real buffer remaps the same protocol role");
    }
}
