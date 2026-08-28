#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hotkey {
    Super,
    Alt,
    Shift,
    Tab,
    Grave,
    Run,
    PrintScreen,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEdge {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeyAction {
    ShowLauncher,
    HideLauncher,
    ShowRun,
    SwitchNext,
    SwitchPrevious,
    SwitchGroupNext,
    SwitchGroupPrevious,
    CommitSwitch,
    CaptureActiveWindow,
    CaptureActiveWindowToFile,
    ShowScreenshotTool,
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
    pub tab_held: bool,
    pub grave_held: bool,
    pub run_held: bool,
    pub print_screen_held: bool,
    pub switch_active: bool,
    pub launcher_visible: bool,
}

#[derive(Debug, Default)]
pub struct HotkeyController {
    super_held: bool,
    super_chorded: bool,
    alt_held: bool,
    shift_held: bool,
    tab_held: bool,
    grave_held: bool,
    run_held: bool,
    print_screen_held: bool,
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
            tab_held: self.tab_held,
            grave_held: self.grave_held,
            run_held: self.run_held,
            print_screen_held: self.print_screen_held,
            switch_active: self.switch_active,
            launcher_visible: self.launcher_visible,
        }
    }

    pub fn handle(&mut self, key: Hotkey, edge: KeyEdge) -> HotkeyOutcome {
        match (key, edge) {
            (Hotkey::Super, KeyEdge::Pressed) => {
                if !self.super_held {
                    self.super_held = true;
                    self.super_chorded = false;
                }
                // Observe Super without taking it away from the platform or applications.
                // This lets another shell present its own Start surface alongside Nickel.
                HotkeyOutcome::default()
            }
            (Hotkey::Super, KeyEdge::Released) => {
                if !self.super_held {
                    return HotkeyOutcome::default();
                }
                self.super_held = false;
                let action = if self.super_chorded {
                    None
                } else if self.launcher_visible {
                    Some(HotkeyAction::HideLauncher)
                } else {
                    Some(HotkeyAction::ShowLauncher)
                };
                self.super_chorded = false;
                HotkeyOutcome {
                    action,
                    suppress: false,
                }
            }
            (Hotkey::Run, KeyEdge::Pressed) if self.super_held => {
                self.super_chorded = true;
                let action = (!self.run_held).then_some(HotkeyAction::ShowRun);
                self.run_held = true;
                HotkeyOutcome {
                    action,
                    suppress: true,
                }
            }
            (Hotkey::Run, KeyEdge::Released) if self.run_held => {
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
            (Hotkey::Alt, KeyEdge::Pressed) => {
                self.alt_held = true;
                HotkeyOutcome::default()
            }
            (Hotkey::Alt, KeyEdge::Released) => {
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
            (Hotkey::Shift, edge) => {
                self.shift_held = edge == KeyEdge::Pressed;
                HotkeyOutcome::default()
            }
            (Hotkey::Tab, KeyEdge::Pressed) if self.alt_held => {
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
            (Hotkey::Tab, KeyEdge::Released) if self.tab_held => {
                self.tab_held = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            (Hotkey::Grave, KeyEdge::Pressed) if self.alt_held => {
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
            (Hotkey::Grave, KeyEdge::Released) if self.grave_held => {
                self.grave_held = false;
                HotkeyOutcome {
                    suppress: true,
                    ..Default::default()
                }
            }
            (Hotkey::PrintScreen, KeyEdge::Pressed) => {
                let action = (!self.print_screen_held).then_some(if self.alt_held {
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
            (Hotkey::PrintScreen, KeyEdge::Released) if self.print_screen_held => {
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

    pub fn handle_reconciled(&mut self, key: Hotkey, edge: KeyEdge) -> HotkeyOutcome {
        if key == Hotkey::PrintScreen && edge == KeyEdge::Released && !self.print_screen_held {
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
    use super::{Hotkey, HotkeyAction, HotkeyController, KeyEdge};

    #[test]
    fn super_release_toggles_launcher_once() {
        let mut controller = HotkeyController::default();
        assert!(!controller.handle(Hotkey::Super, KeyEdge::Pressed).suppress);
        let released = controller.handle(Hotkey::Super, KeyEdge::Released);
        assert!(!released.suppress);
        assert_eq!(released.action, Some(HotkeyAction::ShowLauncher));
        controller.launcher_visibility_applied(true);
        controller.handle(Hotkey::Super, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(Hotkey::Super, KeyEdge::Released).action,
            Some(HotkeyAction::HideLauncher)
        );
    }

    #[test]
    fn registered_bare_super_toggles_only_when_released() {
        let mut controller = HotkeyController::default();
        controller.registered_super_pressed();
        assert!(controller.snapshot().super_held);
        assert_eq!(
            controller.handle(Hotkey::Super, KeyEdge::Released).action,
            Some(HotkeyAction::ShowLauncher)
        );
        controller.launcher_visibility_applied(true);
        controller.registered_super_pressed();
        assert_eq!(
            controller.handle(Hotkey::Super, KeyEdge::Released).action,
            Some(HotkeyAction::HideLauncher)
        );
    }

    #[test]
    fn pointer_chord_never_toggles_launcher() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Super, KeyEdge::Pressed);
        assert!(controller.begin_pointer_chord());
        assert_eq!(
            controller.handle(Hotkey::Super, KeyEdge::Released).action,
            None
        );
    }

    #[test]
    fn run_release_ends_super_chord_even_without_super_release() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Super, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(Hotkey::Run, KeyEdge::Pressed).action,
            Some(HotkeyAction::ShowRun)
        );
        controller.handle(Hotkey::Run, KeyEdge::Released);
        assert!(!controller.snapshot().super_held);
        assert!(!controller.handle(Hotkey::Run, KeyEdge::Pressed).suppress);
    }

    #[test]
    fn alt_tab_commits_on_alt_release() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Alt, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(Hotkey::Tab, KeyEdge::Pressed).action,
            Some(HotkeyAction::SwitchNext)
        );
        controller.handle(Hotkey::Tab, KeyEdge::Released);
        assert_eq!(
            controller.handle(Hotkey::Alt, KeyEdge::Released).action,
            Some(HotkeyAction::CommitSwitch)
        );
    }

    #[test]
    fn alt_grave_cycles_within_active_group() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Alt, KeyEdge::Pressed);
        assert_eq!(
            controller.handle(Hotkey::Grave, KeyEdge::Pressed).action,
            Some(HotkeyAction::SwitchGroupNext)
        );
        controller.handle(Hotkey::Grave, KeyEdge::Released);
        assert_eq!(
            controller.handle(Hotkey::Alt, KeyEdge::Released).action,
            Some(HotkeyAction::CommitSwitch)
        );
    }

    #[test]
    fn alt_print_screen_captures_once_per_press() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Alt, KeyEdge::Pressed);
        assert_eq!(
            controller
                .handle(Hotkey::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::CaptureActiveWindow)
        );
        assert_eq!(
            controller
                .handle(Hotkey::PrintScreen, KeyEdge::Pressed)
                .action,
            None
        );
        assert!(
            controller
                .handle(Hotkey::PrintScreen, KeyEdge::Released)
                .suppress
        );
    }

    #[test]
    fn print_screen_opens_crop_tool() {
        let mut controller = HotkeyController::default();
        assert_eq!(
            controller
                .handle(Hotkey::PrintScreen, KeyEdge::Pressed)
                .action,
            Some(HotkeyAction::ShowScreenshotTool)
        );
    }

    #[test]
    fn orphaned_print_screen_release_opens_crop_tool_once() {
        let mut controller = HotkeyController::default();
        assert_eq!(
            controller
                .handle_reconciled(Hotkey::PrintScreen, KeyEdge::Released)
                .action,
            Some(HotkeyAction::ShowScreenshotTool)
        );
        assert!(!controller.snapshot().print_screen_held);
    }

    #[test]
    fn physical_reconciliation_clears_stale_super() {
        let mut controller = HotkeyController::default();
        controller.handle(Hotkey::Super, KeyEdge::Pressed);
        controller.reconcile_super(false);
        assert!(!controller.begin_pointer_chord());
    }
}
