use smithay::{
    backend::renderer::utils::RendererSurfaceStateUserData,
    desktop::{PopupKind, PopupManager},
    input::pointer::PointerHandle,
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{RectangleKind, RegionAttributes, with_states},
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
const MAX_TRACKED_POINTER_LOCKS: usize = 256;
fn idle_inhibitor_allowed(distinct_surfaces: usize, already_tracked: bool) -> bool {
    already_tracked || distinct_surfaces < MAX_IDLE_INHIBITED_SURFACES
}

fn interpolate(
    start: Point<f64, Logical>,
    end: Point<f64, Logical>,
    progress: f64,
) -> Point<f64, Logical> {
    (
        start.x + (end.x - start.x) * progress,
        start.y + (end.y - start.y) * progress,
    )
        .into()
}

/// Return the last point on a motion segment that remains in the same connected
/// part of a protocol region as `start`.
///
/// Region membership is production-owned by Smithay. Splitting the segment at
/// every protocol rectangle edge and then refining the first transition handles
/// subtractive and disconnected regions without treating a later region island
/// as continuous confinement.
fn confine_motion_to_region(
    region: &RegionAttributes,
    start: Point<f64, Logical>,
    end: Point<f64, Logical>,
) -> Point<f64, Logical> {
    if !region.contains((start.x.floor() as i32, start.y.floor() as i32)) {
        return start;
    }
    let delta_x = end.x - start.x;
    let delta_y = end.y - start.y;
    let mut transitions = vec![0.0, 1.0];
    for (_, rectangle) in &region.rects {
        for edge in [rectangle.loc.x, rectangle.loc.x + rectangle.size.w] {
            if delta_x != 0.0 {
                let progress = (f64::from(edge) - start.x) / delta_x;
                if progress > 0.0 && progress < 1.0 {
                    transitions.push(progress);
                }
            }
        }
        for edge in [rectangle.loc.y, rectangle.loc.y + rectangle.size.h] {
            if delta_y != 0.0 {
                let progress = (f64::from(edge) - start.y) / delta_y;
                if progress > 0.0 && progress < 1.0 {
                    transitions.push(progress);
                }
            }
        }
    }
    transitions.sort_by(f64::total_cmp);
    transitions.dedup_by(|left, right| left.total_cmp(right).is_eq());

    let mut inside_progress = 0.0;
    let mut outside_progress = None;
    for interval in transitions.windows(2) {
        let progress = (interval[0] + interval[1]) / 2.0;
        let point = interpolate(start, end, progress);
        if !region.contains((point.x.floor() as i32, point.y.floor() as i32)) {
            outside_progress = Some(progress);
            break;
        }
        inside_progress = progress;
    }
    let Some(mut outside_progress) = outside_progress else {
        return end;
    };
    for _ in 0..32 {
        let progress = (inside_progress + outside_progress) / 2.0;
        let point = interpolate(start, end, progress);
        if region.contains((point.x.floor() as i32, point.y.floor() as i32)) {
            inside_progress = progress;
        } else {
            outside_progress = progress;
        }
    }
    interpolate(start, end, inside_progress)
}

fn surface_extent_region(surface: &WlSurface) -> Option<RegionAttributes> {
    with_states(surface, |states| {
        let renderer_state = states
            .data_map
            .get::<RendererSurfaceStateUserData>()?
            .lock()
            .ok()?;
        let size = renderer_state.surface_size()?;
        Some(RegionAttributes {
            rects: vec![(RectangleKind::Add, Rectangle::new((0, 0).into(), size))],
        })
    })
}

impl NickelSession {
    fn remember_active_pointer_lock(&mut self, surface: &WlSurface) {
        let id = surface.id();
        if self.active_pointer_locks.contains(&id)
            || self.active_pointer_locks.len() < MAX_TRACKED_POINTER_LOCKS
        {
            self.active_pointer_locks.insert(id);
        } else {
            tracing::warn!(
                limit = MAX_TRACKED_POINTER_LOCKS,
                "active pointer-lock tracking limit reached"
            );
        }
    }

    pub(crate) fn constrained_pointer_position(
        &mut self,
        surface: &WlSurface,
        surface_origin: smithay::utils::Point<f64, Logical>,
        current: smithay::utils::Point<f64, Logical>,
        proposed: smithay::utils::Point<f64, Logical>,
    ) -> (smithay::utils::Point<f64, Logical>, bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return (proposed, false);
        };
        let has_keyboard_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()))
            .as_ref()
            == Some(surface);
        if !has_keyboard_focus {
            with_pointer_constraint(surface, &pointer, |constraint| {
                if let Some(constraint) = constraint
                    && constraint.is_active()
                {
                    constraint.deactivate();
                }
            });
            self.active_pointer_constraint_origins.remove(&surface.id());
            return (proposed, false);
        }
        let proposed_hits_surface = self
            .surface_under(proposed)
            .is_some_and(|(candidate, _)| candidate == *surface);
        let surface_region = surface_extent_region(surface);
        let mut activated_lock = false;
        let (position, active) = with_pointer_constraint(surface, &pointer, |constraint| {
            let Some(constraint) = constraint else {
                return (proposed, false);
            };
            let mut just_activated = false;
            if !constraint.is_active() {
                let local = proposed - surface_origin;
                let enters_constraint = constraint
                    .region()
                    .or(surface_region.as_ref())
                    .map_or(proposed_hits_surface, |region| {
                        region.contains((local.x.floor() as i32, local.y.floor() as i32))
                    });
                if !enters_constraint {
                    return (proposed, false);
                }
                constraint.activate();
                just_activated = true;
                activated_lock = matches!(&*constraint, PointerConstraint::Locked(_));
            }
            let position = match &*constraint {
                PointerConstraint::Locked(_) if just_activated => proposed,
                PointerConstraint::Locked(_) => current,
                PointerConstraint::Confined(confined) => {
                    let current_local = current - surface_origin;
                    let local = proposed - surface_origin;
                    confined.region().or(surface_region.as_ref()).map_or_else(
                        || {
                            if proposed_hits_surface {
                                proposed
                            } else {
                                current
                            }
                        },
                        |region| {
                            confine_motion_to_region(region, current_local, local) + surface_origin
                        },
                    )
                }
            };
            (position, constraint.is_active())
        });
        if activated_lock {
            self.remember_active_pointer_lock(surface);
        }
        let id = surface.id();
        if active {
            self.active_pointer_constraint_origins
                .insert(id, surface_origin);
        } else {
            self.active_pointer_constraint_origins.remove(&id);
        }
        (position, active)
    }

    /// Apply a client's last committed lock hint once that lock has gone away.
    /// The returned point becomes the origin for the next physical delta, so
    /// unlock does not create an artificial relative-motion jump.
    pub(crate) fn restore_released_pointer_lock_hint(
        &mut self,
        current: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let Some((focus, origin)) = self.pointer_surface_under(current) else {
            return current;
        };
        let Some(surface) = focus.wl_surface() else {
            return current;
        };
        let id = surface.id();
        if !self.active_pointer_locks.contains(&id) {
            return current;
        }
        let still_locked = self.seat.get_pointer().is_some_and(|pointer| {
            with_pointer_constraint(&surface, &pointer, |constraint| {
                constraint.is_some_and(|constraint| {
                    constraint.is_active() && matches!(&*constraint, PointerConstraint::Locked(_))
                })
            })
        });
        if still_locked {
            return current;
        }
        self.active_pointer_locks.remove(&id);
        self.pointer_lock_hints
            .remove(&id)
            .map_or(current, |hint| origin + hint)
    }
}

impl PrimarySelectionHandler for NickelSession {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl PointerConstraintsHandler for NickelSession {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        let has_pointer_focus = pointer
            .current_focus()
            .as_ref()
            .and_then(WaylandFocus::wl_surface)
            .as_deref()
            == Some(surface);
        let has_keyboard_focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()))
            .as_ref()
            == Some(surface);
        if has_pointer_focus && has_keyboard_focus {
            let current = pointer.current_location();
            // A replacement constraint can arrive while another surface
            // geometrically overlaps the constrained one. Preserve the
            // protocol focus origin instead of deriving authority again from
            // current stacking order.
            let origin = self
                .active_pointer_constraint_origins
                .get(&surface.id())
                .copied()
                .or_else(|| {
                    self.pointer_surface_under(current)
                        .filter(|(focus, _)| focus.wl_surface().as_deref() == Some(surface))
                        .map(|(_, origin)| origin)
                });
            let mut activated_lock = false;
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(constraint) = constraint {
                    let local = origin.map(|origin| current - origin);
                    let inside = constraint.region().is_none_or(|region| {
                        local.is_some_and(|point| {
                            region.contains((point.x.floor() as i32, point.y.floor() as i32))
                        })
                    });
                    if inside {
                        constraint.activate();
                        activated_lock = matches!(&*constraint, PointerConstraint::Locked(_));
                    }
                }
            });
            if activated_lock {
                self.remember_active_pointer_lock(surface);
            }
        }
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        location: smithay::utils::Point<f64, Logical>,
    ) {
        // The protocol commits hints while the lock is active. Keep only the
        // latest hint and consume it when the lock is released.
        let id = surface.id();
        if self.pointer_lock_hints.contains_key(&id)
            || self.pointer_lock_hints.len() < MAX_TRACKED_POINTER_LOCKS
        {
            self.pointer_lock_hints.insert(id, location);
        } else {
            tracing::warn!(
                limit = MAX_TRACKED_POINTER_LOCKS,
                "pointer-lock hint limit reached"
            );
        }
    }
}

impl IdleInhibitHandler for NickelSession {
    fn inhibit(&mut self, surface: WlSurface) {
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
    use smithay::{
        utils::Rectangle,
        wayland::compositor::{RectangleKind, RegionAttributes},
    };

    use super::{MAX_IDLE_INHIBITED_SURFACES, confine_motion_to_region, idle_inhibitor_allowed};

    fn region(
        rectangles: &[(RectangleKind, Rectangle<i32, smithay::utils::Logical>)],
    ) -> RegionAttributes {
        RegionAttributes {
            rects: rectangles.to_vec(),
        }
    }

    #[test]
    fn idle_inhibitor_limit_still_permits_existing_surface_references() {
        assert!(idle_inhibitor_allowed(
            MAX_IDLE_INHIBITED_SURFACES - 1,
            false
        ));
        assert!(!idle_inhibitor_allowed(MAX_IDLE_INHIBITED_SURFACES, false));
        assert!(idle_inhibitor_allowed(MAX_IDLE_INHIBITED_SURFACES, true));
    }

    #[test]
    fn confinement_preserves_motion_inside_the_authoritative_region() {
        let allowed = region(&[(
            RectangleKind::Add,
            Rectangle::new((10, 20).into(), (100, 80).into()),
        )]);
        let end = (80.0, 70.0).into();

        assert_eq!(
            confine_motion_to_region(&allowed, (20.0, 30.0).into(), end),
            end
        );
    }

    #[test]
    fn confinement_stops_at_the_first_authoritative_region_boundary() {
        let allowed = region(&[
            (
                RectangleKind::Add,
                Rectangle::new((0, 0).into(), (100, 100).into()),
            ),
            (
                RectangleKind::Subtract,
                Rectangle::new((40, 0).into(), (20, 100).into()),
            ),
        ]);

        let result = confine_motion_to_region(&allowed, (20.0, 50.0).into(), (80.0, 50.0).into());

        assert!(allowed.contains((result.x.floor() as i32, result.y.floor() as i32)));
        assert!(
            result.x < 40.0,
            "pointer crossed the subtracted region: {result:?}"
        );
        assert!(result.x > 20.0, "valid motion was discarded: {result:?}");
    }

    #[test]
    fn confinement_cannot_skip_a_narrow_subtracted_region() {
        let allowed = region(&[
            (
                RectangleKind::Add,
                Rectangle::new((0, 0).into(), (10_000, 100).into()),
            ),
            (
                RectangleKind::Subtract,
                Rectangle::new((5_000, 0).into(), (1, 100).into()),
            ),
        ]);

        let result =
            confine_motion_to_region(&allowed, (10.0, 50.0).into(), (9_990.0, 50.0).into());

        assert!(allowed.contains((result.x.floor() as i32, result.y.floor() as i32)));
        assert!(result.x < 5_000.0);
    }
}
