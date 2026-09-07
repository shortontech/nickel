#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SessionActivity {
    #[default]
    Active,
    Paused,
}

impl SessionActivity {
    pub fn pause(&mut self) -> bool {
        let changed = *self != Self::Paused;
        *self = Self::Paused;
        changed
    }

    pub fn activate(&mut self) -> bool {
        let changed = *self != Self::Active;
        *self = Self::Active;
        changed
    }

    pub fn is_active(self) -> bool {
        self == Self::Active
    }
}

#[cfg(test)]
mod tests {
    use super::SessionActivity;

    #[test]
    fn duplicate_pause_and_activation_are_idempotent() {
        let mut activity = SessionActivity::default();
        assert!(activity.pause());
        assert!(!activity.pause());
        assert!(!activity.is_active());
        assert!(activity.activate());
        assert!(!activity.activate());
        assert!(activity.is_active());
    }
}
