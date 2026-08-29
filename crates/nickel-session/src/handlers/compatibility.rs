use smithay::{
    desktop::{PopupKind, PopupManager},
    input::pointer::PointerHandle,
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Rectangle},
    wayland::{
        idle_inhibit::IdleInhibitHandler,
        input_method::{InputMethodHandler, PopupSurface},
        pointer_constraints::{
            PointerConstraint, PointerConstraintsHandler, with_pointer_constraint,
        },
        seat::WaylandFocus,
        selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
    },
};

use crate::NickelSession;

const MAX_IDLE_INHIBITED_SURFACES: usize = 256;

fn idle_inhibitor_allowed(distinct_surfaces: usize, already_tracked: bool) -> bool {
    already_tracked || distinct_surfaces < MAX_IDLE_INHIBITED_SURFACES
}

impl NickelSession {
    pub(crate) fn constrained_pointer_position(
        &self,
        surface: &WlSurface,
        surface_origin: smithay::utils::Point<f64, Logical>,
        current: smithay::utils::Point<f64, Logical>,
        proposed: smithay::utils::Point<f64, Logical>,
    ) -> smithay::utils::Point<f64, Logical> {
        let Some(pointer) = self.seat.get_pointer() else {
            return proposed;
        };
        let proposed_hits_surface = self
            .surface_under(proposed)
            .is_some_and(|(candidate, _)| candidate == *surface);
        with_pointer_constraint(surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return proposed;
            };
            if !constraint.is_active() {
                constraint.activate();
            }
            match &*constraint {
                PointerConstraint::Locked(_) => current,
                PointerConstraint::Confined(confined) => {
                    let local = proposed - surface_origin;
                    let inside = confined.region().map_or(proposed_hits_surface, |region| {
                        region.contains((local.x.floor() as i32, local.y.floor() as i32))
                    });
                    if inside { proposed } else { current }
                }
            }
        })
    }
}

impl PrimarySelectionHandler for NickelSession {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl PointerConstraintsHandler for NickelSession {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer
            .current_focus()
            .as_ref()
            .and_then(WaylandFocus::wl_surface)
            .as_deref()
            == Some(surface)
        {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    constraint.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: smithay::utils::Point<f64, Logical>,
    ) {
        // The hint is applied when a lock is released; Nickel does not warp a
        // locked pointer while the constraint remains active.
    }
}

impl IdleInhibitHandler for NickelSession {
    fn inhibit(&mut self, surface: WlSurface) {
        let surface = surface.id();
        if let Some(count) = self.idle_inhibitors.get_mut(&surface) {
            *count = count.saturating_add(1);
        } else if idle_inhibitor_allowed(self.idle_inhibitors.len(), false) {
            self.idle_inhibitors.insert(surface, 1);
        } else {
            tracing::warn!(
                limit = MAX_IDLE_INHIBITED_SURFACES,
                "idle-inhibited surface limit reached"
            );
        }
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        let surface = surface.id();
        let remove = self.idle_inhibitors.get_mut(&surface).is_some_and(|count| {
            *count = count.saturating_sub(1);
            *count == 0
        });
        if remove {
            self.idle_inhibitors.remove(&surface);
        }
    }
}

impl InputMethodHandler for NickelSession {
    fn new_popup(&mut self, surface: PopupSurface) {
        let _ = self.popups.track_popup(PopupKind::InputMethod(surface));
        self.request_output_redraw();
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        let root = surface.wl_surface().clone();
        let _ = PopupManager::dismiss_popup(&root, &PopupKind::InputMethod(surface));
        self.request_output_redraw();
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {
        self.request_output_redraw();
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.space
            .elements()
            .find(|window| {
                window
                    .toplevel()
                    .is_some_and(|surface| surface.wl_surface() == parent)
            })
            .and_then(|window| self.space.element_geometry(window))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_IDLE_INHIBITED_SURFACES, idle_inhibitor_allowed};

    #[test]
    fn idle_inhibitor_limit_still_permits_existing_surface_references() {
        assert!(idle_inhibitor_allowed(
            MAX_IDLE_INHIBITED_SURFACES - 1,
            false
        ));
        assert!(!idle_inhibitor_allowed(MAX_IDLE_INHIBITED_SURFACES, false));
        assert!(idle_inhibitor_allowed(MAX_IDLE_INHIBITED_SURFACES, true));
    }
}
