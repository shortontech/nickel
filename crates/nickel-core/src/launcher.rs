#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LauncherVisibility {
    #[default]
    Hidden,
    Visible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherPointerTarget {
    Launcher,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherActivationSource {
    Pointer,
    Keyboard,
    Controller,
    Accessibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherSemanticTarget {
    PanelButton,
    Surface,
    Desktop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LauncherActivation {
    pub source: LauncherActivationSource,
    pub target: LauncherSemanticTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherTransition {
    Unchanged,
    Shown,
    Hidden,
}

impl LauncherVisibility {
    pub fn is_visible(self) -> bool {
        self == Self::Visible
    }

    pub fn toggle(&mut self) -> bool {
        self.set(!self.is_visible())
    }

    pub fn set(&mut self, visible: bool) -> bool {
        *self = if visible { Self::Visible } else { Self::Hidden };
        visible
    }

    pub fn pointer_press(&mut self, target: LauncherPointerTarget) -> bool {
        if self.is_visible() && target == LauncherPointerTarget::Other {
            self.set(false);
            true
        } else {
            false
        }
    }

    pub fn activate(&mut self, activation: LauncherActivation) -> LauncherTransition {
        match activation.target {
            LauncherSemanticTarget::PanelButton => {
                if self.toggle() {
                    LauncherTransition::Shown
                } else {
                    LauncherTransition::Hidden
                }
            }
            LauncherSemanticTarget::Surface => LauncherTransition::Unchanged,
            LauncherSemanticTarget::Desktop => {
                if self.pointer_press(LauncherPointerTarget::Other) {
                    LauncherTransition::Hidden
                } else {
                    LauncherTransition::Unchanged
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LauncherPointerTarget, LauncherVisibility};

    #[test]
    fn repeated_toggle_opens_then_closes_launcher() {
        let mut visibility = LauncherVisibility::default();
        assert!(!visibility.is_visible());
        assert!(visibility.toggle());
        assert!(visibility.is_visible());
        assert!(!visibility.toggle());
        assert!(!visibility.is_visible());
    }

    #[test]
    fn only_an_outside_pointer_press_dismisses_a_visible_launcher() {
        let mut visibility = LauncherVisibility::Visible;
        assert!(!visibility.pointer_press(LauncherPointerTarget::Launcher));
        assert!(visibility.is_visible());
        assert!(visibility.pointer_press(LauncherPointerTarget::Other));
        assert!(!visibility.is_visible());
    }
}
