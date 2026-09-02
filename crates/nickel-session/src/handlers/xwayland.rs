use std::{collections::HashMap, hash::Hash, os::fd::OwnedFd, process::Stdio, time::Duration};

use smithay::{
    desktop::Window,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Rectangle, Size},
    wayland::{
        seat::WaylandFocus,
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, request_data_device_client_selection,
                set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, WmWindowProperty, X11Window, XwmId},
    },
};

use crate::{
    NickelSession,
    focus::KeyboardFocusTarget,
    grabs::{MoveSurfaceGrab, ResizeEdge, ResizeSurfaceGrab},
    handlers::{SelectionOwner, bounded_selection_mime_types},
    window_registry::{WindowAdmission, WindowId, WindowMetadataSource},
};

fn x11_retirement_identities<K: Clone + Eq + Hash>(
    window_ids: impl IntoIterator<Item = WindowId>,
    surface_windows: &HashMap<K, WindowId>,
) -> Vec<(Option<K>, WindowId)> {
    window_ids
        .into_iter()
        .map(|window_id| {
            let surface = surface_windows
                .iter()
                .find_map(|(surface, retained)| (*retained == window_id).then(|| surface.clone()));
            (surface, window_id)
        })
        .collect()
}

const XWAYLAND_RESTART_DELAY: Duration = Duration::from_secs(1);
const DEFAULT_X11_WIDTH: i32 = 800;
const DEFAULT_X11_HEIGHT: i32 = 600;

fn admit_managed_x11_window(
    windows: &mut crate::window_registry::WindowRegistry,
) -> Option<WindowId> {
    windows.insert(WindowAdmission::Ordinary)
}

fn x11_pointer_button(button: u32) -> Option<u32> {
    match button {
        1 => Some(0x110),
        2 => Some(0x112),
        3 => Some(0x111),
        _ => None,
    }
}

fn x11_map_geometry(
    mut surface_geometry: Rectangle<i32, Logical>,
    last_configure: Rectangle<i32, Logical>,
    override_redirect: bool,
) -> Rectangle<i32, Logical> {
    if override_redirect {
        // Override-redirect windows (menus, tooltips, and similar popups) are
        // positioned directly by the X11 client. `geometry()` is local to the
        // surface and therefore does not contain that global position.
        surface_geometry.loc = last_configure.loc;
    }
    surface_geometry
}

impl NickelSession {
    pub(crate) fn start_xwayland(&mut self) {
        self.xwayland_restart_pending = false;
        if let Some(registration) = self.xwayland_registration.take() {
            self.event_loop_handle.remove(registration);
        }
        if std::env::var_os("NICKEL_DISABLE_XWAYLAND").is_some() {
            tracing::info!("XWayland disabled by NICKEL_DISABLE_XWAYLAND");
            return;
        }
        let spawned = XWayland::spawn(
            &self.display_handle,
            self.xwayland_display,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| {},
        );
        let (xwayland, client) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
                tracing::warn!(%error, "XWayland is unavailable; native Wayland remains active");
                self.schedule_xwayland_restart();
                return;
            }
        };
        let display = xwayland.display_number();
        self.xwayland_display = Some(display);
        // SAFETY: all compositor environment updates happen on the event-loop
        // thread, before newly supervised application processes are spawned.
        unsafe { std::env::set_var("DISPLAY", format!(":{display}")) };
        let handle = self.event_loop_handle.clone();
        match handle
            .clone()
            .insert_source(xwayland, move |event, _, state| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => match X11Wm::start_wm(
                    handle.clone(),
                    &state.display_handle,
                    x11_socket,
                    client.clone(),
                ) {
                    Ok(xwm) => {
                        tracing::info!(display = display_number, "XWayland window manager ready");
                        state.xwm = Some((xwm.id(), xwm));
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to start XWayland window manager");
                        state.schedule_xwayland_restart();
                    }
                },
                XWaylandEvent::Error => {
                    tracing::error!("XWayland failed during startup");
                    state.schedule_xwayland_restart();
                }
            }) {
            Ok(registration) => self.xwayland_registration = Some(registration),
            Err(error) => {
                tracing::error!(%error, "failed to register XWayland event source");
                self.schedule_xwayland_restart();
            }
        }
    }

    fn schedule_xwayland_restart(&mut self) {
        if self.xwayland_restart_pending {
            return;
        }
        self.xwayland_restart_pending = true;
        let timer = Timer::from_duration(XWAYLAND_RESTART_DELAY);
        if let Err(error) = self.event_loop_handle.insert_source(timer, |_, _, state| {
            state.start_xwayland();
            TimeoutAction::Drop
        }) {
            self.xwayland_restart_pending = false;
            tracing::error!(%error, "failed to schedule XWayland restart");
        }
    }

    fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .find(|window| window.x11_surface() == Some(surface))
            .cloned()
    }

    fn x11_window_id(&self, surface: &X11Surface) -> Option<WindowId> {
        self.x11_windows.get(&surface.window_id()).copied()
    }

    pub(crate) fn raise_x11_surface(&mut self, surface: &X11Surface) {
        if let Some((_, xwm)) = self.xwm.as_mut()
            && let Err(error) = xwm.raise_window(surface)
        {
            tracing::warn!(
                ?error,
                window = surface.window_id(),
                "failed to raise X11 window"
            );
        }
    }

    fn map_x11_window(&mut self, surface: X11Surface, managed: bool) {
        if self.x11_window(&surface).is_some() {
            return;
        }
        let mut geometry = x11_map_geometry(
            surface.geometry(),
            surface.last_configure(),
            surface.is_override_redirect(),
        );
        if geometry.size.w <= 1 || geometry.size.h <= 1 {
            geometry.size = Size::from((DEFAULT_X11_WIDTH, DEFAULT_X11_HEIGHT));
            let _ = surface.configure(geometry);
        }
        if managed {
            let clamped = self.clamp_initial_managed_x11_geometry(geometry);
            if clamped != geometry {
                geometry = clamped;
                let _ = surface.configure(geometry);
            }
        }
        let window = Window::new_x11_window(surface.clone());
        self.space.map_element(window.clone(), geometry.loc, true);
        if managed {
            let Some(id) = admit_managed_x11_window(&mut self.windows) else {
                tracing::warn!(
                    window = surface.window_id(),
                    limit = nickel_session_protocol::MAX_WINDOWS,
                    "rejected X11 window because the live window limit was reached"
                );
                self.space.unmap_elem(&window);
                if let Err(error) = surface.set_mapped(false) {
                    tracing::warn!(?error, "failed to unmap rejected X11 window");
                }
                self.request_output_redraw();
                return;
            };
            self.workspaces.add_window(id);
            self.windows.update_metadata(
                id,
                WindowMetadataSource::X11,
                Some(surface.title()),
                Some(surface.class()),
            );
            self.x11_windows.insert(surface.window_id(), id);
            if let Some(wl_surface) = surface.wl_surface() {
                self.surface_windows.insert(wl_surface.id(), id);
            }
            self.space.elements().for_each(|candidate| {
                candidate.set_activated(candidate == &window);
            });
            self.raise_x11_surface(&surface);
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::X11(surface.clone())),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
            self.workspaces.focused(&id);
            if surface.is_maximized() {
                self.apply_maximized_x11_geometry(&window, &surface, true);
            }
            self.space.elements().for_each(|candidate| {
                if let Some(toplevel) = candidate.toplevel() {
                    toplevel.send_pending_configure();
                }
            });
        }
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }

    fn remove_x11_window(&mut self, surface: &X11Surface) {
        self.forget_x11_geometry(surface);
        if let Some(window) = self.x11_window(surface) {
            self.space.unmap_elem(&window);
        }
        let window_id = self.x11_windows.remove(&surface.window_id());
        let restore_focus = window_id.is_some_and(|id| self.windows.is_active(id));
        let surface_id = surface.wl_surface().map(|surface| surface.id());
        self.retire_surface_window_references(surface_id.as_ref(), window_id);
        if window_id.is_some() {
            self.restore_focus_after_window_removal(restore_focus);
        }
        self.request_output_redraw();
        self.notify_protocol_snapshot();
    }
}

impl XWaylandShellHandler for NickelSession {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, _xwm: XwmId, wl_surface: WlSurface, surface: X11Surface) {
        if let Some(id) = self.x11_window_id(&surface) {
            self.surface_windows.insert(wl_surface.id(), id);
        }
        self.request_output_redraw();
    }
}

impl XwmHandler for NickelSession {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        let (current, state) = self.xwm.as_mut().expect("XWM callback requires live state");
        assert_eq!(*current, xwm, "XWM callback came from stale instance");
        state
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Err(error) = window.set_mapped(true) {
            tracing::warn!(
                ?error,
                window = window.window_id(),
                "failed to map X11 window"
            );
            return;
        }
        self.map_x11_window(window, true);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.map_x11_window(window, false);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.remove_x11_window(&window);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<u32>,
        height: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if let Some(mapped) = self.x11_window(&window)
            && self.is_maximized_window(&mapped)
        {
            self.apply_maximized_x11_geometry(&mapped, &window, false);
            self.request_output_redraw();
            return;
        }
        let old = window.geometry();
        let geometry = Rectangle::new(
            (x.unwrap_or(old.loc.x), y.unwrap_or(old.loc.y)).into(),
            (
                width
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or(old.size.w)
                    .max(1),
                height
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or(old.size.h)
                    .max(1),
            )
                .into(),
        );
        if let Err(error) = window.configure(geometry) {
            tracing::warn!(?error, window = window.window_id(), "X11 configure failed");
        }
        if let Some(mapped) = self.x11_window(&window) {
            self.space.map_element(mapped, geometry.loc, false);
        }
        self.request_output_redraw();
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if let Some(mapped) = self.x11_window(&window) {
            self.space.map_element(mapped, geometry.loc, false);
            self.request_output_redraw();
        }
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, _property: WmWindowProperty) {
        if let Some(id) = self.x11_window_id(&window) {
            self.windows.update_metadata(
                id,
                WindowMetadataSource::X11,
                Some(window.title()),
                Some(window.class()),
            );
            self.notify_protocol_snapshot();
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_window_id(&window) {
            self.maximize_window(id);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if window.is_maximized()
            && let Some(id) = self.x11_window_id(&window)
        {
            self.maximize_window(id);
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.fullscreen_x11(&window);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.unfullscreen_x11(&window);
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_window_id(&window) {
            self.minimize_window(id);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(id) = self.x11_window_id(&window) {
            self.activate_window(id);
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        button: u32,
        resize_edge: X11ResizeEdge,
    ) {
        let pointer = self.seat.get_pointer().unwrap();
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };
        if x11_pointer_button(button) != Some(start_data.button)
            || start_data.focus.as_ref().map(|(surface, _)| surface)
                != Some(&crate::focus::PointerFocusTarget::X11(window.clone()))
        {
            return;
        }
        let Some(mapped) = self.x11_window(&window) else {
            return;
        };
        let Some(initial_window_location) = self.space.element_location(&mapped) else {
            return;
        };
        let initial_rect = Rectangle::new(initial_window_location, mapped.geometry().size);
        pointer.set_grab(
            self,
            ResizeSurfaceGrab::start(
                start_data,
                mapped,
                ResizeEdge::from(resize_edge),
                initial_rect,
            ),
            smithay::utils::SERIAL_COUNTER.next_serial(),
            smithay::input::pointer::Focus::Clear,
        );
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, button: u32) {
        let pointer = self.seat.get_pointer().unwrap();
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };
        if x11_pointer_button(button) != Some(start_data.button)
            || start_data.focus.as_ref().map(|(surface, _)| surface)
                != Some(&crate::focus::PointerFocusTarget::X11(window.clone()))
        {
            return;
        }
        let Some(mapped) = self.x11_window(&window) else {
            return;
        };
        let Some(initial_window_location) = self.space.element_location(&mapped) else {
            return;
        };
        pointer.set_grab(
            self,
            MoveSurfaceGrab {
                start_data,
                window: mapped,
                initial_window_location,
                restored_from_maximized: false,
            },
            smithay::utils::SERIAL_COUNTER.next_serial(),
            smithay::input::pointer::Focus::Clear,
        );
    }

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        true
    }

    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let result = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|error| error.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|error| error.to_string()),
        };
        if let Err(error) = result {
            tracing::debug!(
                ?error,
                ?selection,
                "no Wayland selection was available to XWayland"
            );
        } else if let Err(error) = self.display_handle.flush_clients() {
            tracing::warn!(
                ?error,
                ?selection,
                "failed to flush Wayland selection request for XWayland"
            );
        }
    }

    fn new_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        let mime_types = bounded_selection_mime_types(mime_types);
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                SelectionOwner::XWayland(xwm),
            ),
            SelectionTarget::Primary => set_primary_selection(
                &self.display_handle,
                &self.seat,
                mime_types,
                SelectionOwner::XWayland(xwm),
            ),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                clear_data_device_selection(&self.display_handle, &self.seat)
            }
            SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
        }
    }

    fn disconnected(&mut self, xwm: XwmId) {
        if self
            .xwm
            .as_ref()
            .is_some_and(|(current, _)| *current == xwm)
        {
            self.xwm = None;
        }
        self.retire_xwm_windows();
        self.schedule_xwayland_restart();
    }
}

impl NickelSession {
    fn retire_xwm_windows(&mut self) {
        let x11_surfaces = self
            .space
            .elements()
            .filter_map(|window| window.x11_surface().cloned())
            .collect::<Vec<_>>();
        let x11_windows = self
            .space
            .elements()
            .filter(|window| window.is_x11())
            .cloned()
            .collect::<Vec<_>>();
        for surface in &x11_surfaces {
            self.forget_x11_geometry(surface);
        }
        for window in x11_windows {
            self.space.unmap_elem(&window);
        }
        self.forget_all_x11_geometry();
        let focused_disconnected_x11 = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| match focus {
                KeyboardFocusTarget::X11(surface) => {
                    self.x11_windows.get(&surface.window_id()).copied()
                }
                KeyboardFocusTarget::Wayland(_) => None,
            })
            .is_some();
        let removed_ids = self
            .x11_windows
            .drain()
            .map(|(_, id)| id)
            .collect::<Vec<_>>();
        let removed = x11_retirement_identities(removed_ids, &self.surface_windows);
        for (surface_id, id) in removed {
            self.retire_surface_window_references(surface_id.as_ref(), Some(id));
        }
        if focused_disconnected_x11 {
            if self.locked {
                let lock_focus = self
                    .lock_windows
                    .first()
                    .and_then(Window::wl_surface)
                    .map(|surface| KeyboardFocusTarget::Wayland(surface.into_owned()));
                self.seat.get_keyboard().unwrap().set_focus(
                    self,
                    lock_focus,
                    smithay::utils::SERIAL_COUNTER.next_serial(),
                );
            } else {
                self.restore_focus_after_window_removal(true);
            }
        }
        self.request_output_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::{
        calloop::EventLoop,
        wayland_server::{Display, protocol::wl_surface::WlSurface},
    };
    use smithay_client_toolkit::reexports::client::{
        Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
        protocol::{wl_compositor, wl_registry, wl_surface},
    };
    use std::{
        os::unix::net::UnixStream,
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    struct FocusClient {
        compositor: Option<wl_compositor::WlCompositor>,
        surface: Option<wl_surface::WlSurface>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for FocusClient {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _data: &(),
            _connection: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
                && interface == "wl_compositor"
            {
                let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                    name,
                    version.min(6),
                    qh,
                    (),
                );
                state.surface = Some(compositor.create_surface(qh, ()));
                state.compositor = Some(compositor);
            }
        }
    }

    delegate_noop!(FocusClient: ignore wl_compositor::WlCompositor);
    delegate_noop!(FocusClient: ignore wl_surface::WlSurface);

    struct FocusClientGuard {
        done: mpsc::Sender<()>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Drop for FocusClientGuard {
        fn drop(&mut self) {
            let _ = self.done.send(());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn session_with_wayland_focus() -> (NickelSession, WlSurface, FocusClientGuard) {
        let mut event_loop = EventLoop::try_new().expect("test event loop");
        let display = Display::new().expect("test Wayland display");
        let mut display_handle = display.handle();
        let (server, peer) = UnixStream::pair().expect("test Wayland socket pair");
        let server_client = display_handle
            .insert_client(server, Arc::new(crate::state::ClientState::default()))
            .expect("test Wayland client");
        let mut session = NickelSession::new(&mut event_loop, display, false);
        let (surface_tx, surface_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let client_thread = thread::spawn(move || {
            let connection = Connection::from_socket(peer).expect("test client connection");
            let mut event_queue = connection.new_event_queue::<FocusClient>();
            let qh = event_queue.handle();
            connection.display().get_registry(&qh, ());
            let mut client = FocusClient {
                compositor: None,
                surface: None,
            };
            event_queue
                .roundtrip(&mut client)
                .expect("test registry roundtrip");
            connection.flush().expect("flush focus surface request");
            let surface_id = client
                .surface
                .as_ref()
                .expect("compositor advertised")
                .id()
                .protocol_id();
            surface_tx.send(surface_id).expect("publish focus surface");
            let _ = done_rx.recv();
        });
        let surface_id = loop {
            if let Ok(surface_id) = surface_rx.try_recv() {
                break surface_id;
            }
            event_loop
                .dispatch(Some(Duration::from_millis(20)), &mut session)
                .expect("dispatch test Wayland client");
        };
        event_loop
            .dispatch(Some(Duration::from_millis(20)), &mut session)
            .expect("dispatch focus surface creation");
        let focus_surface = server_client
            .object_from_protocol_id::<WlSurface>(&session.display_handle, surface_id)
            .expect("server focus surface");
        let keyboard = session.seat.get_keyboard().expect("session keyboard");
        keyboard.set_focus(
            &mut session,
            Some(KeyboardFocusTarget::Wayland(focus_surface.clone())),
            smithay::utils::SERIAL_COUNTER.next_serial(),
        );
        (
            session,
            focus_surface,
            FocusClientGuard {
                done: done_tx,
                thread: Some(client_thread),
            },
        )
    }

    #[test]
    fn xwm_disconnect_retires_every_window_with_or_without_a_wayland_surface() {
        let mapped = WindowId(41);
        let surface_less = WindowId(42);
        let surfaces = HashMap::from([(7_u32, mapped)]);

        let retired = x11_retirement_identities([mapped, surface_less], &surfaces);

        assert_eq!(retired, [(Some(7), mapped), (None, surface_less)]);
    }

    #[test]
    fn background_xwm_teardown_preserves_wayland_and_lock_focus() {
        let (mut session, focus_surface, _client) = session_with_wayland_focus();

        for locked in [false, true] {
            session.locked = locked;
            let background = session
                .windows
                .insert_inactive(WindowAdmission::Ordinary)
                .expect("background X11 test window should be admitted");
            session
                .x11_windows
                .insert(70 + u32::from(locked), background);

            session.retire_xwm_windows();

            assert!(session.x11_windows.is_empty());
            assert!(!session.windows.contains(background));
            assert_eq!(
                session
                    .seat
                    .get_keyboard()
                    .and_then(|keyboard| keyboard.current_focus()),
                Some(KeyboardFocusTarget::Wayland(focus_surface.clone())),
                "background X11 teardown changed Wayland focus while locked={locked}"
            );
        }
    }

    #[test]
    fn managed_x11_admission_respects_the_ordinary_window_watermark() {
        let mut windows = crate::window_registry::WindowRegistry::default();
        let ordinary_capacity = nickel_session_protocol::MAX_WINDOWS
            - crate::window_registry::RESERVED_AUTHENTICATED_SHELL_WINDOWS;
        for _ in 0..ordinary_capacity {
            assert!(admit_managed_x11_window(&mut windows).is_some());
        }
        assert_eq!(admit_managed_x11_window(&mut windows), None);
        assert_eq!(windows.len(), ordinary_capacity);
    }

    #[test]
    fn override_redirect_windows_use_the_client_configured_location() {
        let surface_geometry = Rectangle::new((7, 0).into(), (320, 240).into());
        let last_configure = Rectangle::new((843, 612).into(), (300, 200).into());

        let mapped = x11_map_geometry(surface_geometry, last_configure, true);

        assert_eq!(mapped.loc, (843, 612).into());
        assert_eq!(mapped.size, (320, 240).into());
    }

    #[test]
    fn managed_windows_keep_their_surface_geometry() {
        let surface_geometry = Rectangle::new((7, 11).into(), (320, 240).into());
        let last_configure = Rectangle::new((843, 612).into(), (300, 200).into());

        assert_eq!(
            x11_map_geometry(surface_geometry, last_configure, false),
            surface_geometry
        );
    }
}
