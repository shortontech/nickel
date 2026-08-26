//! Renderer-neutral Control Center scene and interaction model.

use nickel_ui::{LinearGradient, PaintCommand, Rect, TextAlign};

use crate::platform::{AudioStatus, BluetoothStatus, NetworkStatus};

const BACKGROUND_TOP: u32 = 0x202b43;
const BACKGROUND_BOTTOM: u32 = 0x111827;
const CARD: u32 = 0x2b3852;
const CARD_BORDER: u32 = 0x42516c;
const PRIMARY: u32 = 0xf4f7ff;
const SECONDARY: u32 = 0xaebbd1;
const ACCENT: u32 = 0x65b8ff;
const GOOD: u32 = 0x6ee7a8;
const WARNING: u32 = 0xf6c76e;

const PADDING: f32 = 16.0;
const HEADER_HEIGHT: f32 = 66.0;
const CARD_GAP: f32 = 12.0;
const ROW_HEIGHT: f32 = 46.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ControlAction {
    ToggleWifiSection,
    SetWifiEnabled(bool),
    ActivateWifi { id: String },
    ToggleBluetoothSection,
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    ToggleBluetoothDevice { id: String },
    ToggleAudioSection,
    SetAudioVolume(u8),
    SelectAudioDevice { id: String },
    ToggleLogoutConfirmation,
    LogOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HitTarget {
    pub bounds: Rect,
    pub action: ControlAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlViewState {
    pub wifi_expanded: bool,
    pub bluetooth_expanded: bool,
    pub audio_expanded: bool,
    pub logout_confirmation: bool,
    /// Positive logical pixels scrolled below the fixed header.
    pub scroll_offset: f32,
}

pub struct ControlCenterFrame {
    pub commands: Vec<PaintCommand>,
    pub hit_targets: Vec<HitTarget>,
    pub content_height: f32,
    pub viewport_height: f32,
    volume_track: Option<Rect>,
}

impl ControlCenterFrame {
    pub fn action_at(&self, x: f32, y: f32) -> Option<ControlAction> {
        self.hit_targets
            .iter()
            .rev()
            .find(|target| contains(target.bounds, x, y))
            .map(|target| match &target.action {
                ControlAction::SetAudioVolume(_) => {
                    let track = self.volume_track.unwrap_or(target.bounds);
                    let fraction = ((x - track.origin.x) / track.size.width).clamp(0.0, 1.0);
                    ControlAction::SetAudioVolume((fraction * 100.0).round() as u8)
                }
                action => action.clone(),
            })
    }

    pub fn maximum_scroll(&self) -> f32 {
        (self.content_height - self.viewport_height).max(0.0)
    }
}

pub fn build_control_center(
    network: &NetworkStatus,
    bluetooth: &BluetoothStatus,
    audio: &AudioStatus,
    state: ControlViewState,
    size: (f32, f32),
) -> ControlCenterFrame {
    let width = size.0.max(280.0);
    let height = size.1.max(240.0);
    let viewport = Rect::new(0.0, HEADER_HEIGHT, width, height - HEADER_HEIGHT);
    let mut builder = ViewBuilder {
        commands: vec![
            PaintCommand::Gradient {
                rect: Rect::new(0.0, 0.0, width, height),
                gradient: LinearGradient::vertical(BACKGROUND_TOP, BACKGROUND_BOTTOM),
            },
            PaintCommand::Fill {
                rect: Rect::new(0.0, 0.0, width, HEADER_HEIGHT),
                color: BACKGROUND_TOP,
            },
            text(
                Rect::new(PADDING, 17.0, width - PADDING * 2.0, 30.0),
                "Control Center",
                3.0,
                PRIMARY,
                true,
            ),
            PaintCommand::PushClip(viewport),
        ],
        hits: Vec::new(),
        width,
        viewport,
        y: HEADER_HEIGHT + PADDING - state.scroll_offset.max(0.0),
        volume_track: None,
    };

    builder.wifi(network, state.wifi_expanded);
    builder.bluetooth(bluetooth, state.bluetooth_expanded);
    builder.audio(audio, state.audio_expanded);
    builder.session(state.logout_confirmation);

    let content_bottom = builder.y + state.scroll_offset.max(0.0);
    builder.commands.push(PaintCommand::PopClip);
    ControlCenterFrame {
        commands: builder.commands,
        hit_targets: builder.hits,
        content_height: (content_bottom - HEADER_HEIGHT + PADDING).max(0.0),
        viewport_height: viewport.size.height,
        volume_track: builder.volume_track,
    }
}

struct ViewBuilder {
    commands: Vec<PaintCommand>,
    hits: Vec<HitTarget>,
    width: f32,
    viewport: Rect,
    y: f32,
    volume_track: Option<Rect>,
}

impl ViewBuilder {
    fn session(&mut self, confirming_logout: bool) {
        let card = self.card(if confirming_logout { 92.0 } else { 68.0 });
        self.commands.push(text(
            Rect::new(
                card.origin.x + 14.0,
                card.origin.y + 11.0,
                card.size.width - 28.0,
                22.0,
            ),
            if confirming_logout {
                "Log out of Nickel?"
            } else {
                "Session"
            },
            1.5,
            PRIMARY,
            true,
        ));

        if confirming_logout {
            self.action_button(
                Rect::new(card.origin.x + 14.0, card.origin.y + 47.0, 104.0, 30.0),
                "Cancel",
                ControlAction::ToggleLogoutConfirmation,
                false,
            );
            self.action_button(
                Rect::new(
                    card.origin.x + card.size.width - 132.0,
                    card.origin.y + 47.0,
                    118.0,
                    30.0,
                ),
                "Log out",
                ControlAction::LogOut,
                true,
            );
        } else {
            self.action_button(
                Rect::new(
                    card.origin.x + card.size.width - 132.0,
                    card.origin.y + 19.0,
                    118.0,
                    30.0,
                ),
                "Log out",
                ControlAction::ToggleLogoutConfirmation,
                false,
            );
        }
        self.finish_card(card);
    }

    fn wifi(&mut self, status: &NetworkStatus, expanded: bool) {
        let rows = usize::from(expanded) * status.networks.len().min(8);
        let height = 78.0 + rows as f32 * ROW_HEIGHT;
        let card = self.card(height);
        self.label(
            card,
            "Wi-Fi",
            if !status.available {
                "Unavailable".into()
            } else if !status.enabled {
                "Powered off".into()
            } else if status.connected {
                format!(
                    "{} · {}% signal",
                    nonempty(&status.name, "Connected"),
                    status.signal_percent
                )
            } else {
                format!("{} nearby", status.networks.len())
            },
            status.connected.then_some(GOOD),
        );
        self.toggle(
            Rect::new(
                card.origin.x + card.size.width - 58.0,
                card.origin.y + 15.0,
                42.0,
                24.0,
            ),
            status.enabled,
            status.available,
            ControlAction::SetWifiEnabled(!status.enabled),
        );
        self.chevron_hit(
            Rect::new(card.origin.x, card.origin.y + 44.0, card.size.width, 34.0),
            expanded,
            ControlAction::ToggleWifiSection,
        );

        if expanded {
            for (index, network) in status.networks.iter().take(8).enumerate() {
                let row = Rect::new(
                    card.origin.x + 10.0,
                    card.origin.y + 78.0 + index as f32 * ROW_HEIGHT,
                    card.size.width - 20.0,
                    ROW_HEIGHT,
                );
                let detail = if network.connected {
                    format!("CONNECTED · {}%", network.signal_percent)
                } else if network.saved {
                    format!("SAVED · {}%", network.signal_percent)
                } else {
                    format!("{}% SIGNAL", network.signal_percent)
                };
                self.row(
                    row,
                    nonempty(&network.name, "Hidden network"),
                    &detail,
                    network.connected,
                );
                if network.saved && !network.connected {
                    self.hit(
                        row,
                        ControlAction::ActivateWifi {
                            id: network.id.clone(),
                        },
                    );
                }
            }
        }
        self.finish_card(card);
    }

    fn bluetooth(&mut self, status: &BluetoothStatus, expanded: bool) {
        let rows = usize::from(expanded) * status.devices.len().min(8);
        let height = 96.0 + rows as f32 * ROW_HEIGHT;
        let card = self.card(height);
        let connected = status
            .devices
            .iter()
            .filter(|device| device.connected)
            .count();
        self.label(
            card,
            "Bluetooth",
            if !status.available {
                "Unavailable".into()
            } else if !status.powered {
                "Powered off".into()
            } else if connected > 0 {
                format!("{connected} connected")
            } else if status.discovering {
                "Discovering nearby devices".into()
            } else {
                format!("{} known devices", status.devices.len())
            },
            (connected > 0).then_some(GOOD),
        );
        self.toggle(
            Rect::new(
                card.origin.x + card.size.width - 58.0,
                card.origin.y + 15.0,
                42.0,
                24.0,
            ),
            status.powered,
            status.available,
            ControlAction::SetBluetoothPowered(!status.powered),
        );
        let discovery = Rect::new(card.origin.x + 12.0, card.origin.y + 50.0, 116.0, 28.0);
        self.pill(
            discovery,
            if status.discovering {
                "Stop scan"
            } else {
                "Scan nearby"
            },
            status.discovering,
        );
        if status.available && status.powered {
            self.hit(
                discovery,
                ControlAction::SetBluetoothDiscovery(!status.discovering),
            );
        }
        self.chevron_hit(
            Rect::new(
                card.origin.x + card.size.width - 80.0,
                card.origin.y + 46.0,
                68.0,
                36.0,
            ),
            expanded,
            ControlAction::ToggleBluetoothSection,
        );

        if expanded {
            for (index, device) in status.devices.iter().take(8).enumerate() {
                let row = Rect::new(
                    card.origin.x + 10.0,
                    card.origin.y + 96.0 + index as f32 * ROW_HEIGHT,
                    card.size.width - 20.0,
                    ROW_HEIGHT,
                );
                let detail = if device.connected {
                    "CONNECTED"
                } else if device.paired {
                    "PAIRED"
                } else {
                    "NEARBY"
                };
                self.row(
                    row,
                    nonempty(&device.name, "Bluetooth device"),
                    detail,
                    device.connected,
                );
                if device.paired {
                    self.hit(
                        row,
                        ControlAction::ToggleBluetoothDevice {
                            id: device.id.clone(),
                        },
                    );
                }
            }
        }
        self.finish_card(card);
    }

    fn audio(&mut self, status: &AudioStatus, expanded: bool) {
        let rows = usize::from(expanded) * status.devices.len().min(8);
        let height = 116.0 + rows as f32 * ROW_HEIGHT;
        let card = self.card(height);
        let selected = status
            .devices
            .iter()
            .find(|device| device.is_default)
            .map(|device| device.name.as_str())
            .unwrap_or("No audio output");
        self.label(
            card,
            "Audio",
            if status.muted {
                format!("Muted · {selected}")
            } else {
                format!("{}% · {selected}", status.volume_percent)
            },
            status.muted.then_some(WARNING),
        );
        let track = Rect::new(
            card.origin.x + 14.0,
            card.origin.y + 58.0,
            card.size.width - 28.0,
            18.0,
        );
        self.commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(track.origin.x, track.origin.y + 6.0, track.size.width, 6.0),
            color: CARD_BORDER,
            radius: 3.0,
        });
        self.commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(
                track.origin.x,
                track.origin.y + 6.0,
                track.size.width * f32::from(status.volume_percent) / 100.0,
                6.0,
            ),
            color: ACCENT,
            radius: 3.0,
        });
        self.volume_track = Some(track);
        if status.available {
            self.hit(track, ControlAction::SetAudioVolume(status.volume_percent));
        }
        self.chevron_hit(
            Rect::new(card.origin.x, card.origin.y + 80.0, card.size.width, 36.0),
            expanded,
            ControlAction::ToggleAudioSection,
        );
        if expanded {
            for (index, device) in status.devices.iter().take(8).enumerate() {
                let row = Rect::new(
                    card.origin.x + 10.0,
                    card.origin.y + 116.0 + index as f32 * ROW_HEIGHT,
                    card.size.width - 20.0,
                    ROW_HEIGHT,
                );
                self.row(
                    row,
                    nonempty(&device.name, "Audio device"),
                    if device.is_default {
                        "DEFAULT"
                    } else {
                        "AVAILABLE"
                    },
                    device.is_default,
                );
                self.hit(
                    row,
                    ControlAction::SelectAudioDevice {
                        id: device.id.clone(),
                    },
                );
            }
        }
        self.finish_card(card);
    }

    fn card(&mut self, height: f32) -> Rect {
        let card = Rect::new(PADDING, self.y, self.width - PADDING * 2.0, height);
        self.commands.push(PaintCommand::RoundedFill {
            rect: card,
            color: CARD,
            radius: 12.0,
        });
        self.commands.push(PaintCommand::Stroke {
            rect: card,
            color: CARD_BORDER,
            width: 1.0,
        });
        card
    }

    fn finish_card(&mut self, card: Rect) {
        self.y = card.origin.y + card.size.height + CARD_GAP;
    }

    fn label(&mut self, card: Rect, title: &str, detail: String, detail_color: Option<u32>) {
        self.commands.push(text(
            Rect::new(
                card.origin.x + 14.0,
                card.origin.y + 10.0,
                card.size.width - 82.0,
                24.0,
            ),
            title,
            2.0,
            PRIMARY,
            true,
        ));
        self.commands.push(text(
            Rect::new(
                card.origin.x + 14.0,
                card.origin.y + 33.0,
                card.size.width - 28.0,
                18.0,
            ),
            &detail,
            1.0,
            detail_color.unwrap_or(SECONDARY),
            false,
        ));
    }

    fn row(&mut self, row: Rect, name: &str, detail: &str, selected: bool) {
        if selected {
            self.commands.push(PaintCommand::RoundedFill {
                rect: row,
                color: 0x344d68,
                radius: 7.0,
            });
        }
        self.commands.push(text(
            Rect::new(
                row.origin.x + 8.0,
                row.origin.y + 5.0,
                row.size.width - 16.0,
                20.0,
            ),
            name,
            1.0,
            PRIMARY,
            selected,
        ));
        self.commands.push(text(
            Rect::new(
                row.origin.x + 8.0,
                row.origin.y + 25.0,
                row.size.width - 16.0,
                15.0,
            ),
            detail,
            0.8,
            if selected { GOOD } else { SECONDARY },
            false,
        ));
    }

    fn toggle(&mut self, rect: Rect, enabled: bool, interactive: bool, action: ControlAction) {
        self.commands.push(PaintCommand::RoundedFill {
            rect,
            color: if enabled { ACCENT } else { CARD_BORDER },
            radius: rect.size.height / 2.0,
        });
        self.commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(
                if enabled {
                    rect.origin.x + rect.size.width - rect.size.height + 3.0
                } else {
                    rect.origin.x + 3.0
                },
                rect.origin.y + 3.0,
                rect.size.height - 6.0,
                rect.size.height - 6.0,
            ),
            color: if interactive { PRIMARY } else { SECONDARY },
            radius: (rect.size.height - 6.0) / 2.0,
        });
        if interactive {
            self.hit(rect, action);
        }
    }

    fn pill(&mut self, rect: Rect, label: &str, active: bool) {
        self.commands.push(PaintCommand::RoundedFill {
            rect,
            color: if active { ACCENT } else { CARD_BORDER },
            radius: rect.size.height / 2.0,
        });
        self.commands.push(text(
            Rect::new(
                rect.origin.x + 8.0,
                rect.origin.y + 5.0,
                rect.size.width - 16.0,
                18.0,
            ),
            label,
            0.9,
            PRIMARY,
            false,
        ));
    }

    fn action_button(&mut self, rect: Rect, label: &str, action: ControlAction, warning: bool) {
        self.commands.push(PaintCommand::RoundedFill {
            rect,
            color: if warning { 0x9f3f4a } else { 0x34445f },
            radius: 7.0,
        });
        self.commands.push(text(
            Rect::new(
                rect.origin.x + 10.0,
                rect.origin.y + 6.0,
                rect.size.width - 20.0,
                18.0,
            ),
            label,
            1.0,
            PRIMARY,
            true,
        ));
        self.hit(rect, action);
    }

    fn chevron_hit(&mut self, rect: Rect, expanded: bool, action: ControlAction) {
        self.commands.push(text(
            Rect::new(
                rect.origin.x + 8.0,
                rect.origin.y + 7.0,
                rect.size.width - 16.0,
                20.0,
            ),
            if expanded {
                "Hide devices"
            } else {
                "Show devices"
            },
            0.9,
            SECONDARY,
            false,
        ));
        self.hit(rect, action);
    }

    fn hit(&mut self, bounds: Rect, action: ControlAction) {
        if let Some(bounds) = intersection(bounds, self.viewport) {
            self.hits.push(HitTarget { bounds, action });
        }
    }
}

fn text(bounds: Rect, value: &str, scale: f32, color: u32, bold: bool) -> PaintCommand {
    PaintCommand::Text {
        bounds,
        text: value.to_owned(),
        scale,
        color,
        align: TextAlign::Start,
        bold,
    }
}

fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn contains(rect: Rect, x: f32, y: f32) -> bool {
    x >= rect.origin.x
        && y >= rect.origin.y
        && x < rect.origin.x + rect.size.width
        && y < rect.origin.y + rect.size.height
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x = left.origin.x.max(right.origin.x);
    let y = left.origin.y.max(right.origin.y);
    let right_edge = (left.origin.x + left.size.width).min(right.origin.x + right.size.width);
    let bottom_edge = (left.origin.y + left.size.height).min(right.origin.y + right.size.height);
    (right_edge > x && bottom_edge > y).then(|| Rect::new(x, y, right_edge - x, bottom_edge - y))
}

#[cfg(test)]
mod tests {
    use crate::platform::{AudioStatus, BluetoothStatus, NetworkStatus};

    use super::{ControlAction, ControlViewState, build_control_center};

    fn logout_action(state: ControlViewState) -> Option<ControlAction> {
        let frame = build_control_center(
            &NetworkStatus::default(),
            &BluetoothStatus::default(),
            &AudioStatus::default(),
            state,
            (380.0, 650.0),
        );
        let target = frame.hit_targets.iter().find(|target| {
            matches!(
                target.action,
                ControlAction::ToggleLogoutConfirmation | ControlAction::LogOut
            )
        })?;
        frame.action_at(
            target.bounds.origin.x + target.bounds.size.width / 2.0,
            target.bounds.origin.y + target.bounds.size.height / 2.0,
        )
    }

    #[test]
    fn logout_requires_confirmation() {
        assert_eq!(
            logout_action(ControlViewState::default()),
            Some(ControlAction::ToggleLogoutConfirmation)
        );
        assert_eq!(
            logout_action(ControlViewState {
                logout_confirmation: true,
                ..ControlViewState::default()
            }),
            Some(ControlAction::ToggleLogoutConfirmation)
        );
    }

    #[test]
    fn confirmed_logout_is_available_as_a_distinct_action() {
        let frame = build_control_center(
            &NetworkStatus::default(),
            &BluetoothStatus::default(),
            &AudioStatus::default(),
            ControlViewState {
                logout_confirmation: true,
                ..ControlViewState::default()
            },
            (380.0, 650.0),
        );
        assert!(
            frame
                .hit_targets
                .iter()
                .any(|target| target.action == ControlAction::LogOut)
        );
    }
}
