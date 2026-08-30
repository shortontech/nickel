use std::collections::BTreeMap;

use nickel_input::{
    AggregateModifier, PhysicalKey, Shortcut, ShortcutKey, ShortcutTrigger,
    global::{
        GlobalShortcutAdapter, GlobalShortcutEdge, Registration, RegistrationError, RegistrationId,
        RegistrationTable, ShortcutCapability, ShortcutOwnership,
    },
};
pub use nickel_input::{KeyCode, KeyEdge};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum HotkeyAction {
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
    controller: HotkeyController,
    registrations: RegistrationTable<HotkeyAction>,
    actions: BTreeMap<HotkeyAction, RegistrationId>,
}

impl Default for CompositorShortcutAdapter {
    fn default() -> Self {
        let mut adapter = Self {
            controller: HotkeyController::default(),
            registrations: RegistrationTable::default(),
            actions: BTreeMap::new(),
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
        let mut outcome = self.controller.handle(key, edge);
        if let Some(action) = outcome.action {
            outcome.action = self
                .actions
                .get(&action)
                .and_then(|id| {
                    self.registrations
                        .deliver(*id, GlobalShortcutEdge::Activated)
                })
                .map(|event| event.action);
        }
        outcome
    }

    pub fn handle_unmapped(&mut self, edge: KeyEdge) -> HotkeyOutcome {
        self.controller.handle_unmapped(edge)
    }

    pub fn snapshot(&self) -> HotkeySnapshot {
        self.controller.snapshot()
    }

    pub fn begin_pointer_chord(&mut self) -> bool {
        self.controller.begin_pointer_chord()
    }

    pub fn reconcile_super(&mut self, physically_held: bool) {
        self.controller.reconcile_super(physically_held);
    }

    pub fn reconcile_alt(&mut self, physically_held: bool) -> Option<HotkeyAction> {
        self.controller.reconcile_alt(physically_held)
    }

    pub fn launcher_visibility_applied(&mut self, visible: bool) {
        self.controller.launcher_visibility_applied(visible);
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
        self.registrations.register(registration)
    }

    fn unregister(&mut self, id: RegistrationId) -> bool {
        self.actions.retain(|_, registered| *registered != id);
        self.registrations.unregister(id)
    }

    fn reset(&mut self) {
        self.controller = HotkeyController::default();
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
            ShortcutTrigger::ModifierReleased(nickel_input::Modifier::AltLeft),
            CommitSwitch,
        ),
        registration(
            KeyCode::AltRight,
            [],
            ShortcutTrigger::ModifierReleased(nickel_input::Modifier::AltRight),
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
    pub print_screen_held: bool,
    pub left_held: bool,
    pub right_held: bool,
    pub switch_active: bool,
    pub launcher_visible: bool,
}

#[derive(Debug, Default)]
pub struct HotkeyController {
    super_held: bool,
    super_chorded: bool,
    alt_held: bool,
    shift_held: bool,
    control_held: bool,
    tab_held: bool,
    grave_held: bool,
    run_held: bool,
    print_screen_held: bool,
    left_held: bool,
    right_held: bool,
    switch_active: bool,
    launcher_visible: bool,
}

impl HotkeyController {
    pub fn snapshot(&self) -> HotkeySnapshot {
        HotkeySnapshot {
            super_held: self.super_held,
            super_chorded: self.super_chorded,
            alt_held: self.alt_held,
            shift_held: self.shift_held,
            control_held: self.control_held,
            tab_held: self.tab_held,
            grave_held: self.grave_held,
            run_held: self.run_held,
            print_screen_held: self.print_screen_held,
            left_held: self.left_held,
            right_held: self.right_held,
            switch_active: self.switch_active,
            launcher_visible: self.launcher_visible,
        }
    }

    pub fn handle(&mut self, key: KeyCode, edge: KeyEdge) -> HotkeyOutcome {
        match (key, edge) {
            (KeyCode::SuperLeft | KeyCode::SuperRight, KeyEdge::Pressed) => {
                if !self.super_held {
                    self.super_held = true;
                    self.super_chorded = false;
                }
                // Observe Super without taking it away from the platform or applications.
                // This lets another shell present its own Start surface alongside Nickel.
                HotkeyOutcome::default()
            }
            (KeyCode::SuperLeft | KeyCode::SuperRight, KeyEdge::Released) => {
                if !self.super_held {
                    return HotkeyOutcome::default();
                }
                self.super_held = false;
                let action = (!self.super_chorded).then_some(HotkeyAction::ToggleLauncher);
                self.super_chorded = false;
                HotkeyOutcome {
                    action,
                    suppress: false,
                }
            }
            (KeyCode::KeyR, KeyEdge::Pressed) if self.super_held => {
                self.super_chorded = true;
                let action = (!self.run_held).then_some(HotkeyAction::ShowRun);
                self.run_held = true;
                HotkeyOutcome {
                    action,
                    suppress: true,
                }
            }
            (KeyCode::KeyR, KeyEdge::Released) if self.run_held => {
                self.run_held = false;
                // Super+R transfers focus to Run, and the platform may omit the later Super
                // release from the low-level hook during that transition. End the completed chord
                // here so a missed release cannot make subsequent R presses look like shortcuts.
                self.super_held = false;
                self.super_chorded = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            (KeyCode::AltLeft | KeyCode::AltRight, KeyEdge::Pressed) => {
                self.alt_held = true;
                HotkeyOutcome::default()
            }
            (KeyCode::AltLeft | KeyCode::AltRight, KeyEdge::Released) => {
                self.alt_held = false;
                self.tab_held = false;
                self.grave_held = false;
                self.print_screen_held = false;
                let action = self.switch_active.then_some(HotkeyAction::CommitSwitch);
                self.switch_active = false;
                HotkeyOutcome {
                    action,
                    suppress: false,
                }
            }
            (KeyCode::ShiftLeft | KeyCode::ShiftRight, edge) => {
                self.shift_held = edge == KeyEdge::Pressed;
                HotkeyOutcome::default()
            }
            (KeyCode::ControlLeft | KeyCode::ControlRight, edge) => {
                self.control_held = edge == KeyEdge::Pressed;
                HotkeyOutcome::default()
            }
            (KeyCode::ArrowLeft | KeyCode::ArrowRight, KeyEdge::Pressed)
                if self.super_held && self.control_held =>
            {
                self.super_chorded = true;
                let held = match key {
                    KeyCode::ArrowLeft => &mut self.left_held,
                    KeyCode::ArrowRight => &mut self.right_held,
                    _ => unreachable!(),
                };
                if *held {
                    return HotkeyOutcome {
                        suppress: true,
                        ..Default::default()
                    };
                }
                *held = true;
                let action = match (key, self.shift_held) {
                    (KeyCode::ArrowLeft, false) => HotkeyAction::SwitchWorkspacePrevious,
                    (KeyCode::ArrowRight, false) => HotkeyAction::SwitchWorkspaceNext,
                    (KeyCode::ArrowLeft, true) => HotkeyAction::MoveWindowToPreviousWorkspace,
                    (KeyCode::ArrowRight, true) => HotkeyAction::MoveWindowToNextWorkspace,
                    _ => unreachable!(),
                };
                HotkeyOutcome {
                    action: Some(action),
                    suppress: true,
                }
            }
            (KeyCode::ArrowLeft | KeyCode::ArrowRight, KeyEdge::Released) => {
                match key {
                    KeyCode::ArrowLeft => self.left_held = false,
                    KeyCode::ArrowRight => self.right_held = false,
                    _ => unreachable!(),
                }
                HotkeyOutcome {
                    suppress: self.super_held && self.control_held,
                    ..Default::default()
                }
            }
            (KeyCode::Tab, KeyEdge::Pressed) if self.alt_held => {
                let action = if self.tab_held {
                    None
                } else if self.shift_held {
                    Some(HotkeyAction::SwitchPrevious)
                } else {
                    Some(HotkeyAction::SwitchNext)
                };
                self.tab_held = true;
                self.switch_active = true;
                HotkeyOutcome {
                    action,
                    suppress: true,
                }
            }
            (KeyCode::Tab, KeyEdge::Released) if self.tab_held => {
                self.tab_held = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            (KeyCode::Backquote, KeyEdge::Pressed) if self.alt_held => {
                let action = if self.grave_held {
                    None
                } else if self.shift_held {
                    Some(HotkeyAction::SwitchGroupPrevious)
                } else {
                    Some(HotkeyAction::SwitchGroupNext)
                };
                self.grave_held = true;
                self.switch_active = true;
                HotkeyOutcome {
                    action,
                    suppress: true,
                }
            }
            (KeyCode::Backquote, KeyEdge::Released) if self.grave_held => {
                self.grave_held = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            (KeyCode::PrintScreen, KeyEdge::Pressed) => {
                let action =
                    (!self.print_screen_held).then_some(if self.alt_held && self.shift_held {
                        HotkeyAction::CaptureActiveWindowToFile
                    } else if self.alt_held {
                        HotkeyAction::CaptureActiveWindow
                    } else {
                        HotkeyAction::ShowScreenshotTool
                    });
                self.print_screen_held = true;
                HotkeyOutcome {
                    action,
                    suppress: true,
                }
            }
            (KeyCode::PrintScreen, KeyEdge::Released) if self.print_screen_held => {
                self.print_screen_held = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            _ => {
                if self.super_held && edge == KeyEdge::Pressed {
                    self.super_chorded = true;
                }
                HotkeyOutcome::default()
            }
        }
    }

    pub fn handle_unmapped(&mut self, edge: KeyEdge) -> HotkeyOutcome {
        if self.super_held && edge == KeyEdge::Pressed {
            self.super_chorded = true;
        }
        HotkeyOutcome::default()
    }

    pub fn handle_reconciled(&mut self, key: KeyCode, edge: KeyEdge) -> HotkeyOutcome {
        if key == KeyCode::PrintScreen && edge == KeyEdge::Released && !self.print_screen_held {
            let pressed = self.handle(key, KeyEdge::Pressed);
            self.handle(key, KeyEdge::Released);
            return pressed;
        }
        self.handle(key, edge)
    }

    pub fn begin_pointer_chord(&mut self) -> bool {
        if self.super_held {
            self.super_chorded = true;
            true
        } else {
            false
        }
    }

    pub fn reconcile_super(&mut self, physically_held: bool) {
        if !physically_held {
            self.super_held = false;
            self.super_chorded = false;
            self.run_held = false;
            self.left_held = false;
            self.right_held = false;
        }
    }

    pub fn reconcile_alt(&mut self, physically_held: bool) -> Option<HotkeyAction> {
        if physically_held || !self.alt_held {
            return None;
        }
        self.alt_held = false;
        self.tab_held = false;
        self.grave_held = false;
        self.print_screen_held = false;
        let action = self.switch_active.then_some(HotkeyAction::CommitSwitch);
        self.switch_active = false;
        action
    }

    pub fn launcher_visibility_applied(&mut self, visible: bool) {
        self.launcher_visible = visible;
    }

    pub fn registered_super_pressed(&mut self) {
        self.super_held = true;
        self.super_chorded = false;
    }
}

#[cfg(test)]
mod tests {
    use nickel_input::global::{
        GlobalShortcutAdapter, RegistrationError, ShortcutCapability, ShortcutOwnership,
    };
    use nickel_input::{KeyCode, KeyEdge};

    use super::{
        CompositorShortcutAdapter, HotkeyAction, HotkeyController, HotkeyOutcome,
        compositor_registrations,
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
        let mut controller = HotkeyController::default();
        assert_eq!(
            controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed),
            HotkeyOutcome::default()
        );
        let released = controller.handle(KeyCode::SuperLeft, KeyEdge::Released);
        assert_eq!(
            released,
            HotkeyOutcome {
                action: Some(HotkeyAction::ToggleLauncher),
                suppress: false,
            }
        );
        controller.launcher_visibility_applied(true);
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(KeyCode::SuperLeft, KeyEdge::Released),
            HotkeyOutcome {
                action: Some(HotkeyAction::ToggleLauncher),
                suppress: false,
            }
        );
    }

    #[test]
    fn registered_bare_super_toggles_only_when_released() {
        let mut controller = HotkeyController::default();
        controller.registered_super_pressed();
        assert!(controller.snapshot().super_held);
        assert_eq!(
            controller
                .handle(KeyCode::SuperLeft, KeyEdge::Released)
                .action,
            Some(HotkeyAction::ToggleLauncher)
        );
        controller.launcher_visibility_applied(true);
        controller.registered_super_pressed();
        assert_eq!(
            controller
                .handle(KeyCode::SuperLeft, KeyEdge::Released)
                .action,
            Some(HotkeyAction::ToggleLauncher)
        );
    }

    #[test]
    fn pointer_chord_never_toggles_launcher() {
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
    fn alt_grave_cycles_within_active_group() {
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
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
        let mut controller = HotkeyController::default();
        controller.handle(KeyCode::SuperLeft, KeyEdge::Pressed);
        controller.reconcile_super(false);
        assert!(!controller.begin_pointer_chord());
    }

    #[test]
    fn workspace_chords_distinguish_switch_move_direction_and_repeats() {
        let mut controller = HotkeyController::default();
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
}
