//! Shared shell keyboard surface. Native delivery stays in the owning shell adapter.

use nickel_core::on_screen_keyboard::{
    KeyboardKey, KeyboardMetrics, KeyboardPanel, Latch, VirtualModifiers, compact_us_keyboard_rows,
    us_keyboard_rows,
};
use nickel_core::theme::ThemePalette;

use crate::{
    Align, Application, Button, ButtonPresentation, Column, Container, Insets, Justify, Row,
    SemanticTheme, SemanticTokenSet, Shortcut, Spacer, Text, ViewContext,
};

#[derive(Clone, Debug, PartialEq)]
pub enum KeyboardMessage {
    Key(KeyboardKey),
    PersistentModifiers,
    ToggleDock,
    Hide,
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeyboardEffect {
    ToggleDock,
    Input {
        key: KeyboardKey,
        modifiers: VirtualModifiers,
    },
    Hide,
}

pub struct KeyboardApp {
    top_docked: bool,
    panel: KeyboardPanel,
    modifiers: VirtualModifiers,
    persistent_modifiers: bool,
    palette: ThemePalette,
    recipient_available: bool,
    effects: Vec<KeyboardEffect>,
    dirty: bool,
}

impl KeyboardApp {
    pub fn new(palette: ThemePalette) -> Self {
        Self {
            top_docked: false,
            panel: KeyboardPanel::Letters,
            modifiers: VirtualModifiers::default(),
            persistent_modifiers: false,
            palette,
            recipient_available: false,
            effects: Vec::new(),
            dirty: false,
        }
    }

    /// A recipient transition clears virtual state without touching physical-key state.
    pub fn recipient_changed(&mut self, available: bool) {
        self.recipient_available = available;
        self.modifiers = VirtualModifiers::default();
        self.persistent_modifiers = false;
        self.effects.clear();
        self.dirty = true;
    }

    pub fn set_top_docked(&mut self, top: bool) {
        if self.top_docked != top {
            self.top_docked = top;
            self.dirty = true;
        }
    }

    pub fn take_effects(&mut self) -> Vec<KeyboardEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn set_palette(&mut self, palette: ThemePalette) {
        if self.palette != palette {
            self.palette = palette;
            self.dirty = true;
        }
    }

    fn theme(&self) -> SemanticTheme {
        let p = self.palette;
        SemanticTheme::from_tokens(SemanticTokenSet::standard(
            p.background,
            p.panel,
            p.surface,
            p.surface_hover,
            p.surface_hover,
            p.text,
            p.muted,
            p.accent,
            p.accent_soft,
            p.complement,
            p.complement,
        ))
    }
}

impl Application for KeyboardApp {
    type Message = KeyboardMessage;

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn update(&mut self, message: Self::Message) {
        match message {
            KeyboardMessage::ToggleDock => self.effects.push(KeyboardEffect::ToggleDock),
            KeyboardMessage::Hide => {
                self.modifiers = VirtualModifiers::default();
                self.persistent_modifiers = false;
                self.effects.clear();
                self.effects.push(KeyboardEffect::Hide);
            }
            KeyboardMessage::PersistentModifiers => {
                self.persistent_modifiers = !self.persistent_modifiers
            }
            KeyboardMessage::Key(KeyboardKey::Panel(panel)) => self.panel = panel,
            KeyboardMessage::Key(key) if self.recipient_available => match key {
                KeyboardKey::Modifier(modifier) => {
                    self.modifiers.toggle(modifier, self.persistent_modifiers)
                }
                KeyboardKey::CapsLock => self.modifiers.caps_lock = !self.modifiers.caps_lock,
                _ => {
                    self.effects.push(KeyboardEffect::Input {
                        key,
                        modifiers: self.modifiers,
                    });
                    self.modifiers.consume();
                }
            },
            KeyboardMessage::Key(_) => {}
        }
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        if shortcut == Shortcut::Escape {
            self.update(KeyboardMessage::Hide);
            true
        } else {
            false
        }
    }

    fn title(&self) -> &str {
        "On-screen keyboard — Nickel"
    }

    fn initial_size(&self) -> (u32, u32) {
        (1056, 420)
    }

    fn view(&self, context: ViewContext) -> impl crate::View<Self::Message> {
        let theme = self.theme();
        let available = (context.viewport.size.width - 32.0).max(0.0);
        let compact =
            available < KeyboardMetrics::MINIMUM_WIDTH || context.viewport.size.height < 400.0;
        let rows = if compact {
            compact_us_keyboard_rows(self.panel)
        } else {
            us_keyboard_rows(self.panel)
        };
        let metrics = if compact {
            KeyboardMetrics::compact(available, context.viewport.size.height - 80.0, rows.len())
        } else {
            KeyboardMetrics::full(available)
        };
        let width = metrics.map_or(available, |m| m.width);
        let button = |message, label: String, selected| {
            Button::semantic(
                theme,
                message,
                label,
                if selected {
                    ButtonPresentation::Primary
                } else {
                    ButtonPresentation::Secondary
                },
            )
            .height(40.0)
        };
        let header = Row::new()
            .width(width)
            .height(44.0)
            .gap(8.0)
            .align_items(Align::Center)
            .child(Text::new(if self.recipient_available {
                if compact { "US" } else { "English (US)" }
            } else {
                "Select a text field"
            }))
            .child(Spacer::flex())
            .child(
                button(
                    KeyboardMessage::PersistentModifiers,
                    if compact { "Hold" } else { "Hold modifiers" }.into(),
                    self.persistent_modifiers,
                )
                .id("osk-hold-modifiers"),
            )
            .child(
                button(
                    KeyboardMessage::Key(KeyboardKey::Panel(
                        if self.panel != KeyboardPanel::Navigation {
                            KeyboardPanel::Navigation
                        } else {
                            KeyboardPanel::Letters
                        },
                    )),
                    if self.panel != KeyboardPanel::Navigation {
                        if compact { "Nav" } else { "Navigation" }
                    } else {
                        "ABC"
                    }
                    .into(),
                    false,
                )
                .id("osk-panel"),
            )
            .child(
                button(
                    KeyboardMessage::ToggleDock,
                    if self.top_docked {
                        "Move down"
                    } else {
                        "Move up"
                    }
                    .into(),
                    false,
                )
                .id("osk-dock"),
            )
            .child(button(KeyboardMessage::Hide, "Hide".into(), false).id("osk-hide"));
        let mut content = Column::new().width(width).gap(4.0).child(header);
        if let Some(metrics) = metrics {
            for row in rows {
                let mut key_row = Row::new()
                    .width(width)
                    .height(metrics.key)
                    .gap(metrics.gap)
                    .justify_content(Justify::Center);
                for key in row {
                    let selected = match key.key {
                        KeyboardKey::Modifier(modifier) => {
                            self.modifiers.latch(modifier) != Latch::Off
                        }
                        KeyboardKey::CapsLock => self.modifiers.caps_lock,
                        _ => false,
                    };
                    let label = key.display_label(self.modifiers);
                    key_row = key_row.child(
                        button(KeyboardMessage::Key(key.key), label, selected)
                            .id(key.id)
                            .width(metrics.key_width(key.quarters))
                            .height(metrics.key)
                            .padding(Insets::all(4.0))
                            .center_label_vertically()
                            .radius(6.0)
                            .enabled(
                                self.recipient_available
                                    || matches!(key.key, KeyboardKey::Panel(_)),
                            ),
                    );
                }
                content = content.child(key_row);
            }
        } else {
            content = content.child(Text::new("Not enough space for usable keys. Increase the available window area or reduce display scaling."));
        }
        Container::new()
            .background(self.palette.panel)
            .padding(Insets::all(16.0))
            .child(
                Row::new()
                    .child(Spacer::flex())
                    .child(content)
                    .child(Spacer::flex()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, SemanticAction, UiHost};

    #[test]
    fn compact_panels_switch_through_host_and_keep_every_key_inside_double_scale_viewport() {
        let mut app = KeyboardApp::new(ThemePalette::from_appearance(Default::default()));
        app.recipient_changed(true);
        let mut host = UiHost::new(app, 640, 360);
        for panel in [
            KeyboardPanel::Symbols,
            KeyboardPanel::Letters,
            KeyboardPanel::Navigation,
        ] {
            let targets =
                host.semantic_targets_for_message(&KeyboardMessage::Key(KeyboardKey::Panel(panel)));
            let target = targets.first().expect("panel has a visible semantic entry");
            let outcome = host.perform_controller_semantic_action(
                target.id.clone(),
                SemanticAction::Invoke(ActionKind::Activate),
            );
            assert!(outcome.semantic_failures.is_empty());
            assert_eq!(host.application().panel, panel);
            for node in host
                .semantic_nodes()
                .into_iter()
                .filter(|node| node.role == Some(crate::SemanticRole::Button))
            {
                assert!(node.bounds.origin.x >= 0.0 && node.bounds.origin.y >= 0.0);
                assert!(node.bounds.origin.x + node.bounds.size.width <= 640.0);
                assert!(node.bounds.origin.y + node.bounds.size.height <= 360.0);
                assert!(node.bounds.size.width >= 40.0 && node.bounds.size.height >= 40.0);
            }
        }
    }

    #[test]
    fn semantic_keys_emit_one_effect_and_dismissal_discards_pending_input() {
        let mut app = KeyboardApp::new(ThemePalette::from_appearance(Default::default()));
        app.recipient_changed(true);
        let mut host = UiHost::new(app, 1280, 420);
        let key = host
            .unique_semantic_target_for_message(&KeyboardMessage::Key(KeyboardKey::Character {
                normal: 'a',
                shifted: 'A',
            }))
            .unwrap();
        host.perform_semantic_action(key.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(host.application_mut().take_effects().len(), 1);
        host.shortcut(Shortcut::Escape);
        assert_eq!(
            host.application_mut().take_effects(),
            vec![KeyboardEffect::Hide]
        );
    }

    #[test]
    fn keyboard_keys_fit_below_the_header_without_overlapping_at_1280_width() {
        let mut app = KeyboardApp::new(ThemePalette::from_appearance(Default::default()));
        app.recipient_changed(true);
        let host = UiHost::new(app, 1280, 420);
        let keys = host
            .semantic_nodes()
            .into_iter()
            .filter(|node| {
                node.role == Some(crate::SemanticRole::Button) && node.bounds.origin.y >= 60.0
            })
            .collect::<Vec<_>>();
        assert_eq!(
            keys.len(),
            us_keyboard_rows(KeyboardPanel::Letters)
                .iter()
                .map(Vec::len)
                .sum::<usize>()
        );
        for (index, key) in keys.iter().enumerate() {
            assert!(key.bounds.origin.x >= 0.0);
            assert!(key.bounds.origin.y + key.bounds.size.height <= 420.0);
            assert!(key.bounds.origin.x + key.bounds.size.width <= 1280.0);
            assert!(key.bounds.size.height >= 40.0);
            for other in keys.iter().skip(index + 1) {
                let a = key.bounds;
                let b = other.bounds;
                assert!(
                    a.origin.x + a.size.width <= b.origin.x
                        || b.origin.x + b.size.width <= a.origin.x
                        || a.origin.y + a.size.height <= b.origin.y
                        || b.origin.y + b.size.height <= a.origin.y
                );
            }
        }
    }
}
