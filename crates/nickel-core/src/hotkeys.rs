use std::collections::{BTreeMap, BTreeSet};

use nickel_input::{
    AggregateModifier, Binding, DeviceId, EventOrder, InputEvent, KeyEvent, KeyLocation,
    LogicalKey, Modifier, ModifierState, NativeCode, NativeKey, PhysicalKey, Shortcut,
    ShortcutEngine, ShortcutKey, ShortcutTrigger,
    global::{
        GlobalShortcutAdapter, GlobalShortcutEdge, Registration, RegistrationError, RegistrationId,
        RegistrationTable, ShortcutCapability, ShortcutOwnership,
    },
};
pub use nickel_input::{KeyCode, KeyEdge};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HotkeyAction {
    LockSession,
    ToggleLauncher,
    ShowRun,
    SwitchNext,
    SwitchPrevious,
    SwitchGroupNext,
    SwitchGroupPrevious,
    CommitSwitch,
    CaptureActiveWindow,
    CaptureActiveWindowToFile,
    ShowScreenshotTool,
    SwitchWorkspacePrevious,
    SwitchWorkspaceNext,
    MoveWindowToPreviousWorkspace,
    MoveWindowToNextWorkspace,
}

#[derive(Debug)]
pub struct CompositorShortcutAdapter {
    engine: ShortcutEngine<RegistrationId>,
    next_order: u64,
    registrations: RegistrationTable<HotkeyAction>,
    actions: BTreeMap<HotkeyAction, RegistrationId>,
    bindings: BTreeMap<RegistrationId, Binding<RegistrationId>>,
    owned_keys: BTreeSet<KeyCode>,
    switch_active: bool,
    launcher_visible: bool,
}

impl Default for CompositorShortcutAdapter {
    fn default() -> Self {
        let mut adapter = Self {
            engine: ShortcutEngine::default(),
            next_order: 0,
            registrations: RegistrationTable::default(),
            actions: BTreeMap::new(),
            bindings: BTreeMap::new(),
            owned_keys: BTreeSet::new(),
            switch_active: false,
            launcher_visible: false,
        };
        for registration in compositor_registrations() {
            let action = registration.action;
            let id = adapter
                .register(registration)
                .expect("built-in compositor shortcuts are conflict-free");
            adapter.actions.entry(action).or_insert(id);
        }
        adapter
    }
}

impl CompositorShortcutAdapter {
    pub fn handle(&mut self, key: KeyCode, edge: KeyEdge) -> HotkeyOutcome {
        let suppress_owned = self.event_is_owned(key) || self.owned_keys.contains(&key);
        self.next_order = self.next_order.saturating_add(1);
        let event = InputEvent::Key(shared_key_event(
            key,
            edge,
            self.next_order,
            self.engine.modifiers(),
        ));
        let recognized = self.engine.handle(&event);
        let suppress = suppress_owned || recognized.iter().any(|outcome| outcome.suppress);
        let action = recognized.into_iter().find_map(|outcome| {
            let id = outcome.action;
            let candidate = self
                .registrations
                .deliver(id, GlobalShortcutEdge::Activated)
                .map(|event| event.action);
            if candidate == Some(HotkeyAction::CommitSwitch) && !self.switch_active {
                return None;
            }
            let delivered = candidate;
            if matches!(
                delivered,
                Some(
                    HotkeyAction::SwitchNext
                        | HotkeyAction::SwitchPrevious
                        | HotkeyAction::SwitchGroupNext
                        | HotkeyAction::SwitchGroupPrevious
                )
            ) {
                self.switch_active = true;
            } else if delivered == Some(HotkeyAction::CommitSwitch) {
                self.switch_active = false;
            }
            delivered
        });
        if key == KeyCode::KeyR
            && edge == KeyEdge::Released
            && self.engine.modifiers().aggregate(AggregateModifier::Super)
        {
            self.engine
                .reconcile_modifier(COMPOSITOR_KEYBOARD, Modifier::SuperLeft, false);
            self.engine
                .reconcile_modifier(COMPOSITOR_KEYBOARD, Modifier::SuperRight, false);
        }
        match edge {
            KeyEdge::Pressed if suppress => {
                self.owned_keys.insert(key);
            }
            KeyEdge::Released => {
                self.owned_keys.remove(&key);
            }
            KeyEdge::Pressed => {}
        }
        HotkeyOutcome { action, suppress }
    }

    pub fn handle_unmapped(&mut self, edge: KeyEdge) -> HotkeyOutcome {
        if edge == KeyEdge::Pressed {
            self.engine.chord_held_modifiers();
        }
        HotkeyOutcome::default()
    }

    pub fn handle_reconciled(&mut self, key: KeyCode, edge: KeyEdge) -> HotkeyOutcome {
        if key == KeyCode::PrintScreen
            && edge == KeyEdge::Released
            && !self.snapshot().print_screen_held
        {
            let pressed = self.handle(key, KeyEdge::Pressed);
            let _ = self.handle(key, KeyEdge::Released);
            return pressed;
        }
        self.handle(key, edge)
    }

    pub fn snapshot(&self) -> HotkeySnapshot {
        let held = |key| {
            self.engine
                .pressed_keys(COMPOSITOR_KEYBOARD)
                .any(|pressed| pressed == &PhysicalKey::Code(key))
        };
        HotkeySnapshot {
            super_held: self.engine.modifiers().aggregate(AggregateModifier::Super),
            alt_held: self.engine.modifiers().aggregate(AggregateModifier::Alt),
            shift_held: self.engine.modifiers().aggregate(AggregateModifier::Shift),
            control_held: self
                .engine
                .modifiers()
                .aggregate(AggregateModifier::Control),
            tab_held: held(KeyCode::Tab),
            grave_held: held(KeyCode::Backquote),
            run_held: held(KeyCode::KeyR),
            lock_held: held(KeyCode::KeyL),
            print_screen_held: held(KeyCode::PrintScreen),
            left_held: held(KeyCode::ArrowLeft),
            right_held: held(KeyCode::ArrowRight),
            switch_active: self.switch_active,
            launcher_visible: self.launcher_visible,
            ..HotkeySnapshot::default()
        }
    }

    pub fn begin_pointer_chord(&mut self) -> bool {
        if !self.engine.modifiers().aggregate(AggregateModifier::Super) {
            return false;
        }
        self.engine.chord_held_modifiers();
        true
    }

    pub fn reconcile_super(&mut self, physically_held: bool) {
        if !physically_held {
            self.engine
                .reconcile_modifier(COMPOSITOR_KEYBOARD, Modifier::SuperLeft, false);
            self.engine
                .reconcile_modifier(COMPOSITOR_KEYBOARD, Modifier::SuperRight, false);
        }
    }

    pub fn reconcile_alt(&mut self, physically_held: bool) -> Option<HotkeyAction> {
        if physically_held {
            return None;
        }
        let held = [KeyCode::AltLeft, KeyCode::AltRight]
            .into_iter()
            .filter(|key| {
                self.engine
                    .pressed_keys(COMPOSITOR_KEYBOARD)
                    .any(|pressed| pressed == &PhysicalKey::Code(*key))
            })
            .collect::<Vec<_>>();
        held.into_iter()
            .find_map(|key| self.handle(key, KeyEdge::Released).action)
    }

    pub fn launcher_visibility_applied(&mut self, visible: bool) {
        self.launcher_visible = visible;
    }

    pub fn reset_pressed_state(&mut self) {
        self.engine.reset();
        self.switch_active = false;
        self.owned_keys.clear();
        self.registrations.reset_edges();
    }

    /// Clears modifier/chord state at a secure focus boundary while retaining
    /// ownership of release edges for keys whose presses were intercepted.
    pub fn reset_chord_state_preserving_owned_releases(&mut self) {
        self.engine.reset();
        self.switch_active = false;
        self.registrations.reset_edges();
    }

    fn event_is_owned(&self, key: KeyCode) -> bool {
        let modifiers = self.engine.modifiers();
        match key {
            KeyCode::Tab | KeyCode::Backquote | KeyCode::PrintScreen => {
                modifiers.aggregate(AggregateModifier::Alt) || key == KeyCode::PrintScreen
            }
            KeyCode::KeyR => modifiers.aggregate(AggregateModifier::Super),
            KeyCode::KeyL => {
                modifiers.aggregate(AggregateModifier::Super)
                    || (modifiers.aggregate(AggregateModifier::Control)
                        && modifiers.aggregate(AggregateModifier::Alt))
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                modifiers.aggregate(AggregateModifier::Super)
                    && modifiers.aggregate(AggregateModifier::Control)
            }
            _ => false,
        }
    }
}

const COMPOSITOR_KEYBOARD: DeviceId = DeviceId(0x0043_4f4d_504b_4244);

fn shared_key_event(key: KeyCode, edge: KeyEdge, order: u64, modifiers: ModifierState) -> KeyEvent {
    KeyEvent {
        device: COMPOSITOR_KEYBOARD,
        order: EventOrder(order),
        physical: PhysicalKey::Code(key),
        logical: LogicalKey::Native(NativeKey {
            namespace: "nickel-compositor".into(),
            code: NativeCode::Numeric(key as u64),
        }),
        location: KeyLocation::Standard,
        edge,
        repeat: false,
        modifiers,
    }
}

impl GlobalShortcutAdapter<HotkeyAction> for CompositorShortcutAdapter {
    fn ownership(&self) -> ShortcutOwnership {
        ShortcutOwnership::Compositor
    }

    fn capability(&self) -> ShortcutCapability {
        ShortcutCapability::Available
    }

    fn register(
        &mut self,
        registration: Registration<HotkeyAction>,
    ) -> Result<RegistrationId, RegistrationError> {
        let shortcut = registration.shortcut.clone();
        let suppress = !matches!(
            registration.shortcut.trigger,
            ShortcutTrigger::ModifierReleased(_) | ShortcutTrigger::ModifierReleasedAfterChord(_)
        );
        let id = self.registrations.register(registration)?;
        let binding = Binding {
            suppress,
            shortcut,
            action: id,
        };
        self.bindings.insert(id, binding);
        self.engine.set_bindings(self.bindings.values().cloned());
        Ok(id)
    }

    fn unregister(&mut self, id: RegistrationId) -> bool {
        self.actions.retain(|_, registered| *registered != id);
        self.bindings.remove(&id);
        self.engine.set_bindings(self.bindings.values().cloned());
        self.registrations.unregister(id)
    }

    fn reset(&mut self) {
        self.engine.reset();
        self.switch_active = false;
        self.owned_keys.clear();
        self.registrations.reset_edges();
    }
}

fn compositor_registrations() -> Vec<Registration<HotkeyAction>> {
    use AggregateModifier::{Alt, Control, Shift, Super};
    use HotkeyAction::*;

    vec![
        registration(
            KeyCode::SuperLeft,
            [],
            ShortcutTrigger::ModifierReleased(nickel_input::Modifier::SuperLeft),
            ToggleLauncher,
        ),
        registration(
            KeyCode::SuperRight,
            [],
            ShortcutTrigger::ModifierReleased(nickel_input::Modifier::SuperRight),
            ToggleLauncher,
        ),
        registration(KeyCode::KeyR, [Super], ShortcutTrigger::Pressed, ShowRun),
        registration(
            KeyCode::KeyL,
            [Super],
            ShortcutTrigger::Pressed,
            LockSession,
        ),
        registration(
            KeyCode::KeyL,
            [Control, Alt],
            ShortcutTrigger::Pressed,
            LockSession,
        ),
        registration(KeyCode::Tab, [Alt], ShortcutTrigger::Pressed, SwitchNext),
        registration(
            KeyCode::Tab,
            [Alt, Shift],
            ShortcutTrigger::Pressed,
            SwitchPrevious,
        ),
        registration(
            KeyCode::Backquote,
            [Alt],
            ShortcutTrigger::Pressed,
            SwitchGroupNext,
        ),
        registration(
            KeyCode::Backquote,
            [Alt, Shift],
            ShortcutTrigger::Pressed,
            SwitchGroupPrevious,
        ),
        registration(
            KeyCode::AltLeft,
            [],
            ShortcutTrigger::ModifierReleasedAfterChord(nickel_input::Modifier::AltLeft),
            CommitSwitch,
        ),
        registration(
            KeyCode::AltRight,
            [],
            ShortcutTrigger::ModifierReleasedAfterChord(nickel_input::Modifier::AltRight),
            CommitSwitch,
        ),
        registration(
            KeyCode::AltLeft,
            [Shift],
            ShortcutTrigger::ModifierReleasedAfterChord(nickel_input::Modifier::AltLeft),
            CommitSwitch,
        ),
        registration(
            KeyCode::AltRight,
            [Shift],
            ShortcutTrigger::ModifierReleasedAfterChord(nickel_input::Modifier::AltRight),
            CommitSwitch,
        ),
        registration(
            KeyCode::ArrowLeft,
            [Control, Super],
            ShortcutTrigger::Pressed,
            SwitchWorkspacePrevious,
        ),
        registration(
            KeyCode::ArrowRight,
            [Control, Super],
            ShortcutTrigger::Pressed,
            SwitchWorkspaceNext,
        ),
        registration(
            KeyCode::ArrowLeft,
            [Control, Shift, Super],
            ShortcutTrigger::Pressed,
            MoveWindowToPreviousWorkspace,
        ),
        registration(
            KeyCode::ArrowRight,
            [Control, Shift, Super],
            ShortcutTrigger::Pressed,
            MoveWindowToNextWorkspace,
        ),
        registration(
            KeyCode::PrintScreen,
            [],
            ShortcutTrigger::Pressed,
            ShowScreenshotTool,
        ),
        registration(
            KeyCode::PrintScreen,
            [Alt],
            ShortcutTrigger::Pressed,
            CaptureActiveWindow,
        ),
        registration(
            KeyCode::PrintScreen,
            [Alt, Shift],
            ShortcutTrigger::Pressed,
            CaptureActiveWindowToFile,
        ),
    ]
}

/// Product bindings consumed by platform-neutral shortcut engines.
///
/// Native adapters own translation and lifecycle state; Nickel Core owns what
/// each shortcut means. Keeping this declaration here prevents platform
/// backends from growing parallel binding tables.
pub fn default_bindings() -> Vec<nickel_input::Binding<HotkeyAction>> {
    compositor_registrations()
        .into_iter()
        .map(|registration| nickel_input::Binding {
            suppress: !matches!(
                registration.shortcut.trigger,
                ShortcutTrigger::ModifierReleased(_)
                    | ShortcutTrigger::ModifierReleasedAfterChord(_)
            ),
            shortcut: registration.shortcut,
            action: registration.action,
        })
        .collect()
}

fn registration(
    key: KeyCode,
    modifiers: impl IntoIterator<Item = AggregateModifier>,
    trigger: ShortcutTrigger,
    action: HotkeyAction,
) -> Registration<HotkeyAction> {
    Registration {
        shortcut: Shortcut {
            key: ShortcutKey::Physical(PhysicalKey::Code(key)),
            modifiers: modifiers.into_iter().collect(),
            trigger,
        },
        action,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotkeyOutcome {
    pub action: Option<HotkeyAction>,
    pub suppress: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotkeySnapshot {
    pub super_held: bool,
    pub super_chorded: bool,
    pub alt_held: bool,
    pub shift_held: bool,
    pub control_held: bool,
    pub tab_held: bool,
    pub grave_held: bool,
    pub run_held: bool,
    pub lock_held: bool,
    pub print_screen_held: bool,
    pub left_held: bool,
    pub right_held: bool,
    pub switch_active: bool,
    pub launcher_visible: bool,
}
#[cfg(test)]
mod tests {
    use nickel_input::global::{
        GlobalShortcutAdapter, RegistrationError, ShortcutCapability, ShortcutOwnership,
    };
    use nickel_input::windows::WindowsInputAdapter;
    use nickel_input::{KeyCode, KeyEdge};

    use super::{
        CompositorShortcutAdapter, HotkeyAction, HotkeyOutcome, compositor_registrations,
        default_bindings,
    };

    #[test]
    fn compositor_adapter_registers_conflicts_and_reports_capability_honestly() {
        let mut adapter = CompositorShortcutAdapter::default();
        assert_eq!(adapter.ownership(), ShortcutOwnership::Compositor);
        assert_eq!(adapter.capability(), ShortcutCapability::Available);
        let duplicate = compositor_registrations()
            .into_iter()
            .find(|registration| registration.action == HotkeyAction::ShowRun)
            .unwrap();
        assert!(matches!(
            adapter.register(duplicate),
            Err(RegistrationError::Conflict { .. })
        ));
    }

    #[test]
    fn unregister_and_reset_are_deterministic_in_the_compositor_path() {
        let mut adapter = CompositorShortcutAdapter::default();
        let show_run = adapter.actions[&HotkeyAction::ShowRun];
        assert!(adapter.unregister(show_run));
        adapter.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert_eq!(adapter.handle(KeyCode::KeyR, KeyEdge::Pressed).action, None);
        adapter.reset();
        assert_eq!(
            adapter.handle(KeyCode::SuperLeft, KeyEdge::Released),
            HotkeyOutcome::default()
        );
    }

    #[test]
    fn windows_and_compositor_adapters_replay_the_same_product_actions() {
        let sequence = [
            (KeyCode::AltLeft, KeyEdge::Pressed),
            (KeyCode::Tab, KeyEdge::Pressed),
            (KeyCode::Tab, KeyEdge::Released),
            (KeyCode::AltLeft, KeyEdge::Released),
            (KeyCode::SuperLeft, KeyEdge::Pressed),
            (KeyCode::KeyL, KeyEdge::Pressed),
            (KeyCode::KeyL, KeyEdge::Released),
            (KeyCode::SuperLeft, KeyEdge::Released),
        ];
        let mut compositor = CompositorShortcutAdapter::default();
        let compositor_actions = sequence
            .iter()
            .filter_map(|(key, edge)| compositor.handle(*key, *edge).action)
            .collect::<Vec<_>>();
        let mut windows = WindowsInputAdapter::new(default_bindings());
        let windows_actions = sequence
            .iter()
            .flat_map(|(key, edge)| windows.handle_key_code(*key, *edge).outcomes)
            .map(|outcome| outcome.action)
            .collect::<Vec<_>>();
        assert_eq!(windows_actions, compositor_actions);
    }

    #[test]
    fn windows_and_compositor_share_reverse_switch_and_shift_held_commit_actions() {
        let sequence = [
            (KeyCode::ShiftRight, KeyEdge::Pressed),
            (KeyCode::AltRight, KeyEdge::Pressed),
            (KeyCode::Tab, KeyEdge::Pressed),
            (KeyCode::Tab, KeyEdge::Released),
            (KeyCode::AltRight, KeyEdge::Released),
            (KeyCode::ShiftRight, KeyEdge::Released),
        ];
        let mut compositor = CompositorShortcutAdapter::default();
        let compositor_actions = sequence
            .iter()
            .filter_map(|(key, edge)| compositor.handle(*key, *edge).action)
            .collect::<Vec<_>>();
        let mut windows = WindowsInputAdapter::new(default_bindings());
        let windows_actions = sequence
            .iter()
            .flat_map(|(key, edge)| windows.handle_key_code(*key, *edge).outcomes)
            .map(|outcome| outcome.action)
            .collect::<Vec<_>>();
        assert_eq!(windows_actions, compositor_actions);
        assert_eq!(
            compositor_actions,
            vec![HotkeyAction::SwitchPrevious, HotkeyAction::CommitSwitch]
        );
    }

    #[test]
    fn compositor_adapter_delivers_each_print_screen_binding() {
        let mut adapter = CompositorShortcutAdapter::default();
        assert_eq!(
            adapter
                .handle(KeyCode::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::ShowScreenshotTool)
        );
        adapter.handle(KeyCode::PrintScreen, KeyEdge::Released);
        adapter.handle(KeyCode::AltLeft, KeyEdge::Pressed);
        assert_eq!(
            adapter
                .handle(KeyCode::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::CaptureActiveWindow)
        );
        adapter.handle(KeyCode::PrintScreen, KeyEdge::Released);
        adapter.handle(KeyCode::ShiftLeft, KeyEdge::Pressed);
        assert_eq!(
            adapter
                .handle(KeyCode::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::CaptureActiveWindowToFile)
        );
    }

    #[test]
    fn super_release_toggles_launcher_once() {
        for key in [KeyCode::SuperLeft, KeyCode::SuperRight] {
            let mut controller = CompositorShortcutAdapter::default();
            assert_eq!(
                controller.handle(key, KeyEdge::Pressed),
                HotkeyOutcome::default()
            );
            let released = controller.handle(key, KeyEdge::Released);
            assert_eq!(
                released,
                HotkeyOutcome {
                    action: Some(HotkeyAction::ToggleLauncher),
                    suppress: false,
                }
            );
            controller.launcher_visibility_applied(true);
            controller.handle(key, KeyEdge::Pressed);
            assert_eq!(
                controller.handle(key, KeyEdge::Released),
                HotkeyOutcome {
                    action: Some(HotkeyAction::ToggleLauncher),
                    suppress: false,
                }
            );
        }
    }

    #[test]
    fn pointer_chord_never_toggles_launcher() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert!(controller.begin_pointer_chord());
        assert_eq!(
            controller
                .handle(KeyCode::SuperLeft, KeyEdge::Released)
                .action,
            None
        );
    }

    #[test]
    fn run_release_ends_super_chord_even_without_super_release() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::KeyR, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::ShowRun),
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::KeyR, KeyEdge::Released),
            HotkeyOutcome {
                action: None,
                suppress: true,
            }
        );
        assert!(!controller.snapshot().super_held);
        assert_eq!(
            controller.handle(KeyCode::KeyR, KeyEdge::Pressed),
            HotkeyOutcome::default()
        );
    }

    #[test]
    fn alt_tab_commits_on_alt_release() {
        let mut controller = CompositorShortcutAdapter::default();
        assert_eq!(
            controller.handle(KeyCode::AltLeft, KeyEdge::Pressed),
            HotkeyOutcome::default()
        );
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::SwitchNext),
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Released),
            HotkeyOutcome {
                action: None,
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::AltLeft, KeyEdge::Released),
            HotkeyOutcome {
                action: Some(HotkeyAction::CommitSwitch),
                suppress: false,
            }
        );
    }

    #[test]
    fn repeated_alt_tab_keydowns_are_suppressed_until_release() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::AltLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Pressed).action,
            Some(HotkeyAction::SwitchNext)
        );
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Pressed).action,
            None
        );
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Pressed).action,
            None
        );
        controller.handle(KeyCode::Tab, KeyEdge::Released);
        assert_eq!(
            controller.handle(KeyCode::Tab, KeyEdge::Pressed).action,
            Some(HotkeyAction::SwitchNext)
        );
    }

    #[test]
    fn owned_tab_release_stays_suppressed_after_alt_commits_first() {
        for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
            let mut controller = CompositorShortcutAdapter::default();
            controller.handle(alt, KeyEdge::Pressed);
            assert!(controller.handle(KeyCode::Tab, KeyEdge::Pressed).suppress);
            assert_eq!(
                controller.handle(alt, KeyEdge::Released).action,
                Some(HotkeyAction::CommitSwitch)
            );
            assert_eq!(
                controller.handle(KeyCode::Tab, KeyEdge::Released),
                HotkeyOutcome {
                    action: None,
                    suppress: true,
                }
            );
            assert_eq!(
                controller.handle(KeyCode::Tab, KeyEdge::Released),
                HotkeyOutcome::default(),
                "ownership ends on the physical release edge"
            );
        }
    }

    #[test]
    fn reverse_switch_accepts_every_modifier_side_and_press_order() {
        for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
            for shift in [KeyCode::ShiftLeft, KeyCode::ShiftRight] {
                for modifiers in [[alt, shift], [shift, alt]] {
                    let mut controller = CompositorShortcutAdapter::default();
                    for modifier in modifiers {
                        controller.handle(modifier, KeyEdge::Pressed);
                    }
                    assert_eq!(
                        controller.handle(KeyCode::Tab, KeyEdge::Pressed),
                        HotkeyOutcome {
                            action: Some(HotkeyAction::SwitchPrevious),
                            suppress: true,
                        }
                    );
                    assert!(controller.handle(KeyCode::Tab, KeyEdge::Released).suppress);
                    assert_eq!(
                        controller.handle(alt, KeyEdge::Released).action,
                        Some(HotkeyAction::CommitSwitch)
                    );
                }
            }
        }
    }

    #[test]
    fn alt_grave_cycles_within_active_group() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::AltLeft, KeyEdge::Pressed);
        assert_eq!(
            controller
                .handle(KeyCode::Backquote, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::SwitchGroupNext)
        );
        controller.handle(KeyCode::Backquote, KeyEdge::Released);
        assert_eq!(
            controller
                .handle(KeyCode::AltLeft, KeyEdge::Released)
                .action,
            Some(HotkeyAction::CommitSwitch)
        );
    }

    #[test]
    fn alt_print_screen_captures_once_per_press() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::AltLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::PrintScreen, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::CaptureActiveWindow),
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::PrintScreen, KeyEdge::Pressed),
            HotkeyOutcome {
                action: None,
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::PrintScreen, KeyEdge::Released),
            HotkeyOutcome {
                action: None,
                suppress: true,
            }
        );
    }

    #[test]
    fn alt_shift_print_screen_uses_the_file_capture_action() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::AltLeft, KeyEdge::Pressed);
        controller.handle(KeyCode::ShiftLeft, KeyEdge::Pressed);

        assert_eq!(
            controller.handle(KeyCode::PrintScreen, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::CaptureActiveWindowToFile),
                suppress: true,
            }
        );
    }

    #[test]
    fn print_screen_opens_crop_tool() {
        let mut controller = CompositorShortcutAdapter::default();
        assert_eq!(
            controller.handle(KeyCode::PrintScreen, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::ShowScreenshotTool),
                suppress: true,
            }
        );
        assert!(controller.snapshot().print_screen_held);
    }

    #[test]
    fn orphaned_print_screen_release_opens_crop_tool_once() {
        let mut controller = CompositorShortcutAdapter::default();
        assert_eq!(
            controller.handle_reconciled(KeyCode::PrintScreen, KeyEdge::Released),
            HotkeyOutcome {
                action: Some(HotkeyAction::ShowScreenshotTool),
                suppress: true,
            }
        );
        assert!(!controller.snapshot().print_screen_held);
    }

    #[test]
    fn physical_reconciliation_clears_stale_super() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        controller.reconcile_super(false);
        assert!(!controller.begin_pointer_chord());
    }

    #[test]
    fn workspace_chords_distinguish_switch_move_direction_and_repeats() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        controller.handle(KeyCode::ControlLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::ArrowRight, KeyEdge::Pressed),
            HotkeyOutcome {
                action: Some(HotkeyAction::SwitchWorkspaceNext),
                suppress: true,
            }
        );
        assert_eq!(
            controller
                .handle(KeyCode::ArrowRight, KeyEdge::Pressed)
                .action,
            None
        );
        controller.handle(KeyCode::ArrowRight, KeyEdge::Released);
        controller.handle(KeyCode::ShiftLeft, KeyEdge::Pressed);
        assert_eq!(
            controller
                .handle(KeyCode::ArrowLeft, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::MoveWindowToPreviousWorkspace)
        );
    }

    #[test]
    fn both_lock_chords_are_edge_triggered_and_consumed() {
        for modifiers in [
            vec![KeyCode::SuperLeft],
            vec![KeyCode::ControlRight, KeyCode::AltLeft],
        ] {
            let mut controller = CompositorShortcutAdapter::default();
            for modifier in modifiers {
                controller.handle(modifier, KeyEdge::Pressed);
            }
            assert_eq!(
                controller.handle(KeyCode::KeyL, KeyEdge::Pressed),
                HotkeyOutcome {
                    action: Some(HotkeyAction::LockSession),
                    suppress: true,
                }
            );
            assert_eq!(
                controller.handle(KeyCode::KeyL, KeyEdge::Pressed),
                HotkeyOutcome {
                    action: None,
                    suppress: true,
                }
            );
            assert_eq!(
                controller.handle(KeyCode::KeyL, KeyEdge::Released),
                HotkeyOutcome {
                    action: None,
                    suppress: true,
                }
            );
        }
    }

    #[test]
    fn owned_lock_release_stays_suppressed_after_modifiers_release_first() {
        for modifiers in [
            vec![KeyCode::SuperRight],
            vec![KeyCode::ControlLeft, KeyCode::AltRight],
        ] {
            let mut controller = CompositorShortcutAdapter::default();
            for modifier in &modifiers {
                controller.handle(*modifier, KeyEdge::Pressed);
            }
            assert_eq!(
                controller.handle(KeyCode::KeyL, KeyEdge::Pressed).action,
                Some(HotkeyAction::LockSession)
            );
            for modifier in modifiers.into_iter().rev() {
                controller.handle(modifier, KeyEdge::Released);
            }
            assert_eq!(
                controller.handle(KeyCode::KeyL, KeyEdge::Released),
                HotkeyOutcome {
                    action: None,
                    suppress: true,
                }
            );
        }
    }

    #[test]
    fn secure_focus_reset_retains_only_intercepted_release_ownership() {
        let mut controller = CompositorShortcutAdapter::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::KeyL, KeyEdge::Pressed).action,
            Some(HotkeyAction::LockSession)
        );
        controller.reset_chord_state_preserving_owned_releases();
        assert!(!controller.snapshot().super_held);
        assert_eq!(
            controller.handle(KeyCode::KeyL, KeyEdge::Released),
            HotkeyOutcome {
                action: None,
                suppress: true,
            }
        );
        assert_eq!(
            controller.handle(KeyCode::SuperLeft, KeyEdge::Released),
            HotkeyOutcome::default()
        );
    }

    #[test]
    fn resetting_pressed_state_recovers_from_missing_releases() {
        let mut adapter = CompositorShortcutAdapter::default();
        adapter.handle(KeyCode::ControlLeft, KeyEdge::Pressed);
        adapter.handle(KeyCode::AltRight, KeyEdge::Pressed);
        assert_eq!(
            adapter.handle(KeyCode::KeyL, KeyEdge::Pressed).action,
            Some(HotkeyAction::LockSession)
        );
        adapter.reset_pressed_state();
        assert_eq!(
            adapter.handle(KeyCode::KeyL, KeyEdge::Pressed),
            HotkeyOutcome::default()
        );
    }
}
