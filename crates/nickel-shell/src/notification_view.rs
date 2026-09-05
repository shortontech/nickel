use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Application, Button, Column, Container, EffectEvidence, Insets, Row, SemanticRole,
    Shortcut, Text, TextAlign, UiHost, VerticalScroll, ViewContext,
};

use crate::notification::DesktopNotification;

#[derive(Clone, Debug, PartialEq)]
pub enum NotificationMessage {
    Invoke(String),
    Dismiss,
    Scroll(f32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotificationEffect {
    Invoke { notification_id: u32, key: String },
    Dismiss { notification_id: u32 },
    CloseHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotificationFailure {
    UnknownAction { notification_id: u32, key: String },
}

pub struct NotificationApp {
    notification: Option<DesktopNotification>,
    history: Vec<DesktopNotification>,
    history_mode: bool,
    history_offset: f32,
    palette: ThemePalette,
    effects: Vec<NotificationEffect>,
    effect_evidence: Vec<EffectEvidence>,
    failures: Vec<NotificationFailure>,
    dirty: bool,
}

impl NotificationApp {
    pub fn new(palette: ThemePalette) -> Self {
        Self {
            notification: None,
            history: Vec::new(),
            history_mode: false,
            history_offset: 0.0,
            palette,
            effects: Vec::new(),
            effect_evidence: Vec::new(),
            failures: Vec::new(),
            dirty: false,
        }
    }

    pub fn sync(&mut self, notification: Option<&DesktopNotification>, palette: ThemePalette) {
        if self.notification.as_ref() != notification || self.palette != palette {
            self.notification = notification.cloned();
            self.palette = palette;
            self.dirty = true;
        }
        self.history_mode = false;
    }

    pub fn sync_history(&mut self, history: &[DesktopNotification], palette: ThemePalette) {
        if self.history != history || self.palette != palette || !self.history_mode {
            self.history = history.to_vec();
            self.notification = history.first().cloned();
            self.palette = palette;
            self.history_mode = true;
            self.dirty = true;
        }
    }

    pub fn request_dismiss(&mut self) {
        if let Some(notification) = &self.notification {
            self.effects.push(NotificationEffect::Dismiss {
                notification_id: notification.id,
            });
            self.effect_evidence.push(EffectEvidence {
                type_name: std::any::type_name::<NotificationEffect>(),
                label: Some("dismiss".into()),
            });
        }
    }

    pub fn take_effects(&mut self) -> Vec<NotificationEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn take_failures(&mut self) -> Vec<NotificationFailure> {
        std::mem::take(&mut self.failures)
    }
}

impl Application for NotificationApp {
    type Message = NotificationMessage;

    fn update(&mut self, message: Self::Message) {
        let Some(notification) = &self.notification else {
            return;
        };
        match message {
            NotificationMessage::Scroll(offset) => {
                self.history_offset = offset.max(0.0);
                self.dirty = true;
            }
            NotificationMessage::Invoke(key) => {
                if notification.actions.iter().any(|action| action.key == key) {
                    self.effects.push(NotificationEffect::Invoke {
                        notification_id: notification.id,
                        key,
                    });
                    self.effect_evidence.push(EffectEvidence {
                        type_name: std::any::type_name::<NotificationEffect>(),
                        label: Some("invoke".into()),
                    });
                } else {
                    self.failures.push(NotificationFailure::UnknownAction {
                        notification_id: notification.id,
                        key,
                    });
                }
            }
            NotificationMessage::Dismiss => self.request_dismiss(),
        }
    }

    fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
        std::mem::take(&mut self.effect_evidence)
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        if shortcut == Shortcut::Escape && self.history_mode {
            self.effects.push(NotificationEffect::CloseHistory);
            true
        } else if shortcut == Shortcut::Escape && self.notification.is_some() {
            self.request_dismiss();
            true
        } else {
            false
        }
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        if self.history_mode {
            let entries = self.history.iter().map(|notification| {
                let heading = if notification.summary.trim().is_empty() {
                    &notification.app_name
                } else {
                    &notification.summary
                };
                Container::new()
                    .padding(Insets::all(10.0))
                    .background(self.palette.surface)
                    .radius(8.0)
                    .child(
                        Column::new()
                            .gap(4.0)
                            .child(Text::new(heading).color(self.palette.text))
                            .child(Text::new(&notification.body).color(self.palette.muted)),
                    )
            });
            return AnyView::new(
                Container::new()
                    .id("notification-history")
                    .semantic_role(SemanticRole::Dialog)
                    .accessibility_label("Notification history")
                    .width(context.viewport.size.width)
                    .height(context.viewport.size.height)
                    .padding(Insets::all(12.0))
                    .background(self.palette.panel)
                    .border(self.palette.surface_hover, 1.0)
                    .radius(16.0)
                    .child(
                        VerticalScroll::new(
                            NotificationMessage::Scroll(self.history_offset),
                            self.history_offset,
                        )
                        .on_scroll(NotificationMessage::Scroll)
                        .height((context.viewport.size.height - 24.0).max(1.0))
                        .child(
                            Column::new()
                                .gap(8.0)
                                .child(Text::new("Notifications").color(self.palette.text))
                                .children(entries),
                        ),
                    ),
            );
        }
        let Some(notification) = &self.notification else {
            return AnyView::new(Container::new());
        };
        let heading = if notification.summary.trim().is_empty() {
            &notification.app_name
        } else {
            &notification.summary
        };
        let action_count = notification.actions.len() + 1;
        let gap = 8.0;
        let button_width = if action_count == 0 {
            0.0
        } else {
            ((context.viewport.size.width - 40.0 - gap * action_count.saturating_sub(1) as f32)
                / action_count as f32)
                .max(1.0)
        };
        let actions = Row::new()
            .gap(gap)
            .children(notification.actions.iter().map(|action| {
                Button::new(
                    NotificationMessage::Invoke(action.key.clone()),
                    action.label.clone(),
                )
                .id(format!("notification-action-{}", action.key))
                .width(button_width)
                .height(30.0)
                .padding(Insets::all(5.0))
                .background(self.palette.surface_hover)
                .focus_background_tint(self.palette.accent)
                .controller_focus_background_tint(self.palette.accent)
                .radius(7.0)
                .color(self.palette.text)
                .label_align(TextAlign::Center)
            }))
            .child(
                Button::new(NotificationMessage::Dismiss, "Dismiss")
                    .id("notification-dismiss")
                    .width(button_width)
                    .height(30.0)
                    .padding(Insets::all(5.0))
                    .background(self.palette.surface_hover)
                    .focus_background_tint(self.palette.accent)
                    .controller_focus_background_tint(self.palette.accent)
                    .radius(7.0)
                    .color(self.palette.text)
                    .label_align(TextAlign::Center),
            );
        AnyView::new(
            Container::new()
                .id("notification")
                .semantic_role(SemanticRole::Dialog)
                .accessibility_label(heading)
                .width(context.viewport.size.width)
                .height(context.viewport.size.height)
                .background(self.palette.panel)
                .border(self.palette.surface_hover, 1.0)
                .radius(16.0)
                .padding(Insets {
                    top: 18.0,
                    right: 20.0,
                    bottom: 16.0,
                    left: 20.0,
                })
                .child(
                    Column::new()
                        .gap(5.0)
                        .child(
                            Text::new(heading)
                                .height(32.0)
                                .scale(20.0)
                                .color(self.palette.text)
                                .bold(true),
                        )
                        .child(
                            Text::new(&notification.body)
                                .height((context.viewport.size.height - 116.0).max(1.0))
                                .scale(16.0)
                                .color(self.palette.muted)
                                .wrap(true),
                        )
                        .child(actions),
                ),
        )
    }
}

pub type NotificationHost = UiHost<NotificationApp>;

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nickel_core::theme::{Appearance, ThemePalette};
    use nickel_ui::{
        ActionKind, Application, ControllerAction, HostBatch, HostEvent, SemanticAction,
        SemanticRole, SemanticSelector, Shortcut,
    };

    use super::{
        NotificationApp, NotificationEffect, NotificationFailure, NotificationHost,
        NotificationMessage,
    };
    use crate::notification::{NotificationAction, NotificationRequest, NotificationStore};

    fn host() -> NotificationHost {
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Test".into(),
                summary: "Ready".into(),
                body: "Choose an action".into(),
                actions: vec![
                    NotificationAction {
                        key: "open".into(),
                        label: "Open".into(),
                    },
                    NotificationAction {
                        key: "later".into(),
                        label: "Later".into(),
                    },
                ],
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        let palette = ThemePalette::from_appearance(Appearance::default());
        let mut app = NotificationApp::new(palette);
        let notification = store.newest().unwrap();
        app.sync(Some(&notification), palette);
        let mut host = NotificationHost::new(app, 420, 180);
        host.poll();
        host
    }

    #[test]
    fn notification_semantics_and_accessibility_come_from_the_host_frame() {
        let host = host();
        assert_eq!(
            host.query(&SemanticSelector::Role(SemanticRole::Dialog))
                .len(),
            1
        );
        assert_eq!(
            host.query(&SemanticSelector::Role(SemanticRole::Button))
                .len(),
            3
        );
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("dialog") && node.label.as_deref() == Some("Ready")
        }));
        assert!(
            host.accessibility_nodes()
                .iter()
                .any(|node| node.label.as_deref() == Some("Open"))
        );
        assert!(
            host.accessibility_nodes()
                .iter()
                .any(|node| node.label.as_deref() == Some("Dismiss"))
        );
    }

    #[test]
    fn semantic_and_controller_activation_emit_typed_effects() {
        let mut semantic = host();
        let open = semantic
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        semantic.perform_semantic_action(open.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(
            semantic.application_mut().take_effects(),
            vec![NotificationEffect::Invoke {
                notification_id: 1,
                key: "open".into(),
            }]
        );

        let mut controller = host();
        controller.handle_controller_action(ControllerAction::Right);
        controller.handle_controller_action(ControllerAction::Confirm);
        assert!(matches!(
            controller.application_mut().take_effects().as_slice(),
            [NotificationEffect::Invoke {
                notification_id: 1,
                ..
            }]
        ));
    }

    #[test]
    fn keyboard_and_accessibility_activation_emit_typed_effects() {
        let mut keyboard = host();
        keyboard.step(HostBatch {
            events: vec![
                HostEvent::Ui(nickel_ui::UiEvent::FocusNext),
                HostEvent::Ui(nickel_ui::UiEvent::KeyboardActivate),
            ],
            ..HostBatch::default()
        });
        assert_eq!(
            keyboard.application_mut().take_effects(),
            vec![NotificationEffect::Invoke {
                notification_id: 1,
                key: "open".into(),
            }]
        );

        let mut accessibility = host();
        let open = accessibility
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        accessibility.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target: open.id,
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            accessibility.application_mut().take_effects(),
            vec![NotificationEffect::Invoke {
                notification_id: 1,
                key: "open".into(),
            }]
        );
    }

    #[test]
    fn dismissal_and_invalid_actions_are_typed() {
        let mut host = host();
        assert!(
            host.step(HostBatch {
                events: vec![HostEvent::Shortcut(Shortcut::Escape)],
                ..HostBatch::default()
            })
            .changed
        );
        assert_eq!(
            host.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );

        host.application_mut()
            .update(NotificationMessage::Invoke("missing".into()));
        assert_eq!(
            host.application_mut().take_failures(),
            vec![NotificationFailure::UnknownAction {
                notification_id: 1,
                key: "missing".into(),
            }]
        );
    }

    #[test]
    fn keyboard_and_accessibility_dismissal_emit_typed_effects() {
        let mut keyboard = host();
        keyboard.step(HostBatch {
            events: vec![
                HostEvent::Ui(nickel_ui::UiEvent::FocusNext),
                HostEvent::Ui(nickel_ui::UiEvent::FocusNext),
                HostEvent::Ui(nickel_ui::UiEvent::FocusNext),
                HostEvent::Ui(nickel_ui::UiEvent::KeyboardActivate),
            ],
            ..HostBatch::default()
        });
        assert_eq!(
            keyboard.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );

        let mut accessibility = host();
        let dismiss = accessibility
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Dismiss".into(),
            })
            .unwrap();
        accessibility.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target: dismiss.id,
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..HostBatch::default()
        });
        assert_eq!(
            accessibility.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );

        let mut controller = host();
        controller.step(HostBatch {
            events: vec![
                HostEvent::Controller(ControllerAction::Right),
                HostEvent::Controller(ControllerAction::Right),
                HostEvent::Controller(ControllerAction::Right),
                HostEvent::Controller(ControllerAction::Confirm),
            ],
            ..HostBatch::default()
        });
        assert_eq!(
            controller.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );
    }
}
