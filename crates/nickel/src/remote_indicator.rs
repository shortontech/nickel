//! Shared production UI for trusted, locally operated remote-control indication.
//!
//! The platform owner must protect placement, capture and input routing. This
//! module supplies the semantic tree; the Windows owner projects it through a
//! dedicated AccessKit UI Automation adapter on its trusted indicator window.
use nickel_ui::{
    Application, Button, ButtonPresentation, Column, Component, Insets, Row, SemanticTheme, Text,
    VerticalScroll, View, ViewContext,
};

#[derive(Clone, PartialEq)]
pub(crate) struct IndicatorGrant {
    pub suspended: bool,
    pub connected: bool,
    pub id: u64,
    pub client: String,
    pub scope: String,
    pub remaining: String,
    pub peer: String,
}

pub(crate) fn indicator_height(leases: usize, output_height: u32) -> u32 {
    let requested = if leases == 1 { 260 } else { 356 };
    requested.min(output_height.saturating_sub(24).max(1))
}

/// Keep the production UI at its logical size on high-DPI outputs, while
/// bounding the actual native window to the selected output's physical extent.
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn indicator_physical_size(
    leases: usize,
    width: u32,
    height: u32,
    scale: f32,
) -> (u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let logical_width = ((width as f32 / scale).floor() as u32).clamp(1, 420);
    let logical_height = indicator_height(leases, (height as f32 / scale).floor() as u32);
    (
        ((logical_width as f32 * scale).round() as u32).clamp(1, width.max(1)),
        ((logical_height as f32 * scale).round() as u32).clamp(1, height.max(1)),
    )
}

fn lease_status(suspended: bool, connected: bool) -> &'static str {
    match (suspended, connected) {
        (false, true) => "Active",
        (true, true) => "Paused",
        (false, false) => "Disconnected",
        (true, false) => "Paused (disconnected)",
    }
}

pub(crate) struct RemoteIndicator {
    pub theme: SemanticTheme,
    pub transport: String,
    pub grants: Vec<IndicatorGrant>,
    pub stop_requested: bool,
    /// Fixed local-only acknowledgement shown after emergency revocation.
    pub stopped_confirmation: bool,
}

#[derive(Clone)]
pub(crate) enum Message {
    Stop,
    Scroll,
}

impl Application for RemoteIndicator {
    type Message = Message;
    fn update(&mut self, message: Message) {
        if matches!(message, Message::Stop) {
            self.stop_requested = true;
        }
    }
    fn view(&self, context: ViewContext) -> impl View<Message> {
        if self.stopped_confirmation {
            return Column::new()
                .fill_width()
                .padding(Insets::all(14.0))
                .gap(6.0)
                .background(self.theme.surfaces.raised)
                .child(Text::new("Remote control stopped").color(self.theme.text.primary))
                .child(
                    Text::new("All remote access and input were released")
                        .color(self.theme.text.secondary)
                        .wrap(true),
                )
                .into_element();
        }
        let mut grants = Column::new().fill_width().gap(12.0);
        for grant in &self.grants {
            grants = grants.child(
                Column::new()
                    .id(format!("remote-control-lease-{}", grant.id))
                    .fill_width()
                    .gap(4.0)
                    .child(
                        Text::new(format!("Client: {}", grant.client))
                            .color(self.theme.text.primary)
                            .wrap(true),
                    )
                    .child(
                        Text::new(format!(
                            "State: {}",
                            lease_status(grant.suspended, grant.connected)
                        ))
                        .color(self.theme.text.primary)
                        .wrap(true),
                    )
                    .child(
                        Text::new(format!("Scope: {}", grant.scope))
                            .color(self.theme.text.primary)
                            .wrap(true),
                    )
                    .child(
                        Text::new(format!("Peer: {}", grant.peer))
                            .color(self.theme.text.primary)
                            .wrap(true),
                    )
                    .child(
                        Text::new(format!("Time: {}", grant.remaining))
                            .color(self.theme.text.primary)
                            .wrap(true),
                    ),
            );
        }
        Column::new()
            .fill_width()
            .padding(Insets::all(10.0))
            .gap(6.0)
            .background(self.theme.surfaces.raised)
            .child(
                Row::new()
                    .fill_width()
                    .gap(12.0)
                    .child(Text::new("Remote AI Control").color(self.theme.text.primary))
                    .child(
                        Button::semantic(
                            self.theme,
                            Message::Stop,
                            "Stop",
                            ButtonPresentation::Destructive,
                        )
                        .id("remote-control-stop")
                        .width(80.0),
                    ),
            )
            .child(
                Text::new(format!(
                    "{} · {} active / {} leases",
                    self.transport,
                    self.grants
                        .iter()
                        .filter(|grant| grant.connected && !grant.suspended)
                        .count(),
                    self.grants.len()
                ))
                .color(self.theme.text.primary)
                .wrap(true),
            )
            .child(
                VerticalScroll::new(Message::Scroll, 0.0)
                    .id("remote-control-lease-scroll")
                    .height((context.viewport.size.height - 116.0).max(1.0))
                    .theme(self.theme)
                    .child(grants),
            )
            .child(
                Text::new("Left Ctrl + Right Ctrl also stops control")
                    .color(self.theme.text.primary)
                    .wrap(true),
            )
            .into_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_height_is_bounded_and_leaves_output_margins() {
        assert_eq!(indicator_height(1, 800), 260);
        assert_eq!(indicator_height(128, 800), 356);
        assert_eq!(indicator_height(128, 200), 176);
        assert_eq!(indicator_height(1, 0), 1);
    }

    #[test]
    fn native_indicator_size_preserves_logical_dpi_size_and_small_output_bounds() {
        assert_eq!(indicator_physical_size(1, 1920, 1080, 1.0), (420, 260));
        assert_eq!(indicator_physical_size(1, 3840, 2160, 2.0), (840, 520));
        assert_eq!(indicator_physical_size(2, 1920, 1080, 1.5), (630, 534));
        assert_eq!(indicator_physical_size(2, 300, 180, 2.0), (300, 132));
        assert_eq!(indicator_physical_size(1, 0, 0, f32::NAN), (1, 1));
    }

    #[test]
    fn paused_and_disconnected_leases_are_not_presented_as_active() {
        assert_eq!(lease_status(false, true), "Active");
        assert_eq!(lease_status(true, true), "Paused");
        assert_eq!(lease_status(false, false), "Disconnected");
        assert_eq!(lease_status(true, false), "Paused (disconnected)");
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use nickel_ui::{ActionKind, HostBatch, HostEvent, SemanticAction, UiHost};

    fn host() -> UiHost<RemoteIndicator> {
        UiHost::new_at(
            RemoteIndicator {
                theme: crate::window_preview::semantic_theme_from_palette(
                    nickel_core::theme::ThemePalette::from_appearance(Default::default()),
                ),
                transport: "Protected transport".into(),
                grants: vec![IndicatorGrant {
                    suspended: false,
                    connected: true,
                    id: 1,
                    client: "Local test client".into(),
                    scope: "Application".into(),
                    remaining: "4 minutes".into(),
                    peer: "127.0.0.1".into(),
                }],
                stop_requested: false,
                stopped_confirmation: false,
            },
            420,
            indicator_height(1, 1080),
            std::time::Instant::now(),
        )
    }

    #[test]
    fn production_host_exposes_lease_state_and_local_stop_action() {
        let mut host = host();
        let labels: Vec<_> = host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect();
        for label in [
            "Remote AI Control",
            "Client: Local test client",
            "Scope: Application",
            "Peer: 127.0.0.1",
            "Time: 4 minutes",
            "State: Active",
            "Protected transport · 1 active / 1 leases",
        ] {
            assert!(
                labels.contains(&label),
                "missing local semantic label {label}"
            );
        }
        let stop = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Stop"))
            .unwrap()
            .clone();
        assert!(stop.actions.contains(&ActionKind::Activate));
        assert!(!host.application().stop_requested);
        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target: stop.id,
                action: SemanticAction::Invoke(ActionKind::Activate),
            }],
            ..Default::default()
        });
        assert!(outcome.semantic_failures.is_empty());
        assert!(host.application().stop_requested);
    }

    #[test]
    fn emergency_confirmation_is_fixed_visible_local_state_without_a_stale_stop_action() {
        let mut host = host();
        host.application_mut().grants.clear();
        host.application_mut().stopped_confirmation = true;
        host.step(HostBatch {
            application_changed: true,
            ..Default::default()
        });
        let labels = host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Remote control stopped"));
        assert!(labels.contains(&"All remote access and input were released"));
        assert!(!labels.contains(&"Stop"));
    }

    #[test]
    fn production_indicator_paints_readable_text_in_light_and_dark_themes() {
        use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};
        use nickel_ui::backend::PaintCommand;
        fn luminance(color: u32) -> f64 {
            let linear = |shift| {
                let component = f64::from((color >> shift) & 255_u32) / 255.0;
                if component <= 0.04045 {
                    component / 12.92
                } else {
                    ((component + 0.055) / 1.055).powf(2.4)
                }
            };
            linear(16) * 0.2126 + linear(8) * 0.7152 + linear(0) * 0.0722
        }
        let mut host = host();
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = crate::window_preview::semantic_theme_from_palette(
                ThemePalette::from_appearance(Appearance {
                    mode,
                    ..Default::default()
                }),
            );
            host.application_mut().theme = theme;
            host.step(HostBatch {
                application_changed: true,
                ..Default::default()
            });
            assert!(host.commands().iter().any(|command| matches!(command,
                PaintCommand::Fill { color, .. } | PaintCommand::RoundedFill { color, .. }
                if *color == theme.surfaces.raised)));
            let mut labels = 0;
            for command in host.commands() {
                if let PaintCommand::Text { text, color, .. }
                | PaintCommand::StyledText { text, color, .. } = command
                    && text != "Stop"
                {
                    assert_eq!(
                        *color, theme.text.primary,
                        "indicator label {text} ignores theme {mode:?}"
                    );
                    let foreground = luminance(*color);
                    let background = luminance(theme.surfaces.raised);
                    assert!(
                        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
                            >= 4.5,
                        "unreadable indicator label {text} in {mode:?}"
                    );
                    labels += 1;
                }
            }
            assert!(
                labels >= 8,
                "expected every production indicator label to paint"
            );
        }
    }

    #[test]
    fn production_host_pointer_stop_uses_hit_testing_after_resize() {
        let mut host = host();
        host.step(HostBatch {
            surface_size: Some((330, 190)),
            ..Default::default()
        });
        let stop = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Stop"))
            .unwrap();
        let position = nickel_input::Point {
            x: f64::from(stop.rect.origin.x + stop.rect.size.width / 2.0),
            y: f64::from(stop.rect.origin.y + stop.rect.size.height / 2.0),
        };
        for (order, edge) in [
            (1, nickel_input::KeyEdge::Pressed),
            (2, nickel_input::KeyEdge::Released),
        ] {
            host.step(HostBatch {
                events: vec![HostEvent::Normalized {
                    input: nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                        device: nickel_input::DeviceId(1),
                        order: nickel_input::EventOrder(order),
                        button: nickel_input::PointerButton::Primary,
                        edge,
                        position: Some(position),
                    }),
                    clipboard_text: None,
                }],
                ..Default::default()
            });
        }
        assert!(host.application().stop_requested);
    }

    #[test]
    fn production_host_updates_disconnected_and_paused_indication() {
        let mut host = host();
        host.application_mut().grants[0].connected = false;
        host.application_mut().grants[0].suspended = true;
        host.step(HostBatch {
            application_changed: true,
            ..Default::default()
        });
        let labels: Vec<_> = host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect();
        assert!(labels.contains(&"State: Paused (disconnected)"));
        assert!(labels.contains(&"Protected transport · 0 active / 1 leases"));
        assert!(!labels.contains(&"State: Active"));
    }
}
