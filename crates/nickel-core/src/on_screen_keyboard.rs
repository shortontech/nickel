//! Portable on-screen keyboard policy. Device discovery and input delivery belong to adapters.

pub const ENVIRONMENT_VARIABLE: &str = "NICKEL_ON_SCREEN_KEYBOARD";

pub use twinkle_input::on_screen_keyboard::{
    KEYBOARD_HEIGHT, KeyDefinition, KeyboardDisplayKey, KeyboardKey, KeyboardMetrics,
    KeyboardPanel, Latch, VirtualModifier, VirtualModifiers, compact_us_keyboard_rows,
    keyboard_display_rows, resolve_keyboard_display_key, us_keyboard_rows,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardPreference {
    #[default]
    Automatic,
    Enabled,
    Disabled,
}

impl KeyboardPreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "automatic" => Some(Self::Automatic),
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TouchscreenPresence {
    Present,
    Absent,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardOverride {
    #[default]
    None,
    Enabled,
    Disabled,
}

impl KeyboardOverride {
    /// Invalid values return `None`, distinct from the valid `auto` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "auto" => Some(Self::None),
            "1" => Some(Self::Enabled),
            "0" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnablementSource {
    Environment,
    Preference,
    Touchscreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardEnablement {
    pub enabled: bool,
    pub source: EnablementSource,
    pub touchscreen: TouchscreenPresence,
}

pub fn resolve_enablement(
    preference: KeyboardPreference,
    environment: KeyboardOverride,
    touchscreen: TouchscreenPresence,
) -> KeyboardEnablement {
    let (enabled, source) = match environment {
        KeyboardOverride::Enabled => (true, EnablementSource::Environment),
        KeyboardOverride::Disabled => (false, EnablementSource::Environment),
        KeyboardOverride::None => match preference {
            KeyboardPreference::Enabled => (true, EnablementSource::Preference),
            KeyboardPreference::Disabled => (false, EnablementSource::Preference),
            KeyboardPreference::Automatic => (
                touchscreen == TouchscreenPresence::Present,
                EnablementSource::Touchscreen,
            ),
        },
    };
    KeyboardEnablement {
        enabled,
        source,
        touchscreen,
    }
}

impl KeyboardEnablement {
    pub const fn status(self) -> &'static str {
        match (self.enabled, self.source, self.touchscreen) {
            (true, EnablementSource::Environment, _) => "On for this session",
            (false, EnablementSource::Environment, _) => "Off for this session",
            (true, EnablementSource::Preference, _) => "On — enabled manually",
            (false, EnablementSource::Preference, _) => "Off — disabled manually",
            (true, EnablementSource::Touchscreen, _) => "On — touchscreen detected",
            (false, EnablementSource::Touchscreen, TouchscreenPresence::Unknown) => {
                "Off — touchscreen detection unavailable"
            }
            (false, EnablementSource::Touchscreen, _) => "Off — no touchscreen detected",
        }
    }
}

/// The epoch distinguishes refocusing the same field from an older delivery request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientLease<T> {
    pub target: T,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenReason {
    Touch,
    Controller,
    Explicit,
}

/// Lifecycle state carries identities only, never typed or surrounding text.
#[derive(Clone, Debug)]
pub struct KeyboardSession<T> {
    enabled: bool,
    recipient: Option<RecipientLease<T>>,
    visible: bool,
    suppressed: bool,
    epoch: u64,
}

impl<T: Clone + Eq> Default for KeyboardSession<T> {
    fn default() -> Self {
        Self {
            enabled: false,
            recipient: None,
            visible: false,
            suppressed: false,
            epoch: 0,
        }
    }
}

impl<T: Clone + Eq> KeyboardSession<T> {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.invalidate();
        }
    }

    /// Call on an authoritative text-focus transition, not on keyboard UI selection changes.
    pub fn focus(&mut self, target: Option<T>) {
        self.invalidate();
        self.recipient = target.map(|target| RecipientLease {
            target,
            epoch: self.epoch,
        });
    }

    pub fn open(&mut self, reason: OpenReason) -> bool {
        if !self.enabled || (reason != OpenReason::Explicit && self.recipient.is_none()) {
            return false;
        }
        // Each call represents a deliberate activation, including reopening the same field.
        self.suppressed = false;
        self.visible = true;
        true
    }

    pub fn automatic_focus_activation(&mut self) -> bool {
        if !self.enabled || self.suppressed || self.recipient.is_none() {
            return false;
        }
        self.visible = true;
        true
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.suppressed = true;
        // Invalidate queued presses while retaining the recipient for explicit reopening.
        self.epoch = self.epoch.wrapping_add(1);
        if let Some(recipient) = &mut self.recipient {
            recipient.epoch = self.epoch;
        }
    }

    pub fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.recipient = None;
        self.visible = false;
        self.suppressed = false;
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn recipient(&self) -> Option<&RecipientLease<T>> {
        self.recipient.as_ref()
    }

    pub fn permits_delivery(&self, lease: &RecipientLease<T>) -> bool {
        self.enabled && self.visible && self.recipient.as_ref() == Some(lease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_default_and_explicit_preferences_obey_override_precedence() {
        for touch in [
            TouchscreenPresence::Present,
            TouchscreenPresence::Absent,
            TouchscreenPresence::Unknown,
        ] {
            for preference in [
                KeyboardPreference::Automatic,
                KeyboardPreference::Enabled,
                KeyboardPreference::Disabled,
            ] {
                assert!(resolve_enablement(preference, KeyboardOverride::Enabled, touch).enabled);
                assert!(!resolve_enablement(preference, KeyboardOverride::Disabled, touch).enabled);
            }
            assert!(
                resolve_enablement(KeyboardPreference::Enabled, KeyboardOverride::None, touch)
                    .enabled
            );
            assert!(
                !resolve_enablement(KeyboardPreference::Disabled, KeyboardOverride::None, touch)
                    .enabled
            );
        }
        assert!(
            resolve_enablement(
                KeyboardPreference::Automatic,
                KeyboardOverride::None,
                TouchscreenPresence::Present
            )
            .enabled
        );
        assert!(
            !resolve_enablement(
                KeyboardPreference::Automatic,
                KeyboardOverride::None,
                TouchscreenPresence::Absent
            )
            .enabled
        );
        let unknown = resolve_enablement(
            KeyboardPreference::Automatic,
            KeyboardOverride::None,
            TouchscreenPresence::Unknown,
        );
        assert!(!unknown.enabled);
        assert!(unknown.status().contains("unavailable"));
        assert_eq!(
            KeyboardOverride::parse(" 1 "),
            Some(KeyboardOverride::Enabled)
        );
        assert_eq!(
            KeyboardOverride::parse("0"),
            Some(KeyboardOverride::Disabled)
        );
        assert_eq!(
            KeyboardOverride::parse("auto"),
            Some(KeyboardOverride::None)
        );
        assert_eq!(KeyboardOverride::parse("yes"), None);
    }

    #[test]
    fn dismissed_keyboard_stays_hidden_and_queued_input_cannot_cross_reopening() {
        let mut session = KeyboardSession::default();
        session.set_enabled(true);
        session.focus(Some("composer"));
        assert!(session.open(OpenReason::Controller));
        let lease = session.recipient().unwrap().clone();
        assert!(session.permits_delivery(&lease));
        session.hide();
        assert!(!session.automatic_focus_activation());
        assert!(session.open(OpenReason::Explicit));
        assert!(!session.permits_delivery(&lease));
        assert!(session.permits_delivery(session.recipient().unwrap()));
    }

    #[test]
    fn focus_change_or_disable_rejects_input_even_when_refocusing_same_field() {
        let mut session = KeyboardSession::default();
        session.set_enabled(true);
        session.focus(Some("password"));
        session.open(OpenReason::Touch);
        let lease = session.recipient().unwrap().clone();
        session.focus(Some("password"));
        session.open(OpenReason::Touch);
        assert!(!session.permits_delivery(&lease));
        let current = session.recipient().unwrap().clone();
        session.set_enabled(false);
        assert!(!session.visible());
        assert!(!session.permits_delivery(&current));
        assert!(!session.open(OpenReason::Explicit));
        session.set_enabled(true);
        session.open(OpenReason::Explicit);
        assert!(session.recipient().is_none());
    }
}
