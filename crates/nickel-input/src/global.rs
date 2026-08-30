//! Contracts shared by compositor-owned and operating-system global shortcuts.
//!
//! Native registration and product actions remain outside `nickel-input`.

use std::collections::BTreeMap;

use crate::{EventOrder, KeyEdge, Shortcut};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutOwnership {
    FocusedApplication,
    Compositor,
    OperatingSystem,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShortcutCapability {
    Available,
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    UnsupportedPlatform,
    MissingRuntime,
    PermissionDenied,
    SessionLocked,
    Backend(String),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RegistrationId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    Unavailable(UnavailableReason),
    Conflict { existing: RegistrationId },
    Reserved,
    Backend(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration<A> {
    pub shortcut: Shortcut,
    pub action: A,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalShortcutEdge {
    Pressed,
    Released,
    Activated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalShortcutEvent<A> {
    pub registration: RegistrationId,
    pub action: A,
    pub edge: GlobalShortcutEdge,
    pub order: EventOrder,
}

/// The narrow behavior each native global-shortcut adapter exposes.
pub trait GlobalShortcutAdapter<A: Clone> {
    fn ownership(&self) -> ShortcutOwnership;
    fn capability(&self) -> ShortcutCapability;
    fn register(
        &mut self,
        registration: Registration<A>,
    ) -> Result<RegistrationId, RegistrationError>;
    fn unregister(&mut self, id: RegistrationId) -> bool;
    fn reset(&mut self);
}

/// Deterministic registration and edge-delivery state for native adapters.
///
/// Backends register with their native API first, then commit the successful
/// registration here. This table rejects duplicate normalized shortcuts and
/// suppresses repeated pressed or activation notifications.
#[derive(Clone, Debug)]
pub struct RegistrationTable<A> {
    next_id: u64,
    next_order: u64,
    by_id: BTreeMap<RegistrationId, Registration<A>>,
    pressed: BTreeMap<RegistrationId, bool>,
}

impl<A> Default for RegistrationTable<A> {
    fn default() -> Self {
        Self {
            next_id: 0,
            next_order: 0,
            by_id: BTreeMap::new(),
            pressed: BTreeMap::new(),
        }
    }
}

impl<A: Clone> RegistrationTable<A> {
    pub fn register(
        &mut self,
        registration: Registration<A>,
    ) -> Result<RegistrationId, RegistrationError> {
        if let Some((id, _)) = self
            .by_id
            .iter()
            .find(|(_, existing)| existing.shortcut == registration.shortcut)
        {
            return Err(RegistrationError::Conflict { existing: *id });
        }
        self.next_id = self.next_id.saturating_add(1);
        let id = RegistrationId(self.next_id);
        self.by_id.insert(id, registration);
        Ok(id)
    }

    pub fn unregister(&mut self, id: RegistrationId) -> bool {
        self.pressed.remove(&id);
        self.by_id.remove(&id).is_some()
    }

    pub fn deliver(
        &mut self,
        id: RegistrationId,
        edge: GlobalShortcutEdge,
    ) -> Option<GlobalShortcutEvent<A>> {
        let registration = self.by_id.get(&id)?;
        let is_pressed = self.pressed.get(&id).copied().unwrap_or(false);
        match edge {
            GlobalShortcutEdge::Pressed if is_pressed => return None,
            GlobalShortcutEdge::Released if !is_pressed => return None,
            GlobalShortcutEdge::Pressed => {
                self.pressed.insert(id, true);
            }
            GlobalShortcutEdge::Released => {
                self.pressed.remove(&id);
            }
            GlobalShortcutEdge::Activated if is_pressed => return None,
            GlobalShortcutEdge::Activated => {
                // An activation-only backend has no release edge. It therefore
                // remains immediately eligible for the next native activation.
            }
        }
        self.next_order = self.next_order.saturating_add(1);
        Some(GlobalShortcutEvent {
            registration: id,
            action: registration.action.clone(),
            edge,
            order: EventOrder(self.next_order),
        })
    }

    /// Clear held state after focus/session loss or a native adapter restart.
    /// Registrations remain valid only when the backend says they survived.
    pub fn reset_edges(&mut self) {
        self.pressed.clear();
    }

    /// Drop native registrations during adapter shutdown or restart.
    pub fn clear(&mut self) {
        self.pressed.clear();
        self.by_id.clear();
    }

    pub fn registration(&self, id: RegistrationId) -> Option<&Registration<A>> {
        self.by_id.get(&id)
    }
}

impl From<KeyEdge> for GlobalShortcutEdge {
    fn from(edge: KeyEdge) -> Self {
        match edge {
            KeyEdge::Pressed => Self::Pressed,
            KeyEdge::Released => Self::Released,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::{KeyCode, PhysicalKey, ShortcutKey, ShortcutTrigger};

    use super::*;

    fn registration(action: u8) -> Registration<u8> {
        Registration {
            shortcut: Shortcut {
                key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyR)),
                modifiers: BTreeSet::new(),
                trigger: ShortcutTrigger::Pressed,
            },
            action,
        }
    }

    #[test]
    fn conflicts_are_explicit_and_unregister_releases_the_shortcut() {
        let mut table = RegistrationTable::default();
        let id = table.register(registration(1)).unwrap();
        assert_eq!(
            table.register(registration(2)),
            Err(RegistrationError::Conflict { existing: id })
        );
        assert!(table.unregister(id));
        assert!(table.register(registration(2)).is_ok());
    }

    #[test]
    fn edges_are_ordered_and_repeated_pressed_edges_are_suppressed() {
        let mut table = RegistrationTable::default();
        let id = table.register(registration(7)).unwrap();
        let pressed = table.deliver(id, GlobalShortcutEdge::Pressed).unwrap();
        assert_eq!(pressed.order, EventOrder(1));
        assert!(table.deliver(id, GlobalShortcutEdge::Pressed).is_none());
        let released = table.deliver(id, GlobalShortcutEdge::Released).unwrap();
        assert_eq!(released.order, EventOrder(2));
        assert!(table.deliver(id, GlobalShortcutEdge::Released).is_none());
    }

    #[test]
    fn reset_and_restart_never_manufacture_actions() {
        let mut table = RegistrationTable::default();
        let id = table.register(registration(3)).unwrap();
        assert!(table.deliver(id, GlobalShortcutEdge::Pressed).is_some());
        table.reset_edges();
        assert!(table.deliver(id, GlobalShortcutEdge::Released).is_none());
        assert!(table.deliver(id, GlobalShortcutEdge::Pressed).is_some());
        table.clear();
        assert!(table.deliver(id, GlobalShortcutEdge::Released).is_none());
    }

    #[test]
    fn activation_only_backends_can_deliver_distinct_native_activations() {
        let mut table = RegistrationTable::default();
        let id = table.register(registration(9)).unwrap();
        assert!(table.deliver(id, GlobalShortcutEdge::Activated).is_some());
        assert!(table.deliver(id, GlobalShortcutEdge::Activated).is_some());
    }
}
