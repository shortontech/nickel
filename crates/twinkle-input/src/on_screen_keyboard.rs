//! Reusable keyboard layouts, display keys, metrics and virtual modifier state.
//! Enablement, recipient leases and native input delivery belong to the host.

pub const KEYBOARD_HEIGHT: u32 = 368;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardPanel {
    Letters,
    Symbols,
    Navigation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualModifier {
    Shift,
    Control,
    Alt,
    Super,
    AltGr,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Latch {
    #[default]
    Off,
    OneShot,
    Locked,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VirtualModifiers {
    pub shift: Latch,
    pub control: Latch,
    pub alt: Latch,
    pub super_key: Latch,
    pub alt_gr: Latch,
    pub caps_lock: bool,
}

impl VirtualModifiers {
    pub fn latch(&self, modifier: VirtualModifier) -> Latch {
        match modifier {
            VirtualModifier::Shift => self.shift,
            VirtualModifier::Control => self.control,
            VirtualModifier::Alt => self.alt,
            VirtualModifier::Super => self.super_key,
            VirtualModifier::AltGr => self.alt_gr,
        }
    }

    pub fn toggle(&mut self, modifier: VirtualModifier, persistent: bool) {
        let latch = match modifier {
            VirtualModifier::Shift => &mut self.shift,
            VirtualModifier::Control => &mut self.control,
            VirtualModifier::Alt => &mut self.alt,
            VirtualModifier::Super => &mut self.super_key,
            VirtualModifier::AltGr => &mut self.alt_gr,
        };
        *latch = if *latch != Latch::Off {
            Latch::Off
        } else if persistent {
            Latch::Locked
        } else {
            Latch::OneShot
        };
    }

    pub fn consume(&mut self) {
        for latch in [
            &mut self.shift,
            &mut self.control,
            &mut self.alt,
            &mut self.super_key,
            &mut self.alt_gr,
        ] {
            if *latch == Latch::OneShot {
                *latch = Latch::Off;
            }
        }
    }

    pub fn shifted(&self, letter: bool) -> bool {
        (self.shift != Latch::Off) ^ (letter && self.caps_lock)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardKey {
    Character { normal: char, shifted: char },
    Named(crate::NamedKey),
    Modifier(VirtualModifier),
    CapsLock,
    Panel(KeyboardPanel),
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyDefinition {
    pub id: String,
    pub label: String,
    pub key: KeyboardKey,
    /// Quarter-key units avoid cumulative row rounding and platform-dependent proportions.
    pub quarters: u16,
}

impl KeyDefinition {
    fn special(id: &str, label: &str, key: KeyboardKey, quarters: u16) -> Self {
        Self {
            id: format!("osk-{id}"),
            label: label.into(),
            key,
            quarters,
        }
    }

    fn character(normal: char, shifted: char) -> Self {
        Self::special(
            &format!("char-{}", normal as u32),
            &normal.to_string(),
            KeyboardKey::Character { normal, shifted },
            4,
        )
    }

    pub fn display_label(&self, modifiers: VirtualModifiers) -> String {
        match self.key {
            KeyboardKey::Character { normal, shifted } if normal != ' ' => {
                if modifiers.shifted(normal.is_alphabetic()) {
                    shifted
                } else {
                    normal
                }
                .to_string()
            }
            _ => self.label.clone(),
        }
    }
}

pub fn us_keyboard_rows(panel: KeyboardPanel) -> Vec<Vec<KeyDefinition>> {
    use crate::NamedKey;
    use KeyboardKey::{CapsLock, Modifier, Named, Panel};
    if panel == KeyboardPanel::Navigation {
        let named = |name: &str, key| KeyDefinition::special(name, name, Named(key), 8);
        return vec![
            vec![
                named("Esc", NamedKey::Escape),
                named("Tab", NamedKey::Tab),
                named("Home", NamedKey::Home),
                named("Up", NamedKey::ArrowUp),
                named("End", NamedKey::End),
            ],
            vec![
                named("Insert", NamedKey::Insert),
                named("Delete", NamedKey::Delete),
                named("Left", NamedKey::ArrowLeft),
                named("Down", NamedKey::ArrowDown),
                named("Right", NamedKey::ArrowRight),
            ],
            vec![
                named("Page Up", NamedKey::PageUp),
                named("Page Down", NamedKey::PageDown),
                KeyDefinition::special("letters", "ABC", Panel(KeyboardPanel::Letters), 12),
            ],
            [
                NamedKey::F1,
                NamedKey::F2,
                NamedKey::F3,
                NamedKey::F4,
                NamedKey::F5,
                NamedKey::F6,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, key)| named(&format!("F{}", index + 1), key))
            .collect(),
            [
                NamedKey::F7,
                NamedKey::F8,
                NamedKey::F9,
                NamedKey::F10,
                NamedKey::F11,
                NamedKey::F12,
            ]
            .into_iter()
            .enumerate()
            .map(|(index, key)| named(&format!("F{}", index + 7), key))
            .collect(),
        ];
    }
    let characters = |normal: &str, shifted: &str| {
        normal
            .chars()
            .zip(shifted.chars())
            .map(|(a, b)| KeyDefinition::character(a, b))
            .collect::<Vec<_>>()
    };
    let mut number = characters("`1234567890-=", "~!@#$%^&*()_+");
    number.push(KeyDefinition::special(
        "backspace",
        "Backspace",
        Named(NamedKey::Backspace),
        8,
    ));
    let mut upper = vec![KeyDefinition::special(
        "tab",
        "Tab",
        Named(NamedKey::Tab),
        6,
    )];
    upper.extend(characters("qwertyuiop[]", "QWERTYUIOP{}"));
    let mut slash = KeyDefinition::character('\\', '|');
    slash.quarters = 6;
    upper.push(slash);
    let mut home = vec![KeyDefinition::special("caps", "Caps Lock", CapsLock, 7)];
    home.extend(characters("asdfghjkl;'", "ASDFGHJKL:\""));
    home.push(KeyDefinition::special(
        "enter",
        "Enter",
        Named(NamedKey::Enter),
        9,
    ));
    let mut lower = vec![KeyDefinition::special(
        "shift-left",
        "Shift",
        Modifier(VirtualModifier::Shift),
        9,
    )];
    lower.extend(characters("zxcvbnm,./", "ZXCVBNM<>?"));
    lower.push(KeyDefinition::special(
        "shift-right",
        "Shift",
        Modifier(VirtualModifier::Shift),
        11,
    ));
    let mut space = KeyDefinition::character(' ', ' ');
    space.label = "Space".into();
    space.quarters = 24;
    let bottom = vec![
        KeyDefinition::special("control", "Ctrl", Modifier(VirtualModifier::Control), 6),
        KeyDefinition::special("super", "Windows", Modifier(VirtualModifier::Super), 6),
        KeyDefinition::special("alt", "Alt", Modifier(VirtualModifier::Alt), 6),
        space,
        KeyDefinition::special("alt-gr", "AltGr", Modifier(VirtualModifier::AltGr), 6),
        KeyDefinition::special(
            "navigation",
            "Navigation",
            Panel(KeyboardPanel::Navigation),
            12,
        ),
    ];
    vec![number, upper, home, lower, bottom]
}

/// Compact panels retain the same semantic key identities as the full keyboard.
pub fn compact_us_keyboard_rows(panel: KeyboardPanel) -> Vec<Vec<KeyDefinition>> {
    use crate::NamedKey;
    use KeyboardKey::{Modifier, Named, Panel};
    let character_row = |normal: &str, shifted: &str| {
        normal
            .chars()
            .zip(shifted.chars())
            .map(|(a, b)| KeyDefinition::character(a, b))
            .collect::<Vec<_>>()
    };
    let key = KeyDefinition::special;
    let mut space = KeyDefinition::character(' ', ' ');
    space.label = "Space".into();
    space.quarters = 16;
    let footer = vec![
        key("symbols", "123 / #+=", Panel(KeyboardPanel::Symbols), 8),
        key("letters", "ABC", Panel(KeyboardPanel::Letters), 8),
        space,
        key("enter", "Enter", Named(NamedKey::Enter), 8),
    ];
    match panel {
        KeyboardPanel::Letters => {
            let mut third = vec![key(
                "shift-left",
                "Shift",
                Modifier(VirtualModifier::Shift),
                6,
            )];
            third.extend(character_row("zxcvbnm", "ZXCVBNM"));
            third.push(key("backspace", "⌫", Named(NamedKey::Backspace), 6));
            vec![
                character_row("qwertyuiop", "QWERTYUIOP"),
                character_row("asdfghjkl", "ASDFGHJKL"),
                third,
                footer,
            ]
        }
        KeyboardPanel::Symbols => {
            let mut punctuation = character_row("[]\\;'`,./", "{}|:\"~<>?");
            punctuation.push(key("backspace", "⌫", Named(NamedKey::Backspace), 4));
            let mut operators = character_row("-=", "_+");
            operators.push(key(
                "shift-left",
                "Shift",
                Modifier(VirtualModifier::Shift),
                8,
            ));
            vec![
                character_row("1234567890", "!@#$%^&*()"),
                punctuation,
                operators,
                footer,
            ]
        }
        KeyboardPanel::Navigation => {
            let mut rows = us_keyboard_rows(panel);
            // Six function keys fit inside the same ten-unit row without undersized targets.
            for row in &mut rows[3..] {
                for key in row {
                    key.quarters = 6;
                }
            }
            rows.push(vec![
                key("control", "Ctrl", Modifier(VirtualModifier::Control), 6),
                key("super", "Win", Modifier(VirtualModifier::Super), 6),
                key("alt", "Alt", Modifier(VirtualModifier::Alt), 6),
                key("alt-gr", "AltGr", Modifier(VirtualModifier::AltGr), 6),
                key("caps", "Caps", KeyboardKey::CapsLock, 8),
                key("shift-left", "Shift", Modifier(VirtualModifier::Shift), 8),
            ]);
            rows
        }
    }
}

/// Display-only keyboard data for a plugin. Key meanings and recipient state stay in the host.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct KeyboardDisplayKey {
    pub id: String,
    pub label: String,
    pub quarters: u16,
    pub selected: bool,
    pub enabled: bool,
}

pub fn keyboard_display_rows(
    panel: KeyboardPanel,
    modifiers: VirtualModifiers,
    compact: bool,
    recipient_available: bool,
) -> Vec<Vec<KeyboardDisplayKey>> {
    keyboard_rows(panel, compact)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|definition| KeyboardDisplayKey {
                    selected: match definition.key {
                        KeyboardKey::Modifier(modifier) => modifiers.latch(modifier) != Latch::Off,
                        KeyboardKey::CapsLock => modifiers.caps_lock,
                        _ => false,
                    },
                    enabled: recipient_available || matches!(definition.key, KeyboardKey::Panel(_)),
                    label: definition.display_label(modifiers),
                    id: definition.id,
                    quarters: definition.quarters,
                })
                .collect()
        })
        .collect()
}

/// Resolve a displayed ID only against the host's current panel and density.
pub fn resolve_keyboard_display_key(
    panel: KeyboardPanel,
    compact: bool,
    id: &str,
) -> Option<KeyboardKey> {
    keyboard_rows(panel, compact)
        .into_iter()
        .flatten()
        .find(|definition| definition.id == id)
        .map(|definition| definition.key)
}

fn keyboard_rows(panel: KeyboardPanel, compact: bool) -> Vec<Vec<KeyDefinition>> {
    if compact {
        compact_us_keyboard_rows(panel)
    } else {
        us_keyboard_rows(panel)
    }
}

/// A full keyboard is bounded and centered; callers use compact mode below `minimum_width`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyboardMetrics {
    pub key: f32,
    pub gap: f32,
    pub width: f32,
}

impl KeyboardMetrics {
    pub const MINIMUM_WIDTH: f32 = 656.0;

    pub fn full(available_width: f32) -> Option<Self> {
        if !available_width.is_finite() || available_width < Self::MINIMUM_WIDTH {
            return None;
        }
        let gap = 4.0;
        let key = ((available_width - 14.0 * gap) / 15.0).clamp(40.0, 64.0);
        Some(Self {
            key,
            gap,
            width: 15.0 * key + 14.0 * gap,
        })
    }

    pub fn full_in(available_width: f32, available_height: f32, rows: usize) -> Option<Self> {
        let mut metrics = Self::full(available_width)?;
        if !available_height.is_finite() || rows == 0 {
            return None;
        }
        metrics.key = metrics
            .key
            .min((available_height - (rows - 1) as f32 * metrics.gap) / rows as f32);
        if metrics.key < 40.0 {
            return None;
        }
        metrics.width = 15.0 * metrics.key + 14.0 * metrics.gap;
        Some(metrics)
    }

    pub fn compact(available_width: f32, available_height: f32, rows: usize) -> Option<Self> {
        if !available_width.is_finite() || !available_height.is_finite() || rows == 0 {
            return None;
        }
        let gap = 4.0;
        let key = ((available_width - 9.0 * gap) / 10.0)
            .min((available_height - (rows - 1) as f32 * gap) / rows as f32)
            .min(48.0);
        if key < 40.0 {
            return None;
        }
        Some(Self {
            key,
            gap,
            width: key * 10.0 + gap * 9.0,
        })
    }

    pub fn key_width(self, quarters: u16) -> f32 {
        let units = f32::from(quarters) / 4.0;
        units * self.key + (units - 1.0) * self.gap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_layout_fits_720p_with_equal_rows_and_stable_unique_key_identities() {
        let metrics = KeyboardMetrics::full(1280.0 - 32.0).unwrap();
        let rows = us_keyboard_rows(KeyboardPanel::Letters);
        let mut ids = std::collections::BTreeSet::new();
        for row in &rows {
            assert_eq!(row.iter().map(|key| key.quarters).sum::<u16>(), 60);
            let width = row
                .iter()
                .map(|key| metrics.key_width(key.quarters))
                .sum::<f32>()
                + (row.len() - 1) as f32 * metrics.gap;
            assert!((width - metrics.width).abs() < 0.01);
            for key in row {
                assert!(ids.insert(key.id.clone()));
            }
        }
        assert!(metrics.width <= 1248.0);
        assert!(5.0 * metrics.key + 4.0 * metrics.gap + 64.0 < 420.0);
        assert_eq!(metrics.key_width(4), metrics.key);
        assert!(KeyboardMetrics::full(600.0).is_none());
    }

    #[test]
    fn shift_caps_and_shortcut_latches_have_distinct_lifetimes() {
        let mut modifiers = VirtualModifiers::default();
        modifiers.toggle(VirtualModifier::Shift, false);
        modifiers.toggle(VirtualModifier::Control, true);
        assert!(modifiers.shifted(true));
        modifiers.caps_lock = true;
        assert!(!modifiers.shifted(true));
        assert!(modifiers.shifted(false));
        modifiers.consume();
        assert!(modifiers.shifted(true));
        assert!(!modifiers.shifted(false));
        assert_eq!(modifiers.control, Latch::Locked);
        modifiers.toggle(VirtualModifier::Control, false);
        assert_eq!(modifiers.control, Latch::Off);
    }

    #[test]
    fn compact_panels_retain_all_full_keyboard_actions_with_usable_targets_at_double_scale() {
        let mut compact = Vec::new();
        for panel in [
            KeyboardPanel::Letters,
            KeyboardPanel::Symbols,
            KeyboardPanel::Navigation,
        ] {
            let rows = compact_us_keyboard_rows(panel);
            let metrics = KeyboardMetrics::compact(608.0, 280.0, rows.len()).unwrap();
            for row in rows {
                assert!(row.iter().map(|key| key.quarters).sum::<u16>() <= 40);
                assert!(
                    row.iter()
                        .all(|key| metrics.key_width(key.quarters) >= 40.0)
                );
                compact.extend(row.into_iter().map(|key| key.key));
            }
        }
        for panel in [KeyboardPanel::Letters, KeyboardPanel::Navigation] {
            for key in us_keyboard_rows(panel).into_iter().flatten() {
                // Panel navigation is supplied by the host header as well as key rows.
                if matches!(key.key, KeyboardKey::Panel(_)) {
                    continue;
                }
                assert!(
                    compact.contains(&key.key),
                    "missing compact action: {:?}",
                    key.key
                );
            }
        }
    }

    #[test]
    fn plugin_display_keys_resolve_only_in_the_current_host_layout() {
        let mut modifiers = VirtualModifiers::default();
        modifiers.toggle(VirtualModifier::Shift, false);
        let rows = keyboard_display_rows(KeyboardPanel::Letters, modifiers, true, false);
        let keys = rows.into_iter().flatten().collect::<Vec<_>>();
        let shift = keys.iter().find(|key| key.id == "osk-shift-left").unwrap();
        assert!(shift.selected);
        assert!(!shift.enabled);
        let letter = keys.iter().find(|key| key.id == "osk-char-113").unwrap();
        assert_eq!(letter.label, "Q");
        assert!(!letter.enabled);
        let symbols = keys.iter().find(|key| key.id == "osk-symbols").unwrap();
        assert!(symbols.enabled);
        assert_eq!(
            resolve_keyboard_display_key(KeyboardPanel::Letters, true, &letter.id),
            Some(KeyboardKey::Character {
                normal: 'q',
                shifted: 'Q'
            })
        );
        assert_eq!(
            resolve_keyboard_display_key(KeyboardPanel::Navigation, true, &letter.id),
            None
        );
        assert_eq!(
            resolve_keyboard_display_key(KeyboardPanel::Letters, true, "osk-unknown"),
            None
        );
        let serialized = serde_json::to_string(&keys).unwrap();
        assert!(!serialized.contains("normal"));
        assert!(!serialized.contains("shifted"));
    }
}
