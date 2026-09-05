use nickel_session_protocol::ShellRole;
use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, Window,
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
    utils::{Logical, Point, Rectangle, Serial},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
            decoration::XdgDecorationHandler,
            dialog::{ToplevelDialogHint, XdgDialogHandler},
        },
    },
};

use crate::{
    NickelSession,
    focus::KeyboardFocusTarget,
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    shell_layout,
    window_registry::{WindowAdmission, WindowId, WindowMetadataSource, WindowRegistry},
};

fn admit_xdg_toplevel(
    windows: &mut WindowRegistry,
    authenticated_shell_client: bool,
) -> Option<WindowId> {
    windows.insert_inactive(if authenticated_shell_client {
        WindowAdmission::AuthenticatedShell
    } else {
        WindowAdmission::Ordinary
    })
}

fn is_codex_project_chat(app_id: Option<&str>) -> bool {
    app_id.is_some_and(|app_id| app_id.starts_with("io.nickel.codex.project."))
}

fn shell_owned_window_is_application(app_id: &str, shell_role: Option<ShellRole>) -> bool {
    !app_id.is_empty() && shell_role.is_none() && ShellRole::from_application_id(app_id).is_none()
}

fn popup_output_for_anchor(
    outputs: impl IntoIterator<Item = Rectangle<i32, Logical>>,
    anchor: Point<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    outputs
        .into_iter()
        .filter(|geometry| geometry.contains(anchor))
        .min_by_key(|geometry| {
            (
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            )
        })
}

fn popup_constraint_area(output: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let area = shell_layout::work_area(shell_layout::Geometry {
        x: output.loc.x,
        y: output.loc.y,
        width: output.size.w,
        height: output.size.h,
    });
    Rectangle::new((area.x, area.y).into(), (area.width, area.height).into())
}

fn unauthenticated_reserved_shell_role(
    app_id: Option<&str>,
    authenticated: bool,
) -> Option<ShellRole> {
    (!authenticated)
        .then(|| app_id.and_then(ShellRole::from_application_id))
        .flatten()
}

fn new_toplevel_may_focus(current_focus_is_shell: Option<bool>, has_parent: bool) -> bool {
    has_parent || current_focus_is_shell.unwrap_or(true)
}

fn parent_relative_dialog_location(
    parent: Rectangle<i32, Logical>,
    child: (i32, i32),
    work_area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    let centered = shell_layout::centered_in(
        shell_layout::Geometry {
            x: parent.loc.x,
            y: parent.loc.y,
            width: parent.size.w,
            height: parent.size.h,
        },
        child,
    );
    let constrained = shell_layout::constrain_to_area(
        centered,
        shell_layout::Geometry {
            x: work_area.loc.x,
            y: work_area.loc.y,
            width: work_area.size.w,
            height: work_area.size.h,
        },
    );
    (constrained.x, constrained.y).into()
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
        let Some(id) = admit_xdg_toplevel(&mut self.windows, is_shell_client) else {
            tracing::warn!(
                limit = nickel_session_protocol::MAX_WINDOWS,
                "rejected XDG toplevel because the live window limit was reached"
            );
            surface.send_close();
            return;
        };
        if is_shell_client {
            self.shell_owned_windows.insert(id);
        } else {
            self.workspaces.add_window(id);
        }
        self.surface_windows.insert(surface.wl_surface().id(), id);
        // The authenticated shell creates role-sized surfaces before the
        // app ID arrives. Giving those provisional surfaces an ordinary app
        // configure makes a hidden transient recreate as a full application
        // window and temporarily steals pointer hit testing from the panel.
        let parent_output = surface.parent().and_then(|parent| {
            self.xdg_toplevel_window(&parent)
                .and_then(|window| self.output_name_for_window(&window))
        });
        let active_output = self.new_window_active_output_name();
        let placement = (!is_shell_client)
            .then(|| {
                shell_layout::resolve_window_output(
                    &self.placement_outputs(),
                    parent_output.as_deref(),
                    None,
                    None,
                    active_output.as_deref(),
                )
            })
            .flatten();
        if let Some(placement) = &placement {
            tracing::info!(
                surface = ?surface_id,
                output = %placement.output_name,
                reason = ?placement.reason,
                "diagnostic: captured Wayland new-window placement"
            );
        }
        let geometry =
            placement.map(|placement| shell_layout::initial_window(placement.work_area, cascade));
        if let Some(geometry) = geometry {
            surface.with_pending_state(|state| {
                state.size = Some((geometry.width, geometry.height).into());
            });
        }
        let window = Window::new_wayland_window(surface);
        let location = geometry
            .map(|geometry| (geometry.x, geometry.y))
            .unwrap_or((cascade, cascade));
        self.xdg_toplevel_locations
            .insert(surface_id.clone(), location.into());
        self.xdg_toplevel_windows.insert(surface_id, window);
        self.notify_protocol_snapshot();
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.forget_toplevel_geometry(&surface);
        let surface_id = surface.wl_surface().id();
        if let Some(window) = self.xdg_toplevel_windows.get(&surface_id).cloned() {
            self.space.unmap_elem(&window);
        }
        self.mapped_xdg_toplevels.remove(&surface_id);
        self.restored_xdg_toplevels.remove(&surface_id);
        self.xdg_toplevel_windows.remove(&surface_id);
        self.xdg_toplevel_locations.remove(&surface_id);
        let window_id = self.surface_windows.get(&surface_id).copied();
        let destroyed_surface_had_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| {
                focus
                    .wl_surface()
                    .is_some_and(|focused| focused.id() == surface_id)
            });
        let restore_focus = destroyed_surface_had_focus
            || window_id.is_some_and(|id| {
                self.windows.is_active(id) && !self.shell_owned_windows.contains(&id)
            });
        self.retire_surface_window_references(Some(&surface_id), window_id);
        if window_id.is_some() {
            self.restore_focus_after_window_removal(restore_focus);
        }
        self.notify_protocol_snapshot();
        self.request_output_redraw();
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
        if self.unconstrain_popup(&surface) {
            let _ = self.popups.track_popup(PopupKind::Xdg(surface));
        }
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
        if self.unconstrain_popup(&surface) {
            surface.send_repositioned(token);
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();
        tracing::info!(
            surface = ?wl_surface.id(),
            ?serial,
            "diagnostic: xdg toplevel move requested"
        );

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            tracing::info!(
                surface = ?wl_surface.id(),
                ?serial,
                "diagnostic: xdg toplevel move accepted"
            );
            let pointer = seat.get_pointer().unwrap();

            let Some(window) = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(wl_surface))
                .cloned()
            else {
                return;
            };
            if self.is_shell_owned_window(&window) {
                tracing::warn!("ignored move request from compositor-owned shell surface");
                return;
            }
            let Some(initial_window_location) = self.space.element_location(&window) else {
                return;
            };

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
                restored_from_maximized: false,
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

            let Some(window) = self
                .space
                .elements()
                .find(|window| window.wl_surface().as_deref() == Some(wl_surface))
                .cloned()
            else {
                return;
            };
            if self.is_shell_owned_window(&window) {
                tracing::warn!("ignored resize request from compositor-owned shell surface");
                return;
            }
            let Some(initial_window_location) = self.space.element_location(&window) else {
                return;
            };
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
        let popup_id = surface.wl_surface().id();
        let popup = PopupKind::Xdg(surface);
        let Ok(root_surface) = find_popup_root_surface(&popup) else {
            return;
        };
        let root = KeyboardFocusTarget::Wayland(root_surface);
        tracing::info!(
            popup = ?popup_id,
            root = ?root.wl_surface().map(|surface| surface.id()),
            ?serial,
            "diagnostic: xdg popup requested a seat grab"
        );
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

impl XdgDialogHandler for NickelSession {
    fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, hint: ToplevelDialogHint) {
        let Some(window) = self.xdg_toplevel_window(toplevel.wl_surface()) else {
            return;
        };
        tracing::info!(surface = ?toplevel.wl_surface().id(), ?hint, "xdg dialog hint changed");
        if hint == ToplevelDialogHint::Modal && self.space.element_location(&window).is_some() {
            self.space.raise_element(&window, true);
            self.space.elements().for_each(|candidate| {
                candidate.set_activated(candidate == &window);
            });
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(toplevel.wl_surface().clone())),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
            self.raise_panels();
            self.request_output_redraw();
        }
    }
}

// Xdg Shell
impl XdgDecorationHandler for NickelSession {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        tracing::info!(
            surface = ?toplevel.wl_surface().id(),
            "diagnostic: xdg decoration object created"
        );
        self.prefer_server_decoration(toplevel);
    }

    fn request_mode(
        &mut self,
        toplevel: ToplevelSurface,
        mode: smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
    ) {
        tracing::info!(
            surface = ?toplevel.wl_surface().id(),
            ?mode,
            "diagnostic: xdg decoration mode requested"
        );
        self.configure_decoration(toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        tracing::info!(
            surface = ?toplevel.wl_surface().id(),
            "diagnostic: xdg decoration mode unset"
        );
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        // A client can unset its server-decoration preference at runtime when
        // switching back to its own titlebar. Keeping ServerSide here leaves
        // both the compositor frame and the new client-side frame visible.
        self.configure_decoration(toplevel, Mode::ClientSide);
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
        // Advertising server decorations makes the windowing client exclude a
        // synthetic titlebar strip from xdg-window-geometry, leaving visible
        // controls in that strip outside the compositor's pointer hit region.
        let shell_client = toplevel
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(&self.display_handle).ok())
            .and_then(|credentials| u32::try_from(credentials.pid).ok())
            .is_some_and(|pid| self.is_authenticated_shell_pid(pid));
        let mode = if shell_client { Mode::ClientSide } else { mode };
        let decoration_changed = match mode {
            Mode::ServerSide => self.server_decorated.insert(toplevel.wl_surface().id()),
            Mode::ClientSide => self.server_decorated.remove(&toplevel.wl_surface().id()),
            _ => return,
        };
        tracing::info!(
            surface = ?toplevel.wl_surface().id(),
            ?mode,
            decoration_changed,
            server_decorated = self.server_decorated.contains(&toplevel.wl_surface().id()),
            "diagnostic: xdg decoration state configured"
        );
        if decoration_changed {
            #[cfg(feature = "backend-udev")]
            self.invalidate_native_outputs();
        }
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        if decoration_changed {
            self.reconcile_maximized_toplevel_geometry(&toplevel);
        }
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
    let has_grab = pointer.has_grab(serial);
    if !has_grab {
        tracing::info!(
            surface = ?surface.id(),
            ?serial,
            "diagnostic: xdg interactive request rejected without matching pointer grab"
        );
        return None;
    }

    let Some(start_data) = pointer.grab_start_data() else {
        tracing::info!(
            surface = ?surface.id(),
            ?serial,
            "diagnostic: xdg interactive request rejected without grab start data"
        );
        return None;
    };

    let Some((focus, _)) = start_data.focus.as_ref() else {
        tracing::info!(
            surface = ?surface.id(),
            ?serial,
            "diagnostic: xdg interactive request rejected without grab focus"
        );
        return None;
    };
    // If the focus was for a different surface, ignore the request.
    if !focus.same_client_as(&surface.id()) {
        tracing::info!(
            surface = ?surface.id(),
            ?serial,
            focus = ?focus.wl_surface().map(|surface| surface.id()),
            "diagnostic: xdg interactive request rejected for mismatched focus client"
        );
        return None;
    }

    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, window: Option<Window>, surface: &WlSurface) {
    // Handle toplevel commits.
    if let Some(window) = window {
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
    pub(crate) fn xdg_toplevel_window(&self, surface: &WlSurface) -> Option<Window> {
        self.xdg_toplevel_windows.get(&surface.id()).cloned()
    }

    pub(crate) fn map_xdg_toplevel(&mut self, surface: &WlSurface) -> Option<Window> {
        let surface_id = surface.id();
        let window = self.xdg_toplevel_windows.get(&surface_id)?.clone();
        let current_focus_is_shell =
            self.seat
                .get_keyboard()
                .unwrap()
                .current_focus()
                .map(|focus| {
                    focus.wl_surface().is_some_and(|focused| {
                        self.shell_windows()
                            .filter_map(Window::wl_surface)
                            .any(|shell| shell.as_ref() == focused.as_ref())
                    })
                });
        if !self.mapped_xdg_toplevels.insert(surface_id.clone()) {
            // Every rendered frame normally attaches a buffer. Mapping,
            // metadata classification, focus, and shell relayout are
            // transition work and must run only for the first buffer after an
            // unmapped state.
            return Some(window);
        }
        let restoring = self.restored_xdg_toplevels.remove(&surface_id);
        let mut location = self
            .xdg_toplevel_locations
            .get(&surface_id)
            .copied()
            .unwrap_or_default();
        let outputs = self.placement_outputs();
        let size = window.geometry().size;
        let captured = shell_layout::Geometry {
            x: location.x,
            y: location.y,
            width: size.w.max(1),
            height: size.h.max(1),
        };
        let parent_window = window.toplevel().and_then(|toplevel| {
            toplevel
                .parent()
                .and_then(|parent| self.xdg_toplevel_window(&parent))
        });
        let parent_output = parent_window
            .as_ref()
            .and_then(|parent| self.output_name_for_window(parent));
        let restored_output = restoring
            .then(|| {
                outputs
                    .iter()
                    .find(|output| {
                        shell_layout::is_reachable(captured, std::slice::from_ref(output))
                    })
                    .map(|output| output.name.clone())
            })
            .flatten();
        if parent_output.is_some() || restoring || !shell_layout::is_reachable(captured, &outputs) {
            let active_output = self.new_window_active_output_name();
            if let Some(decision) = shell_layout::resolve_window_output(
                &outputs,
                parent_output.as_deref(),
                None,
                restored_output.as_deref(),
                active_output.as_deref(),
            ) {
                let replacement = if decision.reason == shell_layout::PlacementReason::Restored {
                    shell_layout::constrain_to_area(captured, decision.work_area)
                } else if let Some(parent_bounds) = parent_window
                    .as_ref()
                    .and_then(|parent| self.space.element_geometry(parent))
                {
                    let location = parent_relative_dialog_location(
                        parent_bounds,
                        (captured.width, captured.height),
                        Rectangle::new(
                            (decision.work_area.x, decision.work_area.y).into(),
                            (decision.work_area.width, decision.work_area.height).into(),
                        ),
                    );
                    shell_layout::Geometry {
                        x: location.x,
                        y: location.y,
                        width: captured.width,
                        height: captured.height,
                    }
                } else {
                    shell_layout::centered_in(decision.work_area, (captured.width, captured.height))
                };
                location = (replacement.x, replacement.y).into();
                self.xdg_toplevel_locations
                    .insert(surface_id.clone(), location);
                tracing::info!(
                    surface = ?surface_id,
                    output = %decision.output_name,
                    reason = ?decision.reason,
                    "diagnostic: revalidated Wayland new-window placement before map"
                );
            }
        }
        self.space.map_element(window.clone(), location, true);
        if let Some(toplevel) = window.toplevel() {
            self.update_window_metadata(toplevel);
        }
        let registry_id = self.surface_windows.get(&surface_id).copied();
        if let Some(id) = registry_id.filter(|id| {
            !self.shell_owned_windows.contains(id)
                && new_toplevel_may_focus(current_focus_is_shell, parent_window.is_some())
        }) {
            self.windows.raise(id);
            self.workspaces.focused(&id);
            self.space.elements().for_each(|candidate| {
                candidate.set_activated(candidate == &window);
            });
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(surface.clone())),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
        }
        self.raise_panels();
        Some(window)
    }

    pub(crate) fn unmap_xdg_toplevel(&mut self, surface: &WlSurface) -> Option<Window> {
        let surface_id = surface.id();
        let window = self.xdg_toplevel_windows.get(&surface_id)?.clone();
        if !self.mapped_xdg_toplevels.remove(&surface_id) {
            return Some(window);
        }
        self.restored_xdg_toplevels.insert(surface_id.clone());
        if let Some(location) = self.space.element_location(&window) {
            self.xdg_toplevel_locations
                .insert(surface_id.clone(), location);
        }
        let had_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .is_some_and(|focus| {
                focus
                    .wl_surface()
                    .is_some_and(|focused| focused.id() == surface_id)
            });
        self.space.unmap_elem(&window);
        let registry_id = self.surface_windows.get(&surface_id).copied();
        window.set_activated(false);
        self.restore_focus_after_window_removal(
            had_focus || registry_id.is_some_and(|id| self.windows.is_active(id)),
        );
        Some(window)
    }

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
        let authenticated = client_pid.is_some_and(|pid| self.is_authenticated_shell_pid(pid));
        let registry_id = self
            .surface_windows
            .get(&surface.wl_surface().id())
            .copied();
        if let Some(id) = registry_id {
            self.windows
                .update_metadata(id, WindowMetadataSource::Xdg, title, app_id);
        }
        // The registry is the canonical session projection. Role, grouping,
        // decoration, and protocol consumers all derive from the same bounded
        // application identity rather than the protocol role's unbounded copy.
        let projected_app_id = registry_id.and_then(|id| self.windows.app_id(id));
        let shell_role = client_pid
            .filter(|_| authenticated)
            .and_then(|_| projected_app_id.and_then(ShellRole::from_application_id));
        let is_launcher = shell_role == Some(ShellRole::Launcher);
        let is_desktop = shell_role == Some(ShellRole::Desktop);
        let is_panel = shell_role == Some(ShellRole::Panel);
        let is_context_menu = shell_role == Some(ShellRole::ContextMenu);
        let is_preview = shell_role == Some(ShellRole::Preview);
        let is_notification = shell_role == Some(ShellRole::Notification);
        let is_lock = shell_role == Some(ShellRole::Lock);
        let is_codex_project_chat = is_codex_project_chat(projected_app_id);
        let is_utility = matches!(
            shell_role,
            Some(
                ShellRole::ControlCenter
                    | ShellRole::Notification
                    | ShellRole::VolumeOsd
                    | ShellRole::ProjectMenu
                    | ShellRole::Screenshot
                    | ShellRole::Recovery
            )
        );
        if let Some(id) = registry_id {
            self.clear_changed_shell_surface_role(&surface.wl_surface().id(), shell_role);
            if let Some(role) =
                unauthenticated_reserved_shell_role(self.windows.app_id(id), authenticated)
            {
                self.workspaces.remove_window(&id);
                self.shell_owned_windows.remove(&id);
                let window = self
                    .space
                    .elements()
                    .find(|window| window.wl_surface().as_deref() == Some(surface.wl_surface()))
                    .cloned();
                if let Some(window) = window {
                    self.space.unmap_elem(&window);
                }
                tracing::error!(
                    ?client_pid,
                    ?role,
                    "rejected reserved Nickel shell role from unauthenticated client"
                );
                surface.send_close();
                let surface_id = surface.wl_surface().id();
                let restore_focus = self.windows.is_active(id);
                self.retire_surface_window_references(Some(&surface_id), Some(id));
                self.restore_focus_after_window_removal(restore_focus);
                self.notify_protocol_snapshot();
                return;
            }
            if shell_role.is_some() {
                self.workspaces.remove_window(&id);
                // A hidden shell role may recreate its wl_surface when shown.
                // Registration happens before its trusted app ID arrives and
                // temporarily marks that new registry entry active. Preserve
                // the application that still owns keyboard focus without
                // perturbing its compositor stacking order.
                if self.windows.is_active(id)
                    && let Some(focused) = self
                        .seat
                        .get_keyboard()
                        .and_then(|keyboard| keyboard.current_focus())
                        .and_then(|focus| match focus {
                            KeyboardFocusTarget::Wayland(surface) => {
                                self.surface_windows.get(&surface.id()).copied()
                            }
                            KeyboardFocusTarget::X11(surface) => {
                                self.x11_windows.get(&surface.window_id()).copied()
                            }
                        })
                        .filter(|focused| *focused != id)
                {
                    self.windows.set_active(focused);
                }
            } else if self
                .windows
                .app_id(id)
                .is_some_and(|app_id| shell_owned_window_is_application(app_id, shell_role))
                && self.shell_owned_windows.remove(&id)
            {
                self.workspaces.add_window(id);
            }
        }
        if let Some(role) = shell_role {
            let window = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(window) = window {
                self.record_shell_role_registration(&window, role);
            }
        }
        if !self
            .mapped_xdg_toplevels
            .contains(&surface.wl_surface().id())
        {
            return;
        }
        self.notify_protocol_snapshot();
        if is_launcher {
            let launcher = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(launcher) = launcher {
                self.register_launcher(launcher);
            }
        }
        if is_desktop {
            let desktop = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(desktop) = desktop {
                self.register_desktop(desktop);
            }
        }
        if is_panel {
            let panel = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(panel) = panel {
                self.register_panel(panel);
            }
        }
        if is_context_menu {
            let menu = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(menu) = menu {
                self.register_context_menu(menu);
            }
        }
        if is_preview {
            let preview = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(preview) = preview {
                self.register_preview(preview);
            }
        }
        if is_utility {
            let utility = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(utility) = utility {
                if is_notification {
                    utility.override_z_index(45);
                }
                self.register_utility_window(utility, shell_role.expect("utility has shell role"));
            }
        }
        if is_lock {
            let lock = self.xdg_toplevel_window(surface.wl_surface());
            if let Some(lock) = lock {
                self.register_lock(lock);
            }
        }
        if let Some(role) = shell_role
            && self.pending_shell_focus_role == Some(role)
        {
            self.focus_shell_role(role);
        }
        // The shell and its dynamic Codex windows share one Wayland
        // client. New toplevels from that client are deliberately not focused
        // until their role is known, so shell chrome cannot steal keyboard
        // focus while starting. Once metadata identifies an ordinary Codex
        // project window, complete the deferred focus handoff.
        if is_codex_project_chat {
            if let Some(id) = self
                .surface_windows
                .get(&surface.wl_surface().id())
                .copied()
            {
                self.windows.raise(id);
                self.workspaces.focused(&id);
            }
            self.space.elements().for_each(|candidate| {
                candidate
                    .set_activated(candidate.wl_surface().as_deref() == Some(surface.wl_surface()));
            });
            self.seat.get_keyboard().unwrap().set_focus(
                self,
                Some(KeyboardFocusTarget::Wayland(surface.wl_surface().clone())),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
            self.space.elements().for_each(|window| {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_pending_configure();
                }
            });
        }
    }

    pub(crate) fn reconstrain_reactive_popups(&self, root: &WlSurface) {
        let popups = PopupManager::popups_for_surface(root)
            .filter_map(|(popup, _)| match popup {
                PopupKind::Xdg(popup)
                    if popup.with_pending_state(|state| state.positioner.reactive) =>
                {
                    Some(popup)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for popup in popups {
            if self.unconstrain_popup(&popup)
                && let Err(error) = popup.send_pending_configure()
            {
                tracing::debug!(?error, "could not reconfigure reactive xdg popup");
            }
        }
    }

    pub(crate) fn reconstrain_all_reactive_popups(&self) {
        let roots = self
            .space
            .elements()
            .filter_map(|window| {
                window
                    .toplevel()
                    .map(|surface| surface.wl_surface().clone())
            })
            .collect::<Vec<_>>();
        for root in roots {
            self.reconstrain_reactive_popups(&root);
        }
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) -> bool {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            popup.send_popup_done();
            return false;
        };
        let Some(window) = self.space.elements().find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == &root)
        }) else {
            popup.send_popup_done();
            return false;
        };

        let Some(window_geo) = self.space.element_geometry(window) else {
            popup.send_popup_done();
            return false;
        };
        let popup_from_toplevel = get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        let anchor = popup.with_pending_state(|state| {
            window_geo.loc + popup_from_toplevel + state.positioner.get_anchor_point()
        });
        let Some(output_geo) = popup_output_for_anchor(
            self.space
                .outputs()
                .filter_map(|output| self.space.output_geometry(output)),
            anchor,
        ) else {
            tracing::debug!(
                popup = ?popup.wl_surface().id(),
                parent = ?root.id(),
                ?anchor,
                "dismissing xdg popup whose parent anchor is outside enabled outputs"
            );
            popup.send_popup_done();
            return false;
        };
        let output_work_area = popup_constraint_area(output_geo);

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_work_area;
        target.loc -= popup_from_toplevel;
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
            tracing::debug!(
                popup = ?popup.wl_surface().id(),
                parent = ?root.id(),
                ?anchor,
                selected_output = ?output_geo,
                requested_geometry = ?state.positioner.get_geometry(),
                final_geometry = ?state.geometry,
                "positioned xdg popup from its parent-local anchor"
            );
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        admit_xdg_toplevel, is_codex_project_chat, new_toplevel_may_focus,
        parent_relative_dialog_location, popup_constraint_area, popup_output_for_anchor,
        shell_owned_window_is_application, unauthenticated_reserved_shell_role,
    };
    use nickel_session_protocol::ShellRole;
    use smithay::utils::Rectangle;

    #[test]
    fn popup_anchor_selects_its_parent_output_with_negative_and_reordered_origins() {
        let left = Rectangle::new((-1920, 0).into(), (1920, 1080).into());
        let right = Rectangle::new((0, 0).into(), (2560, 1440).into());

        assert_eq!(
            popup_output_for_anchor([right, left], (-220, 400).into()),
            Some(left)
        );
        assert_eq!(
            popup_output_for_anchor([left, right], (2100, 700).into()),
            Some(right)
        );
    }

    #[test]
    fn popup_anchor_on_boundary_has_a_stable_half_open_owner() {
        let left = Rectangle::new((-1920, 0).into(), (1920, 1080).into());
        let right = Rectangle::new((0, 0).into(), (2560, 1440).into());

        assert_eq!(
            popup_output_for_anchor([right, left], (0, 500).into()),
            Some(right)
        );
    }

    #[test]
    fn popup_anchor_outside_enabled_outputs_is_rejected() {
        let output = Rectangle::new((100, 200).into(), (800, 600).into());
        assert_eq!(popup_output_for_anchor([output], (99, 200).into()), None);
    }

    #[test]
    fn popup_constraints_reserve_the_panel_on_the_selected_output() {
        let output = Rectangle::new((-1600, 240).into(), (1600, 900).into());
        assert_eq!(
            popup_constraint_area(output),
            Rectangle::new((-1600, 240).into(), (1600, 844).into())
        );
    }

    #[test]
    fn ordinary_exhaustion_preserves_critical_authenticated_shell_admission() {
        let mut windows = crate::window_registry::WindowRegistry::default();
        let reserved = crate::window_registry::RESERVED_AUTHENTICATED_SHELL_WINDOWS;
        let ordinary_capacity = nickel_session_protocol::MAX_WINDOWS - reserved;

        for _ in 0..ordinary_capacity {
            assert!(admit_xdg_toplevel(&mut windows, false).is_some());
        }
        assert_eq!(admit_xdg_toplevel(&mut windows, false), None);
        for _ in 0..reserved {
            assert!(admit_xdg_toplevel(&mut windows, true).is_some());
        }
        assert_eq!(windows.len(), nickel_session_protocol::MAX_WINDOWS);
        assert_eq!(admit_xdg_toplevel(&mut windows, true), None);
    }

    #[test]
    fn codex_project_chat_uses_canonical_application_identity() {
        assert!(is_codex_project_chat(Some(
            "io.nickel.codex.project.bd247278c96614ec"
        )));
        assert!(!is_codex_project_chat(Some("Codex — sentrygist")));
        assert!(!is_codex_project_chat(Some("io.nickel.shell")));
    }

    #[test]
    fn background_toplevel_cannot_replace_application_focus() {
        assert!(new_toplevel_may_focus(None, false));
        assert!(new_toplevel_may_focus(Some(true), false));
        assert!(!new_toplevel_may_focus(Some(false), false));
        assert!(new_toplevel_may_focus(Some(false), true));
    }

    #[test]
    fn child_dialog_is_centered_on_parent_and_constrained_to_work_area() {
        let work_area = Rectangle::new((0, 0).into(), (1920, 1024).into());
        let parent = Rectangle::new((300, 100).into(), (1200, 800).into());
        assert_eq!(
            parent_relative_dialog_location(parent, (600, 400), work_area),
            (600, 300).into()
        );

        let edge_parent = Rectangle::new((-100, -50).into(), (500, 300).into());
        assert_eq!(
            parent_relative_dialog_location(edge_parent, (700, 500), work_area),
            (0, 0).into()
        );
    }

    #[test]
    fn shell_client_stays_private_until_a_non_shell_identity_is_known() {
        assert!(!shell_owned_window_is_application("", None));
        assert!(!shell_owned_window_is_application(
            ShellRole::Preview.application_id(),
            Some(ShellRole::Preview)
        ));
        assert!(!shell_owned_window_is_application(
            ShellRole::Screenshot.application_id(),
            Some(ShellRole::Screenshot)
        ));
        assert!(shell_owned_window_is_application(
            "io.nickel.codex.project.bd247278c96614ec",
            None
        ));
        assert!(!shell_owned_window_is_application(
            ShellRole::Panel.application_id(),
            None
        ));
    }

    #[test]
    fn reserved_shell_identity_requires_authenticated_peer() {
        assert_eq!(
            unauthenticated_reserved_shell_role(Some(ShellRole::Panel.application_id()), false),
            Some(ShellRole::Panel)
        );
        assert_eq!(
            unauthenticated_reserved_shell_role(Some(ShellRole::Panel.application_id()), true),
            None
        );
        assert_eq!(
            unauthenticated_reserved_shell_role(Some("org.example.Application"), false),
            None
        );
    }
}
