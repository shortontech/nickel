#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LauncherVisibility {
    #[default]
    Hidden,
    Visible,
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
}

#[cfg(test)]
mod tests {
    use super::LauncherVisibility;

    #[test]
    fn repeated_toggle_opens_then_closes_launcher() {
        let mut visibility = LauncherVisibility::default();
        assert!(visibility.toggle());
        assert!(!visibility.toggle());
    }
}
