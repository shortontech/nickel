use schemars::JsonSchema;
use serde::Deserialize;

/// Bounded transactions and explicitly owned, cancellable held-key gestures.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyboardAction {
    Text { text: String },
    Key { keysym: u32, modifiers: Vec<u32> },
    HoldStart { keysym: u32, modifiers: Vec<u32> },
    HoldKeepAlive,
    HoldEnd,
    HoldCancel,
}

impl std::fmt::Debug for KeyboardAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("KeyboardAction(<redacted>)")
    }
}

impl KeyboardAction {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Text { text }
                if text.is_empty()
                    || text.chars().count() > 256
                    || text.chars().any(char::is_control) =>
            {
                Err("text must contain 1–256 printable characters".into())
            }
            Self::Key { keysym, modifiers } | Self::HoldStart { keysym, modifiers }
                if *keysym == 0
                    || modifiers.len() > 5
                    || modifiers
                        .iter()
                        .any(|key| !matches!(*key, 0xffe1 | 0xffe3 | 0xffe9 | 0xffeb | 0xfe03)) =>
            {
                Err("invalid key or chord modifiers".into())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_text_and_chords_do_not_expose_input_in_debug_output() {
        let text = KeyboardAction::Text {
            text: "private input".into(),
        };
        assert!(text.validate().is_ok());
        assert_eq!(format!("{text:?}"), "KeyboardAction(<redacted>)");
        for text in [String::new(), "a".repeat(257), "secret\n".into()] {
            assert!(KeyboardAction::Text { text }.validate().is_err());
        }
        assert!(
            KeyboardAction::Key {
                keysym: 0xff0d,
                modifiers: vec![0xffe3]
            }
            .validate()
            .is_ok()
        );
        assert!(
            KeyboardAction::Key {
                keysym: 0xff0d,
                modifiers: vec![0x61]
            }
            .validate()
            .is_err()
        );
    }
}
