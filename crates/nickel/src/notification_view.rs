use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Application, Button, ButtonLabel, Column, ComponentBuilderExt, Container, DragGesture,
    DragPhase, EffectEvidence, Insets, Point, Row, SemanticRole, Shortcut, Text, TextAlign, UiHost,
    VerticalScroll, ViewContext,
};

const DISMISS_DRAG_THRESHOLD: f32 = 96.0;
const DISMISS_ANIMATION: std::time::Duration = std::time::Duration::from_millis(180);
pub(crate) const NOTIFICATION_MAX_WIDTH: u32 = 420;
pub(crate) const NOTIFICATION_MAX_HEIGHT: u32 = 320;
const NOTIFICATION_PADDING: f32 = 20.0;
const NOTIFICATION_SECTION_GAP: f32 = 8.0;
const NOTIFICATION_HEADER_HEIGHT: f32 = 40.0;
const NOTIFICATION_TITLE_HEIGHT: f32 = 30.0;
const NOTIFICATION_ACTION_HEIGHT: f32 = 42.0;

use crate::notification::DesktopNotification;

#[derive(Clone, Debug, PartialEq)]
pub enum NotificationMessage {
    Invoke(String),
    Dismiss,
    Scroll(f32),
    ScrollBody(f32),
    Drag(DragGesture),
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
    body_offset: f32,
    palette: ThemePalette,
    effects: Vec<NotificationEffect>,
    effect_evidence: Vec<EffectEvidence>,
    failures: Vec<NotificationFailure>,
    dirty: bool,
    drag_start_x: Option<f32>,
    drag_offset: f32,
    dismiss_animation: Option<(std::time::Instant, f32)>,
    dismiss_target_width: f32,
    measurement_mode: bool,
}

impl NotificationApp {
    pub fn new(palette: ThemePalette) -> Self {
        Self {
            notification: None,
            history: Vec::new(),
            history_mode: false,
            history_offset: 0.0,
            body_offset: 0.0,
            palette,
            effects: Vec::new(),
            effect_evidence: Vec::new(),
            failures: Vec::new(),
            dirty: false,
            drag_start_x: None,
            drag_offset: 0.0,
            dismiss_animation: None,
            dismiss_target_width: 420.0,
            measurement_mode: false,
        }
    }

    pub fn sync(&mut self, notification: Option<&DesktopNotification>, palette: ThemePalette) {
        if self.notification.as_ref() != notification || self.palette != palette {
            // A revised request starts at its new scope rather than inheriting old scroll.
            self.body_offset = 0.0;
            self.notification = notification.cloned();
            self.drag_start_x = None;
            self.drag_offset = 0.0;
            self.dismiss_animation = None;
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
            NotificationMessage::ScrollBody(offset) => {
                self.body_offset = offset.max(0.0);
                self.dirty = true;
            }
            NotificationMessage::Drag(gesture) if !self.history_mode => match gesture.phase {
                DragPhase::Started => {
                    self.drag_start_x = Some(gesture.position.x);
                    self.dismiss_target_width = gesture.bounds.size.width.max(1.0);
                    self.dismiss_animation = None;
                }
                DragPhase::Moved => {
                    if let Some(start) = self.drag_start_x {
                        self.drag_offset = (gesture.position.x - start).max(0.0);
                        self.dirty = true;
                    }
                }
                DragPhase::Ended => {
                    self.drag_start_x = None;
                    if self.drag_offset >= DISMISS_DRAG_THRESHOLD {
                        self.dismiss_animation =
                            Some((std::time::Instant::now(), self.drag_offset));
                    } else {
                        self.drag_offset = 0.0;
                        self.request_dismiss();
                    }
                    self.dirty = true;
                }
                DragPhase::Cancelled => {
                    self.drag_start_x = None;
                    self.drag_offset = 0.0;
                    self.dirty = true;
                }
            },
            NotificationMessage::Drag(_) => {}
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

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        nickel_ui::ShortcutOutcome::from_changed(
            if shortcut == Shortcut::Escape && self.history_mode {
                self.effects.push(NotificationEffect::CloseHistory);
                true
            } else if shortcut == Shortcut::Escape && self.notification.is_some() {
                self.request_dismiss();
                true
            } else {
                false
            },
        )
    }

    fn poll(&mut self) -> bool {
        if let Some((started, initial)) = self.dismiss_animation {
            let progress =
                (started.elapsed().as_secs_f32() / DISMISS_ANIMATION.as_secs_f32()).clamp(0.0, 1.0);
            self.drag_offset = initial + (self.viewport_width() + 24.0 - initial) * progress;
            self.dirty = true;
            if progress >= 1.0 {
                self.dismiss_animation = None;
                self.request_dismiss();
            }
        }
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
                        .theme(self.palette.into())
                        .on_scroll(NotificationMessage::Scroll)
                        .height((context.viewport.size.height - 24.0).max(1.0))
                        .child(
                            Column::new()
                                .gap(8.0)
                                .child(
                                    Text::new(nickel_i18n::system_text(
                                        "ui-notification-view-notifications",
                                    ))
                                    .color(self.palette.text),
                                )
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
        let action_count = notification.actions.len();
        let gap = 8.0;
        let button_width = if action_count == 0 {
            0.0
        } else {
            ((context.viewport.size.width
                - NOTIFICATION_PADDING * 2.0
                - gap * action_count.saturating_sub(1) as f32)
                / action_count as f32)
                .max(1.0)
        };
        let actions = AnyView::new(Row::new().id("notification-actions").gap(gap).children(
            notification.actions.iter().map(|action| {
                Button::new(
                    NotificationMessage::Invoke(action.key.clone()),
                    action.label.clone(),
                )
                .id(format!("notification-action-{}", action.key))
                .width(button_width)
                .height(NOTIFICATION_ACTION_HEIGHT)
                .padding(Insets::all(8.0))
                .background(self.palette.surface_hover)
                .focus_background_tint(self.palette.accent)
                .controller_focus_background_tint(self.palette.accent)
                .radius(8.0)
                .color(self.palette.text)
                .label_align(TextAlign::Center)
            }),
        ));
        let action_height = if action_count == 0 {
            0.0
        } else {
            NOTIFICATION_ACTION_HEIGHT + NOTIFICATION_SECTION_GAP
        };
        let body_height = (context.viewport.size.height
            - NOTIFICATION_PADDING * 2.0
            - NOTIFICATION_HEADER_HEIGHT
            - NOTIFICATION_TITLE_HEIGHT
            - action_height
            - NOTIFICATION_SECTION_GAP * 2.0)
            .max(1.0);
        let body = if self.measurement_mode {
            AnyView::new(
                Text::new(&notification.body)
                    .scale(1.7)
                    .color(self.palette.muted)
                    .wrap(true)
                    .max_height(body_height),
            )
        } else {
            AnyView::new(
                VerticalScroll::new(
                    NotificationMessage::ScrollBody(self.body_offset),
                    self.body_offset,
                )
                .theme(self.palette.into())
                .on_scroll(NotificationMessage::ScrollBody)
                .max_height(body_height)
                .child(
                    Text::new(&notification.body)
                        .scale(1.7)
                        .color(self.palette.muted)
                        .wrap(true),
                ),
            )
        };
        let mut content = Column::new()
            .gap(NOTIFICATION_SECTION_GAP)
            .child(
                Row::new()
                    .id("notification-header")
                    .gap(10.0)
                    .height(NOTIFICATION_HEADER_HEIGHT)
                    .child(
                        Container::new()
                            .width(24.0)
                            .height(24.0)
                            .background(self.palette.accent)
                            .radius(6.0),
                    )
                    .child(
                        Text::new(&notification.app_name)
                            .scale(1.55)
                            .color(self.palette.text)
                            .grow(1.0),
                    )
                    .child(
                        Button::with_label(
                            NotificationMessage::Dismiss,
                            ButtonLabel::new("✕").scale(2.1),
                        )
                        .id("notification-close")
                        .width(46.0)
                        .height(NOTIFICATION_HEADER_HEIGHT)
                        .background(self.palette.panel)
                        .focus_background_tint(self.palette.surface_hover)
                        .controller_focus_background_tint(self.palette.surface_hover)
                        .radius(8.0)
                        .color(self.palette.muted)
                        .label_align(TextAlign::Center)
                        .center_label_vertically()
                        .accessibility_label("Dismiss"),
                    ),
            )
            .child(
                Text::new(heading)
                    .id("notification-title")
                    .height(NOTIFICATION_TITLE_HEIGHT)
                    .scale(1.7)
                    .color(self.palette.text)
                    .bold(true),
            )
            .child(body);
        if action_count > 0 {
            content = content.child(actions);
        }
        content = content.child(
            Container::new()
                .semantic_role(SemanticRole::Group)
                .accessibility_label("Notification content end")
                .height(1.0),
        );
        AnyView::new(
            Container::new()
                .id("notification")
                .semantic_role(SemanticRole::Dialog)
                .accessibility_label(heading)
                .width(context.viewport.size.width)
                .max_height(context.viewport.size.height)
                .background(self.palette.panel)
                .border(self.palette.surface_hover, 1.0)
                .radius(14.0)
                .padding(Insets::all(NOTIFICATION_PADDING))
                .position(Point {
                    x: self.drag_offset,
                    y: 0.0,
                })
                .on_drag((NotificationMessage::Dismiss, |_, gesture| {
                    NotificationMessage::Drag(gesture)
                }))
                .child(content),
        )
    }
}

impl NotificationApp {
    fn viewport_width(&self) -> f32 {
        self.dismiss_target_width
    }
}

pub type NotificationHost = UiHost<NotificationApp>;

pub(crate) fn preferred_notification_surface_size(
    notification: &DesktopNotification,
    _palette: ThemePalette,
    maximum: (u32, u32),
) -> (u32, u32) {
    let width = NOTIFICATION_MAX_WIDTH.min(maximum.0).max(1);
    let height = NOTIFICATION_MAX_HEIGHT.min(maximum.1).max(1);
    let mut application = NotificationApp::new(_palette);
    application.sync(Some(notification), _palette);
    application.measurement_mode = true;
    let mut host = NotificationHost::new(application, width, height);
    host.poll();
    let measured = host
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Group,
            name: "Notification content end".into(),
        })
        .map(|node| node.bounds.origin.y + node.bounds.size.height)
        .map(|bottom| (bottom + NOTIFICATION_PADDING).ceil() as u32)
        .unwrap_or(height);
    (width, measured.clamp(1, height))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nickel_core::theme::{Appearance, ThemePalette};
    use nickel_ui::{
        ActionKind, Application, HostBatch, HostEvent, SemanticAction, SemanticRole,
        SemanticSelector, Shortcut,
    };

    use super::{
        NOTIFICATION_MAX_HEIGHT, NOTIFICATION_MAX_WIDTH, NotificationApp, NotificationEffect,
        NotificationFailure, NotificationHost, NotificationMessage,
        preferred_notification_surface_size,
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
    fn long_consent_body_scrolls_without_moving_decision_buttons() {
        let mut host = host();
        let palette = ThemePalette::from_appearance(Appearance::default());
        let mut request = host.application().notification.clone().unwrap();
        request.body = "Inspect the requested command and scope before approving. ".repeat(80);
        host.application_mut().sync(Some(&request), palette);
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        let button = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap()
            .bounds;
        assert!(button.origin.y + button.size.height <= 180.0);

        host.application_mut()
            .update(NotificationMessage::ScrollBody(140.0));
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        assert_eq!(host.application().body_offset, 140.0);
        let scrolled_button = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap()
            .bounds;
        assert_eq!(button, scrolled_button);

        request.body = "Revised scope".into();
        host.application_mut().sync(Some(&request), palette);
        assert_eq!(host.application().body_offset, 0.0);
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
    fn semantic_activation_emits_typed_effects() {
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
    }

    #[test]
    fn keyboard_and_accessibility_activation_emit_typed_effects() {
        let mut keyboard = host();
        let open = keyboard
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        keyboard.request_focus(open.id);
        keyboard.step(HostBatch {
            events: vec![HostEvent::Ui(nickel_ui::UiEvent::KeyboardActivate)],
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
        let close = keyboard
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Dismiss".into(),
            })
            .unwrap();
        keyboard.request_focus(close.id);
        keyboard.step(HostBatch {
            events: vec![HostEvent::Ui(nickel_ui::UiEvent::KeyboardActivate)],
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
    }

    #[test]
    fn pointer_close_dispatches_dismiss_effect() {
        let mut host = host();
        let close = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Dismiss".into(),
            })
            .unwrap();
        let point = nickel_ui::Point {
            x: close.bounds.origin.x + close.bounds.size.width / 2.0,
            y: close.bounds.origin.y + close.bounds.size.height / 2.0,
        };
        host.step(HostBatch {
            events: vec![
                HostEvent::Ui(nickel_ui::UiEvent::PointerPressed(point)),
                HostEvent::Ui(nickel_ui::UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );
    }

    #[test]
    fn pointer_body_tap_dispatches_dismiss_effect() {
        let mut host = host();
        let dialog = host
            .query_unique(&SemanticSelector::Role(SemanticRole::Dialog))
            .unwrap();
        let point = nickel_ui::Point {
            x: dialog.bounds.origin.x + 30.0,
            y: dialog.bounds.origin.y + 100.0,
        };
        host.step(HostBatch {
            events: vec![
                HostEvent::Ui(nickel_ui::UiEvent::PointerPressed(point)),
                HostEvent::Ui(nickel_ui::UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![NotificationEffect::Dismiss { notification_id: 1 }]
        );
    }

    #[test]
    fn preferred_surface_uses_resolved_content_height_with_design_limits() {
        let palette = ThemePalette::from_appearance(Appearance::default());
        let short = host().application().notification.clone().unwrap();
        let short_size = preferred_notification_surface_size(&short, palette, (1920, 1080));

        let mut long = short.clone();
        long.body = "A wrapped notification body. ".repeat(100);
        let long_size = preferred_notification_surface_size(&long, palette, (1920, 1080));

        assert_eq!(short_size.0, NOTIFICATION_MAX_WIDTH);
        assert_eq!(long_size.0, NOTIFICATION_MAX_WIDTH);
        assert!(short_size.1 < long_size.1);
        assert!(long_size.1 <= NOTIFICATION_MAX_HEIGHT);
    }
}
