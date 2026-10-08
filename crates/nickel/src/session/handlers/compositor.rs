use crate::session::{
    NickelSession,
    grabs::resize_grab,
    state::{ClientState, SurfaceBufferCommit},
};
use smithay::{
    backend::{allocator::Buffer, renderer::utils::on_commit_buffer_handler},
    reexports::wayland_server::{
        Client, Resource,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            BufferAssignment, CompositorClientState, CompositorHandler, CompositorState,
            SurfaceAttributes, add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface,
            with_states,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MappingWork {
    Map,
    Unmap,
    None,
}

fn mapping_work(mapped: bool, transition: BufferTransition) -> MappingWork {
    match (mapped, transition) {
        (false, BufferTransition::Attached) => MappingWork::Map,
        (true, BufferTransition::Removed) => MappingWork::Unmap,
        _ => MappingWork::None,
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

    fn new_surface(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<Self, _>(surface, |state, dh, surface| {
            let dmabuf = with_states(surface, |states| {
                let mut attributes = states.cached_state.get::<SurfaceAttributes>();
                let Some(BufferAssignment::NewBuffer(buffer)) =
                    attributes.pending().buffer.as_ref()
                else {
                    return None;
                };
                smithay::wayland::dmabuf::get_dmabuf(buffer).ok().cloned()
            });
            let Some(dmabuf) = dmabuf else { return };
            let Ok((blocker, source)) =
                dmabuf.generate_blocker(smithay::reexports::calloop::Interest::READ)
            else {
                // The client's write fence has already signalled.
                return;
            };
            let Some(client) = surface.client() else {
                return;
            };
            let pending_client = client.clone();
            let registered = state
                .event_loop_handle
                .insert_source(source, move |_, _, state| {
                    let dh = state.display_handle.clone();
                    state
                        .client_compositor_state(&pending_client)
                        .blocker_cleared(state, &dh);
                    Ok(())
                });
            match registered {
                Ok(_) => {
                    // Keep the old committed surface tree visible until the
                    // producer finishes writing. Waiting after our own draw
                    // only synchronizes compositor output, not client input.
                    add_blocker(surface, blocker);
                }
                Err(error) => {
                    tracing::error!(?error, "could not monitor client DMA-BUF readiness");
                    client.kill(
                        dh,
                        smithay::reexports::wayland_server::backend::protocol::ProtocolError {
                            code: 3, // wl_display.error.implementation
                            object_id: 1,
                            object_interface: "wl_display".into(),
                            message: "could not monitor DMA-BUF readiness".into(),
                        },
                    );
                }
            }
        });
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
                let mapped = self.mapped_xdg_toplevels.contains(&root.id());
                match mapping_work(mapped, transition) {
                    MappingWork::Map => {
                        self.map_xdg_toplevel(&root);
                    }
                    MappingWork::Unmap => {
                        self.unmap_xdg_toplevel(&root);
                    }
                    MappingWork::None => {}
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
                self.fit_window_above_keyboard(&window);
            }
        };

        // A newly-created xdg_toplevel is deliberately not inserted into Space until it
        // attaches its first buffer.  Its initial (bufferless) commit must nevertheless
        // receive a configure; looking it up only in Space deadlocks undecorated child
        // windows such as Chromium's portal chooser and Electron confirmation dialogs.
        let toplevel = self.xdg_toplevel_window(surface);
        let initial_configure = xdg_shell::handle_commit(&mut self.popups, toplevel, surface);
        if let Some(toplevel) = initial_configure {
            self.send_tracked_xdg_initial_configure(&toplevel);
        }
        if let Some((window, acked, anchor)) = resize_grab::handle_commit(&mut self.space, surface)
            && self.observe_xdg_geometry_commit(&window, acked)
            && let Some(anchor) = anchor
        {
            self.space.map_element(window, anchor, false);
        }
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

impl smithay::wayland::dmabuf::DmabufHandler for NickelSession {
    fn dmabuf_state(&mut self) -> &mut smithay::wayland::dmabuf::DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &smithay::wayland::dmabuf::DmabufGlobal,
        dmabuf: smithay::backend::allocator::dmabuf::Dmabuf,
        notifier: smithay::wayland::dmabuf::ImportNotifier,
    ) {
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_mut()
            && !native.import_dmabuf(&dmabuf)
        {
            tracing::warn!(format = ?dmabuf.format(), "rejected client DMA-BUF that the primary renderer could not import");
            notifier.failed();
            return;
        }
        if notifier.successful::<Self>().is_err() {
            tracing::debug!("DMA-BUF client disconnected before wl_buffer creation");
        }
    }
}

impl ShmHandler for NickelSession {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

#[cfg(test)]
mod tests {
    use super::{BufferTransition, MappingWork, commit_is_render_visible, mapping_work};

    // A socket supplies deterministic poll readiness in place of a GPU write
    // fence. It is never imported into a renderer. The real DMA-BUF protocol,
    // pre-commit hook, transaction queue, and calloop wakeup are exercised.
    fn dma_buf_commit_readiness(ready_before_commit: bool, synchronized_child: bool) {
        use crate::session::{NickelSession, state::ClientState};
        use smithay::{
            backend::allocator::{Format, Fourcc, Modifier},
            reexports::{
                calloop::{EventLoop, channel},
                wayland_protocols::wp::linux_dmabuf::zv1::client::{
                    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
                },
                wayland_server::{Display, protocol::wl_surface::WlSurface},
            },
        };
        use smithay_client_toolkit::reexports::client::{
            Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
            protocol::{
                wl_buffer, wl_compositor, wl_registry, wl_subcompositor, wl_subsurface, wl_surface,
            },
        };
        use std::{
            io::Write,
            os::{fd::AsFd, unix::net::UnixStream},
            sync::{Arc, mpsc},
            time::{Duration, Instant},
        };

        #[derive(Default)]
        struct Client {
            compositor: Option<wl_compositor::WlCompositor>,
            subcompositor: Option<wl_subcompositor::WlSubcompositor>,
            dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
        }
        impl Dispatch<wl_registry::WlRegistry, ()> for Client {
            fn event(
                state: &mut Self,
                registry: &wl_registry::WlRegistry,
                event: wl_registry::Event,
                _: &(),
                _: &Connection,
                qh: &QueueHandle<Self>,
            ) {
                if let wl_registry::Event::Global {
                    name,
                    interface,
                    version,
                } = event
                {
                    match interface.as_str() {
                        "wl_compositor" => {
                            state.compositor = Some(registry.bind(name, version.min(6), qh, ()))
                        }
                        "zwp_linux_dmabuf_v1" => {
                            state.dmabuf = Some(registry.bind(name, version.min(3), qh, ()))
                        }
                        "wl_subcompositor" => {
                            state.subcompositor = Some(registry.bind(name, 1, qh, ()))
                        }
                        _ => {}
                    }
                }
            }
        }
        delegate_noop!(Client: ignore wl_compositor::WlCompositor);
        delegate_noop!(Client: ignore wl_surface::WlSurface);
        delegate_noop!(Client: ignore wl_buffer::WlBuffer);
        delegate_noop!(Client: ignore wl_subcompositor::WlSubcompositor);
        delegate_noop!(Client: ignore wl_subsurface::WlSubsurface);
        delegate_noop!(Client: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);
        delegate_noop!(Client: ignore zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1);

        let mut event_loop = EventLoop::try_new().unwrap();
        let display = Display::new().unwrap();
        let mut dh = display.handle();
        let (server, peer) = UnixStream::pair().unwrap();
        let server_client = dh
            .insert_client(server, Arc::new(ClientState::default()))
            .unwrap();
        let mut session = NickelSession::new(&mut event_loop, display, false);
        session.dmabuf_state.create_global::<NickelSession>(
            &dh,
            [Format {
                code: Fourcc::Abgr8888,
                modifier: Modifier::Linear,
            }],
        );
        let (commit_tx, commit_rx) = channel::channel();
        session.buffer_commit_tx = Some(commit_tx);
        let (buffer_fd, mut producer) = UnixStream::pair().unwrap();
        if ready_before_commit {
            producer.write_all(&[1]).unwrap();
        }
        let (surface_tx, surface_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let client_thread = std::thread::spawn(move || {
            let connection = Connection::from_socket(peer).unwrap();
            let mut queue = connection.new_event_queue::<Client>();
            let qh = queue.handle();
            connection.display().get_registry(&qh, ());
            let mut client = Client::default();
            queue.roundtrip(&mut client).unwrap();
            let surface = client.compositor.as_ref().unwrap().create_surface(&qh, ());
            let parent = synchronized_child
                .then(|| client.compositor.as_ref().unwrap().create_surface(&qh, ()));
            let _subsurface = parent.as_ref().map(|parent| {
                client
                    .subcompositor
                    .as_ref()
                    .unwrap()
                    .get_subsurface(&surface, parent, &qh, ())
            });
            let params = client.dmabuf.as_ref().unwrap().create_params(&qh, ());
            params.add(buffer_fd.as_fd(), 0, 0, 4, 0, 0);
            let buffer = params.create_immed(
                1,
                1,
                Fourcc::Abgr8888 as u32,
                zwp_linux_buffer_params_v1::Flags::empty(),
                &qh,
                (),
            );
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            if let Some(parent) = &parent {
                parent.commit();
            }
            queue.roundtrip(&mut client).unwrap();
            surface_tx
                .send(parent.as_ref().unwrap_or(&surface).id().protocol_id())
                .unwrap();
            let _ = done_rx.recv_timeout(Duration::from_secs(5));
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let surface_id = loop {
            if let Ok(id) = surface_rx.try_recv() {
                break id;
            }
            assert!(
                Instant::now() < deadline,
                "client commit was not dispatched"
            );
            event_loop
                .dispatch(Duration::from_millis(5), &mut session)
                .unwrap();
        };
        let surface = server_client
            .object_from_protocol_id::<WlSurface>(&dh, surface_id)
            .unwrap();
        if !ready_before_commit {
            assert!(
                commit_rx.try_recv().is_err(),
                "unready client buffer reached the renderer"
            );
            for _ in 0..3 {
                event_loop
                    .dispatch(Duration::from_millis(5), &mut session)
                    .unwrap();
                assert!(commit_rx.try_recv().is_err());
            }
            producer.write_all(&[1]).unwrap();
        }
        let mut child_commits = 0;
        let commit = loop {
            if let Ok(commit) = commit_rx.try_recv() {
                if commit.render_visible {
                    break commit;
                }
                child_commits += 1;
            }
            assert!(
                Instant::now() < deadline,
                "ready client commit did not resume"
            );
            event_loop
                .dispatch(Duration::from_millis(5), &mut session)
                .unwrap();
        };
        assert_eq!(commit.surface, surface);
        assert!(commit.render_visible);
        assert_eq!(child_commits, usize::from(synchronized_child));
        event_loop.dispatch(Duration::ZERO, &mut session).unwrap();
        assert!(
            commit_rx.try_recv().is_err(),
            "readiness dispatched the commit twice"
        );
        done_tx.send(()).unwrap();
        client_thread.join().unwrap();
    }

    #[test]
    fn dma_buf_commit_waits_for_producer_readiness_without_blocking_dispatch() {
        dma_buf_commit_readiness(false, false);
    }

    #[test]
    fn ready_dma_buf_commit_does_not_wait_for_another_event() {
        dma_buf_commit_readiness(true, false);
    }

    #[test]
    fn dma_buf_commit_readiness_holds_the_synchronized_parent_transaction() {
        dma_buf_commit_readiness(false, true);
    }

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
        assert_eq!(
            mapping_work(false, BufferTransition::Unchanged),
            MappingWork::None,
            "the initial bufferless configure stays unmapped"
        );
        assert_eq!(
            mapping_work(false, BufferTransition::Attached),
            MappingWork::Map,
            "the first real buffer runs mapping work"
        );
        for _ in 0..32 {
            assert_eq!(
                mapping_work(true, BufferTransition::Attached),
                MappingWork::None,
                "ordinary attached-buffer frames must not repeat mapping, metadata, focus, or relayout work"
            );
        }
        assert_eq!(
            mapping_work(true, BufferTransition::Unchanged),
            MappingWork::None,
            "metadata-only commits preserve mapping"
        );
        assert_eq!(
            mapping_work(true, BufferTransition::Removed),
            MappingWork::Unmap,
            "an explicit null buffer runs unmapping work"
        );
        assert_eq!(
            mapping_work(false, BufferTransition::Attached),
            MappingWork::Map,
            "a later real buffer remaps the same protocol role"
        );
    }
}
