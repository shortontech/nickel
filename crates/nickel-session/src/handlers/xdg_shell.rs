use nickel_session_protocol::ShellRole;
use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, Space, Window,
        find_popup_root_surface, get_popup_toplevel_coords,
    },
    input::{
        Seat,
        pointer::{Focus, GrabStartData as PointerGrabStartData},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Resource,
            protocol::{wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData, decoration::XdgDecorationHandler,
        },
    },
};

use crate::{
    NickelSession,
    focus::KeyboardFocusTarget,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    shell_layout,
};

fn is_codex_project_chat(app_id: Option<&str>) -> bool {
    app_id.is_some_and(|app_id| app_id.starts_with("io.nickel.codex.project."))
}

fn shell_owned_window_is_application(app_id: &str, shell_role: Option<ShellRole>) -> bool {
    !app_id.is_empty() && shell_role.is_none()
}

impl XdgShellHandler for NickelSession {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let surface_id = surface.wl_surface().id();
        let is_shell_client = surface
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .and_then(|credentials| u32::try_from(credentials.pid).ok())
            .is_some_and(|pid| self.is_authenticated_shell_pid(pid))
            || self.shell_windows().any(|window| {
                window
                    .toplevel()
                    .unwrap()
                    .wl_surface()
                    .id()
                    .same_client_as(&surface_id)
            });
        let cascade = i32::try_from(self.windows.len() % 8).unwrap_or(0) * 32;
        let id = self.windows.insert();
        if is_shell_client {
            self.shell_owned_windows.insert(id);
        } else {
            self.workspaces.add_window(id);
        }
        self.surface_windows.insert(surface.wl_surface().id(), id);
        // The authenticated shell creates role-sized SDL surfaces before the
        // app ID arrives. Giving those provisional surfaces an ordinary app
        // configure makes a hidden transient recreate as a full application
        // window and temporarily steals pointer hit testing from the panel.
        let geometry = (!is_shell_client)
            .then(|| self.output_geometry_for_shell())
            .flatten()
            .map(shell_layout::work_area)
            .map(|area| shell_layout::initial_window(area, cascade));
        if let Some(geometry) = geometry {
            surface.with_pending_state(|state| {
                state.size = Some((geometry.width, geometry.height).into());
            });
        }
        let wl_surface = surface.wl_surface().clone();
        let window = Window::new_wayland_window(surface);
        let location = geometry
            .map(|geometry| (geometry.x, geometry.y))
            .unwrap_or((cascade, cascade));
        self.space.map_element(window, location, true);
        if !is_shell_client && self.launcher_window.is_some() {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(wl_surface)),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
            self.workspaces.focused(&id);
        }
        for panel in self.panel_windows.clone() {
            self.space.raise_element(&panel, false);
        }
        self.notify_protocol_snapshot();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.forget_toplevel_geometry(&surface);
        if self
            .launcher_window
            .as_ref()
            .is_some_and(|window| window.toplevel().unwrap().wl_surface() == surface.wl_surface())
        {
            self.launcher_window = None;
            self.launcher_visibility.set(false);
        }
        self.panel_windows
            .retain(|window| window.toplevel().unwrap().wl_surface() != surface.wl_surface());
        self.desktop_windows
            .retain(|window| window.toplevel().unwrap().wl_surface() != surface.wl_surface());
        self.utility_windows
            .retain(|window| window.toplevel().unwrap().wl_surface() != surface.wl_surface());
        self.server_decorated.remove(&surface.wl_surface().id());
        if self
            .context_menu_window
            .as_ref()
            .is_some_and(|window| window.toplevel().unwrap().wl_surface() == surface.wl_surface())
        {
            self.context_menu_window = None;
        }
        if self
            .preview_window
            .as_ref()
            .is_some_and(|window| window.toplevel().unwrap().wl_surface() == surface.wl_surface())
        {
            self.preview_window = None;
        }
        if let Some(id) = self.surface_windows.remove(&surface.wl_surface().id()) {
            self.shell_owned_windows.remove(&id);
            self.minimized_windows.remove(&id);
            self.workspace_hidden_windows.remove(&id);
            self.workspaces.remove_window(&id);
            self.remove_window_from_switcher(id);
            self.windows.remove(id);
        }
        self.notify_protocol_snapshot();
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.update_window_metadata(&surface);
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        self.update_window_metadata(&surface);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.maximize_toplevel(&surface);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.unmaximize_toplevel(&surface);
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        self.fullscreen_toplevel(&surface);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.unfullscreen_toplevel(&surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(wl_surface))
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            let window = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(wl_surface))
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });

            surface.send_pending_configure();

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(seat) = Seat::from_resource(&seat) else {
            return;
        };
        let popup = PopupKind::Xdg(surface);
        let Ok(root_surface) = find_popup_root_surface(&popup) else {
            return;
        };
        let root = KeyboardFocusTarget::Wayland(root_surface);
        match self.popups.grab_popup(root, popup, &seat, serial) {
            Ok(grab) => {
                if let Some(keyboard) = seat.get_keyboard() {
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
            }
            Err(error) => tracing::debug!(?error, "denied invalid popup grab"),
        }
    }
}

// Xdg Shell
impl XdgDecorationHandler for NickelSession {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.prefer_server_decoration(toplevel);
    }

    fn request_mode(
        &mut self,
        toplevel: ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        self.configure_decoration(toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.prefer_server_decoration(toplevel);
    }
}

impl NickelSession {
    fn prefer_server_decoration(&mut self, toplevel: ToplevelSurface) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        self.configure_decoration(toplevel, Mode::ServerSide);
    }

    fn configure_decoration(
        &mut self,
        toplevel: ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        // Nickel's authenticated shell surfaces are already borderless UI.
        // Advertising server decorations makes SDL exclude a synthetic
        // titlebar strip from xdg-window-geometry, leaving visible controls in
        // that strip outside the compositor's pointer hit region.
        let shell_client = toplevel
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .and_then(|credentials| u32::try_from(credentials.pid).ok())
            .is_some_and(|pid| self.is_authenticated_shell_pid(pid));
        let mode = if shell_client { Mode::ClientSide } else { mode };
        match mode {
            Mode::ServerSide => {
                self.server_decorated.insert(toplevel.wl_surface().id());
            }
            Mode::ClientSide => {
                self.server_decorated.remove(&toplevel.wl_surface().id());
            }
            _ => return,
        }
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        toplevel.send_pending_configure();
    }
}

fn check_grab(
    seat: &Seat<NickelSession>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<NickelSession>> {
    let pointer = seat.get_pointer()?;

    // Check that this surface has a click grab.
    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;

    let (focus, _) = start_data.focus.as_ref()?;
    // If the focus was for a different surface, ignore the request.
    if !focus.same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface) {
    // Handle toplevel commits.
    if let Some(window) = space
        .elements()
        .find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        })
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    // NOTE: This should never fail as the initial configure is always
                    // allowed.
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl NickelSession {
    fn update_window_metadata(&mut self, surface: &ToplevelSurface) {
        let (title, app_id) = with_states(surface.wl_surface(), |states| {
            let attributes = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("xdg toplevel has role attributes")
                .lock()
                .expect("xdg toplevel attributes are not poisoned");
            (attributes.title.clone(), attributes.app_id.clone())
        });
        let client_pid = surface
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .and_then(|credentials| u32::try_from(credentials.pid).ok());
        let shell_role = client_pid
            .filter(|pid| self.is_authenticated_shell_pid(*pid))
            .and_then(|_| app_id.as_deref().and_then(ShellRole::from_application_id));
        let is_launcher = shell_role == Some(ShellRole::Launcher);
        let is_desktop = shell_role == Some(ShellRole::Desktop);
        let is_panel = shell_role == Some(ShellRole::Panel);
        let is_context_menu = shell_role == Some(ShellRole::ContextMenu);
        let is_preview = shell_role == Some(ShellRole::Preview);
        let is_notification = shell_role == Some(ShellRole::Notification);
        let is_lock = shell_role == Some(ShellRole::Lock);
        let is_codex_project_chat = is_codex_project_chat(app_id.as_deref());
        let is_utility = matches!(
            shell_role,
            Some(
                ShellRole::ControlCenter
                    | ShellRole::Notification
                    | ShellRole::ProjectMenu
                    | ShellRole::Recovery
            )
        );
        if let Some(id) = self
            .surface_windows
            .get(&surface.wl_surface().id())
            .copied()
        {
            self.windows.update_metadata(id, title, app_id);
            if shell_role.is_some() {
                self.workspaces.remove_window(&id);
            } else if self
                .windows
                .app_id(id)
                .is_some_and(|app_id| shell_owned_window_is_application(app_id, shell_role))
                && self.shell_owned_windows.remove(&id)
            {
                self.workspaces.add_window(id);
            }
        }
        self.notify_protocol_snapshot();
        if is_launcher && self.launcher_window.is_none() {
            let launcher = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(launcher) = launcher {
                self.register_launcher(launcher);
            }
        }
        if is_desktop {
            let desktop = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(desktop) = desktop {
                self.register_desktop(desktop);
            }
        }
        if is_panel {
            let panel = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(panel) = panel {
                self.register_panel(panel);
            }
        }
        if is_context_menu && self.context_menu_window.is_none() {
            let menu = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(menu) = menu {
                self.register_context_menu(menu);
            }
        }
        if is_preview && self.preview_window.is_none() {
            let preview = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(preview) = preview {
                self.register_preview(preview);
            }
        }
        if is_utility {
            let utility = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(utility) = utility {
                if is_notification {
                    utility.override_z_index(45);
                }
                self.register_utility_window(utility);
            }
        }
        if is_lock {
            let lock = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                .cloned();
            if let Some(lock) = lock {
                self.register_lock(lock);
            }
        }
        if let Some(role) = shell_role
            && self.pending_shell_focus_role == Some(role)
        {
            self.focus_shell_role(role);
        }
        // The SDL shell and its dynamic Codex windows share one Wayland
        // client. New toplevels from that client are deliberately not focused
        // until their role is known, so shell chrome cannot steal keyboard
        // focus while starting. Once metadata identifies an ordinary Codex
        // project window, complete the deferred focus handoff.
        if is_codex_project_chat {
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(surface.wl_surface().clone())),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
            if let Some(id) = self
                .surface_windows
                .get(&surface.wl_surface().id())
                .copied()
            {
                self.workspaces.focused(&id);
            }
            self.space.elements().for_each(|window| {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            });
        }
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self.space.elements().find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == &root)
        }) else {
            return;
        };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{is_codex_project_chat, shell_owned_window_is_application};
    use nickel_session_protocol::ShellRole;

    #[test]
    fn codex_project_chat_uses_canonical_application_identity() {
        assert!(is_codex_project_chat(Some(
            "io.nickel.codex.project.bd247278c96614ec"
        )));
        assert!(!is_codex_project_chat(Some("Codex — sentrygist")));
        assert!(!is_codex_project_chat(Some("io.nickel.shell")));
    }

    #[test]
    fn shell_client_stays_private_until_a_non_shell_identity_is_known() {
        assert!(!shell_owned_window_is_application("", None));
        assert!(!shell_owned_window_is_application(
            ShellRole::Preview.application_id(),
            Some(ShellRole::Preview)
        ));
        assert!(shell_owned_window_is_application(
            "io.nickel.codex.project.bd247278c96614ec",
            None
        ));
    }
}
