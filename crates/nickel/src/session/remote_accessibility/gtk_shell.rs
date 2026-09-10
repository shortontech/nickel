//! GTK's authenticated, per-wl_surface accessibility association (gtk-shell v7).
use super::super::NickelSession;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    protocol::wl_surface::WlSurface,
};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    unused_unsafe,
    clippy::all
)]
mod wire {
    use wayland_server;
    use wayland_server::protocol::*;
    mod __interfaces {
        use wayland_server::backend as wayland_backend;
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/gtk-shell.xml");
    }
    use __interfaces::*;
    wayland_scanner::generate_server_code!("protocols/gtk-shell.xml");
}
use wire::{
    gtk_shell1::{self, GtkShell1},
    gtk_surface1::{self, GtkSurface1},
};

/// Private descriptor; no bus names or object paths are exposed to remote clients.
#[derive(Clone)]
pub(in crate::session) struct Association {
    pub generation: u64,
    pub peer: String,
    pub root: String,
    pub current: Arc<AtomicBool>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.invalidate();
    }
}
#[derive(Default)]
struct Slot(Mutex<Option<Association>>);
impl Slot {
    fn invalidate(&self) {
        if let Some(previous) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            previous.current.store(false, Ordering::Release);
        }
    }
    fn replace(&self, peer: String, root: String) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(previous) = slot.take() {
            previous.current.store(false, Ordering::Release);
        }
        if valid_association(&peer, &root) {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let Ok(generation) =
                NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            else {
                return;
            };
            *slot = Some(Association {
                generation,
                peer,
                root,
                current: Arc::new(AtomicBool::new(true)),
            });
        }
    }
}
fn valid_association(peer: &str, root: &str) -> bool {
    peer.len() <= 255
        && root.len() <= 512
        && zbus::names::UniqueName::try_from(peer).is_ok()
        && zbus::zvariant::ObjectPath::try_from(root).is_ok()
        && root != "/org/a11y/atspi/null"
        && root != "/org/a11y/atspi/accessible/root"
        && root != "/"
}

pub(in crate::session) fn association(surface: &WlSurface) -> Option<Association> {
    if !surface.is_alive() {
        return None;
    }
    smithay::wayland::compositor::with_states(surface, |state| {
        state.data_map.get::<Slot>()?.0.lock().ok()?.clone()
    })
}

#[derive(Clone)]
pub(in crate::session) enum PressOrigin {
    Local,
    Remote(
        nickel_remote_control::DesktopPermit,
        crate::session::window_registry::WindowId,
        u64,
    ),
    Denied,
}

/// A single recent *delivered* press proves click actions even when the client
/// request arrives after release. It is not a drag grab, telemetry, or input log.
#[derive(Default)]
struct RecentPress(Mutex<Option<(u32, u32, std::time::Instant, PressOrigin)>>);
impl RecentPress {
    fn consume(&self, serial: u32, button: u32, now: std::time::Instant) -> Option<PressOrigin> {
        let mut press = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if press
            .as_ref()
            .is_some_and(|(actual, actual_button, time, _)| {
                *actual == serial
                    && *actual_button == button
                    && now.saturating_duration_since(*time) <= std::time::Duration::from_secs(1)
            })
        {
            press.take().map(|(_, _, _, origin)| origin)
        } else {
            None
        }
    }
}
pub(in crate::session) fn record_pointer_press(
    surface: &WlSurface,
    event: &smithay::input::pointer::ButtonEvent,
    origin: PressOrigin,
) {
    if event.state != smithay::backend::input::ButtonState::Pressed {
        return;
    }
    smithay::wayland::compositor::with_states(surface, |state| {
        state.data_map.insert_if_missing(RecentPress::default);
        if let Some(press) = state.data_map.get::<RecentPress>() {
            *press.0.lock().unwrap_or_else(|error| error.into_inner()) = Some((
                event.serial.into(),
                event.button,
                std::time::Instant::now(),
                origin,
            ));
        }
    });
}

pub(in crate::session) fn install(display: &DisplayHandle) {
    display.create_global::<NickelSession, GtkShell1, _>(7, ());
}

impl GlobalDispatch<GtkShell1, ()> for NickelSession {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<GtkShell1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let shell = init.init(resource, ());
        // No global menu/desktop icon protocol is advertised.
        shell.capabilities(0);
    }
}
impl Dispatch<GtkShell1, ()> for NickelSession {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &GtkShell1,
        request: gtk_shell1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let gtk_shell1::Request::GetGtkSurface {
            gtk_surface,
            surface,
        } = request
        {
            if resource.client() != surface.client() {
                resource.post_error(0u32, "surface belongs to a different client");
                return;
            }
            smithay::wayland::compositor::with_states(&surface, |state| {
                state.data_map.insert_if_missing(Slot::default);
            });
            let gtk = init.init(gtk_surface, surface);
            gtk.configure(Vec::new());
            if gtk.version() >= 2 {
                gtk.configure_edges(
                    [1u32, 2, 3, 4]
                        .into_iter()
                        .flat_map(u32::to_ne_bytes)
                        .collect(),
                );
            }
        }
        // Startup IDs/bell/launch notifications confer no additional local or remote authority.
    }
}
impl Dispatch<GtkSurface1, WlSurface> for NickelSession {
    fn request(
        session: &mut Self,
        _: &Client,
        gtk: &GtkSurface1,
        request: gtk_surface1::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !surface.is_alive() {
            return;
        }
        match request {
            gtk_surface1::Request::SetA11yProperties {
                a11y_dbus_name,
                toplevel_object_path,
            } => {
                smithay::wayland::compositor::with_states(surface, |state| {
                    if let Some(slot) = state.data_map.get::<Slot>() {
                        slot.replace(a11y_dbus_name, toplevel_object_path);
                    }
                });
            }
            gtk_surface1::Request::Release => {
                smithay::wayland::compositor::with_states(surface, |state| {
                    if let Some(slot) = state.data_map.get::<Slot>() {
                        slot.invalidate();
                    }
                });
            }
            gtk_surface1::Request::TitlebarGesture {
                serial,
                seat,
                gesture,
            } => {
                use smithay::{input::Seat, reexports::wayland_server::WEnum};
                let Some(seat) = Seat::<NickelSession>::from_resource(&seat) else {
                    return;
                };
                use smithay::wayland::seat::WaylandFocus;
                let button = match gesture {
                    WEnum::Value(gtk_surface1::Gesture::DoubleClick) => 272,
                    WEnum::Value(gtk_surface1::Gesture::RightClick) => 273,
                    WEnum::Value(gtk_surface1::Gesture::MiddleClick) => 274,
                    _ => {
                        gtk.post_error(
                            gtk_surface1::Error::InvalidGesture,
                            "invalid GTK titlebar gesture",
                        );
                        return;
                    }
                };
                let focused = session
                    .seat
                    .get_keyboard()
                    .and_then(|keyboard| keyboard.current_focus())
                    .and_then(|focus| focus.wl_surface().map(std::borrow::Cow::into_owned));
                if session.locked || seat != session.seat || focused.as_ref() != Some(surface) {
                    return;
                }
                let current_press = smithay::wayland::compositor::with_states(surface, |state| {
                    state
                        .data_map
                        .get::<RecentPress>()
                        .and_then(|press| press.consume(serial, button, std::time::Instant::now()))
                });
                let Some(origin) = current_press else {
                    return;
                };
                let Some(id) = session.surface_windows.get(&surface.id()).copied() else {
                    return;
                };
                session.dispatch_gtk_titlebar_gesture(id, button, origin);
            }
            gtk_surface1::Request::SetModal | gtk_surface1::Request::UnsetModal => {
                if session.locked {
                    return;
                }
                use smithay::wayland::shell::xdg::dialog::{ToplevelDialogHint, XdgDialogHandler};
                if let Some(window) = session.xdg_toplevel_window(surface)
                    && let Some(toplevel) = window.toplevel()
                {
                    session.dialog_hint_changed(
                        toplevel.clone(),
                        if matches!(request, gtk_surface1::Request::SetModal) {
                            ToplevelDialogHint::Modal
                        } else {
                            ToplevelDialogHint::Dialog
                        },
                    );
                }
            }
            gtk_surface1::Request::RequestFocus {
                startup_id: Some(token),
            } => {
                use smithay::wayland::xdg_activation::{XdgActivationHandler, XdgActivationToken};
                let token = XdgActivationToken::from(token);
                if let Some(data) = session.activation_state.data_for_token(&token).cloned() {
                    session.request_activation(token, data, surface.clone());
                }
            }
            // Legacy timestamp-only presentation cannot establish Wayland input
            // authority. Modern GTK uses the advertised xdg_activation protocol.
            gtk_surface1::Request::Present { .. } => {}
            // Global-menu capabilities are absent. These advisory metadata fields
            // are not app identity, activation tokens, or remote control grants.
            _ => {}
        }
    }
    fn destroyed(
        _: &mut Self,
        _: wayland_server::backend::ClientId,
        _: &GtkSurface1,
        surface: &WlSurface,
    ) {
        if surface.is_alive() {
            smithay::wayland::compositor::with_states(surface, |state| {
                if let Some(slot) = state.data_map.get::<Slot>() {
                    slot.invalidate();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivered_click_authority_is_one_shot_button_specific_and_expires_after_release() {
        let now = std::time::Instant::now();
        let press = RecentPress(Mutex::new(Some((42, 272, now, PressOrigin::Local))));
        assert!(press.consume(43, 272, now).is_none());
        assert!(press.consume(42, 273, now).is_none());
        assert!(
            press
                .consume(42, 272, now + std::time::Duration::from_millis(80))
                .is_some()
        );
        assert!(press.consume(42, 272, now).is_none());
        *press.0.lock().unwrap() = Some((44, 273, now, PressOrigin::Local));
        assert!(
            press
                .consume(44, 273, now + std::time::Duration::from_secs(2))
                .is_none()
        );
    }
    #[test]
    fn association_replacement_and_release_retire_every_retained_proof() {
        let slot = Slot::default();
        slot.replace(":1.2".into(), "/org/gtk/window/1".into());
        let first = slot.0.lock().unwrap().clone().unwrap();
        slot.replace(":1.2".into(), "/org/gtk/window/2".into());
        assert!(!first.current.load(Ordering::Acquire));
        let second = slot.0.lock().unwrap().clone().unwrap();
        slot.invalidate();
        assert!(!second.current.load(Ordering::Acquire));
        for (peer, root) in [
            ("org.app.Name", "/root"),
            (":1.3", "/org/a11y/atspi/accessible/root"),
            (":1.3", "/org/a11y/atspi/null"),
        ] {
            slot.replace(peer.into(), root.into());
            assert!(slot.0.lock().unwrap().is_none());
        }
    }
}
