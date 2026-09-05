//! Declarative Control Center scene and semantic interaction model.

use crate::platform::{
    AudioStatus, BluetoothStatus, NetworkStatus, SessionAction, WorkspaceSummary,
};
use nickel_core::display_projection::ProjectionMode;
use nickel_ui::{
    Align, AnyView, Application, Button, Column, ComponentBuilderExt, Container, Grid, Insets,
    Length, LinearGradient, Row, SemanticRole, Slider, Spacer, Text, UiHost, VerticalScroll,
    ViewContext,
};

const TOP: u32 = 0x202b43;
const BOTTOM: u32 = 0x111827;
const CARD: u32 = 0x2b3852;
const BORDER: u32 = 0x42516c;
const PRIMARY: u32 = 0xf4f7ff;
const SECONDARY: u32 = 0xaebbd1;
const ACCENT: u32 = 0x65b8ff;
const GOOD: u32 = 0x6ee7a8;
const WARNING: u32 = 0xf6c76e;
const HEADER: f32 = 66.0;
const ROW: f32 = 46.0;

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
    SwitchWorkspace(u64),
    CreateWorkspace,
    ToggleShowDesktop,
    ShowNotifications,
    RemoveWorkspace(u64),
    PreviewProjection(ProjectionMode),
    ConfirmProjection,
    CancelProjection,
    RequestSessionAction(SessionAction),
    CancelSessionAction,
    ConfirmSessionAction,
    SessionAction(SessionAction),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlViewState {
    pub wifi_expanded: bool,
    pub bluetooth_expanded: bool,
    pub audio_expanded: bool,
    pub pending_session_action: Option<SessionAction>,
    pub pending_projection: Option<ProjectionMode>,
}

pub struct ControlCenterApp {
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    workspaces: Vec<WorkspaceSummary>,
    state: ControlViewState,
    effects: Vec<ControlAction>,
    dirty: bool,
}

impl ControlCenterApp {
    pub fn new(
        network: NetworkStatus,
        bluetooth: BluetoothStatus,
        audio: AudioStatus,
        workspaces: Vec<WorkspaceSummary>,
    ) -> Self {
        Self {
            network,
            bluetooth,
            audio,
            workspaces,
            state: ControlViewState::default(),
            effects: Vec::new(),
            dirty: false,
        }
    }

    pub fn sync(
        &mut self,
        network: &NetworkStatus,
        bluetooth: &BluetoothStatus,
        audio: &AudioStatus,
        workspaces: &[WorkspaceSummary],
    ) {
        if self.network != *network
            || self.bluetooth != *bluetooth
            || self.audio != *audio
            || self.workspaces != workspaces
        {
            self.network = network.clone();
            self.bluetooth = bluetooth.clone();
            self.audio = audio.clone();
            self.workspaces = workspaces.to_vec();
            self.dirty = true;
        }
    }

    pub fn request_session_action(&mut self, action: SessionAction) {
        if self.state.pending_session_action != Some(action) {
            self.state.pending_session_action = Some(action);
            self.dirty = true;
        }
    }

    pub fn take_effects(&mut self) -> Vec<ControlAction> {
        std::mem::take(&mut self.effects)
    }
}

impl Application for ControlCenterApp {
    type Message = ControlAction;

    fn update(&mut self, message: Self::Message) {
        match message {
            ControlAction::ToggleWifiSection => {
                self.state.wifi_expanded = !self.state.wifi_expanded;
            }
            ControlAction::ToggleBluetoothSection => {
                self.state.bluetooth_expanded = !self.state.bluetooth_expanded;
            }
            ControlAction::ToggleAudioSection => {
                self.state.audio_expanded = !self.state.audio_expanded;
            }
            ControlAction::RequestSessionAction(action) => {
                self.state.pending_session_action = Some(action);
            }
            ControlAction::CancelSessionAction => self.state.pending_session_action = None,
            ControlAction::PreviewProjection(mode) => self.state.pending_projection = Some(mode),
            ControlAction::ConfirmProjection | ControlAction::CancelProjection => {
                self.state.pending_projection = None
            }
            ControlAction::ConfirmSessionAction => {
                if let Some(action) = self.state.pending_session_action.take() {
                    self.effects.push(ControlAction::SessionAction(action));
                }
                return;
            }
            _ => {}
        }
        self.effects.push(message);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        control_center_view(
            &self.network,
            &self.bluetooth,
            &self.audio,
            &self.workspaces,
            self.state,
            context.viewport.size.width,
            context.viewport.size.height,
        )
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

pub type ControlCenterHost = UiHost<ControlCenterApp>;

struct Card {
    view: AnyView<ControlAction>,
}

fn control_center_view(
    network: &NetworkStatus,
    bluetooth: &BluetoothStatus,
    audio: &AudioStatus,
    workspaces: &[WorkspaceSummary],
    state: ControlViewState,
    width: f32,
    height: f32,
) -> AnyView<ControlAction> {
    let width = width.max(280.0);
    let height = height.max(240.0);
    let viewport_height = height - HEADER;
    let cards = vec![
        wifi(network, state.wifi_expanded),
        bluetooth_view(bluetooth, state.bluetooth_expanded),
        audio_view(audio, state.audio_expanded),
        workspaces_view(workspaces),
        card(
            64.0,
            vec![AnyView::new(
                Row::new()
                    .gap(8.0)
                    .child(
                        button(action(ControlAction::ToggleShowDesktop), "Show desktop")
                            .id("show-desktop"),
                    )
                    .child(
                        button(action(ControlAction::ShowNotifications), "Notifications")
                            .id("show-notifications"),
                    ),
            )],
        ),
        projection_view(state.pending_projection),
        session_view(state.pending_session_action),
    ];
    let content = Column::new()
        .gap(12.0)
        .padding(16.0)
        .children(cards.into_iter().map(|card| card.view));
    AnyView::new(
        Column::new()
            .width(width)
            .height(height)
            .background(LinearGradient::vertical(TOP, BOTTOM))
            .child(
                Container::new()
                    .height(HEADER)
                    .padding(Insets {
                        top: 17.0,
                        right: 16.0,
                        bottom: 19.0,
                        left: 16.0,
                    })
                    .background(TOP)
                    .child(
                        Text::new("Control Center")
                            .scale(3.0)
                            .bold(true)
                            .color(PRIMARY),
                    ),
            )
            .child(
                VerticalScroll::new(ControlAction::ToggleWifiSection, 0.0)
                    .id("control-center-scroll")
                    .height(viewport_height)
                    .child(content),
            ),
    )
}

fn projection_view(pending: Option<ProjectionMode>) -> Card {
    if pending.is_some() {
        return card(
            82.0,
            vec![
                AnyView::new(Text::new("Keep these display settings?").color(PRIMARY)),
                AnyView::new(
                    Row::new()
                        .gap(8.0)
                        .child(button(action(ControlAction::CancelProjection), "Revert"))
                        .child(button(action(ControlAction::ConfirmProjection), "Keep")),
                ),
            ],
        );
    }
    let modes = [
        ("PC screen", ProjectionMode::InternalOnly),
        ("Duplicate", ProjectionMode::Duplicate),
        ("Extend", ProjectionMode::Extend),
        ("Second screen", ProjectionMode::ExternalOnly),
    ];
    card(
        96.0,
        vec![
            AnyView::new(Text::new("Project displays").color(PRIMARY)),
            AnyView::new(
                Row::new()
                    .gap(6.0)
                    .children(modes.into_iter().map(|(label, mode)| {
                        AnyView::new(button(
                            action(ControlAction::PreviewProjection(mode)),
                            label,
                        ))
                    })),
            ),
        ],
    )
}

fn card(height: f32, children: Vec<AnyView<ControlAction>>) -> Card {
    Card {
        view: AnyView::new(
            Column::new()
                .height(height)
                .padding(14.0)
                .gap(8.0)
                .background(CARD)
                .border(BORDER, 1.0)
                .radius(12.0)
                .children(children),
        ),
    }
}

fn title(name: &str, detail: String, color: u32) -> AnyView<ControlAction> {
    AnyView::new(
        Column::new()
            .height(38.0)
            .gap(1.0)
            .child(
                Text::new(name)
                    .height(22.0)
                    .scale(2.0)
                    .bold(true)
                    .color(PRIMARY),
            )
            .child(Text::new(detail).height(15.0).scale(1.0).color(color)),
    )
}

fn action(value: ControlAction) -> ControlAction {
    value
}

fn button(value: ControlAction, label: impl Into<String>) -> Button<ControlAction> {
    Button::new(value, label)
        .height(32.0)
        .padding(Insets {
            top: 6.0,
            right: 10.0,
            bottom: 6.0,
            left: 10.0,
        })
        .radius(7.0)
        .background(0x34445f)
        .color(PRIMARY)
        .focus_background_tint(ACCENT)
        .controller_focus_background_tint(ACCENT)
}

fn section(id: &str, expanded: bool, value: ControlAction) -> AnyView<ControlAction> {
    AnyView::new(
        Button::new(
            action(value),
            if expanded {
                "Hide devices"
            } else {
                "Show devices"
            },
        )
        .id(id)
        .height(34.0)
        .padding(8.0)
        .background(CARD)
        .color(SECONDARY)
        .focus_background_tint(ACCENT)
        .controller_focus_background_tint(ACCENT),
    )
}

fn toggle(id: &str, value: bool, enabled: bool, message: ControlAction) -> AnyView<ControlAction> {
    let thumb = || {
        AnyView::new(
            Container::new()
                .width(18.0)
                .height(18.0)
                .radius(9.0)
                .background(if enabled { PRIMARY } else { SECONDARY }),
        )
    };
    AnyView::new(
        Container::new()
            .id(id)
            .width(42.0)
            .height(24.0)
            .radius(12.0)
            .padding(3.0)
            .background(if value { ACCENT } else { BORDER })
            .semantic_role(SemanticRole::Switch)
            .accessibility_label(id)
            .message(message)
            .enabled(enabled)
            .child(
                Row::new()
                    .fill_width()
                    .child(if value {
                        AnyView::new(Spacer::flex())
                    } else {
                        thumb()
                    })
                    .child(if value {
                        thumb()
                    } else {
                        AnyView::new(Spacer::flex())
                    }),
            ),
    )
}

fn status_row(
    id: String,
    name: &str,
    detail: String,
    selected: bool,
    message: Option<ControlAction>,
) -> AnyView<ControlAction> {
    let row = Column::new()
        .height(ROW)
        .padding(Insets {
            top: 5.0,
            right: 8.0,
            bottom: 5.0,
            left: 8.0,
        })
        .gap(1.0)
        .background(if selected { 0x344d68 } else { CARD })
        .radius(7.0)
        .child(Text::new(name).height(20.0).bold(selected).color(PRIMARY))
        .child(
            Text::new(detail)
                .height(15.0)
                .scale(0.8)
                .color(if selected { GOOD } else { SECONDARY }),
        );
    match message {
        Some(message) => AnyView::new(
            Container::new()
                .id(id)
                .message(message)
                .semantic_role(SemanticRole::Button)
                .accessibility_label(name)
                .child(row),
        ),
        None => AnyView::new(row),
    }
}

fn wifi(status: &NetworkStatus, expanded: bool) -> Card {
    let detail = if !status.available {
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
    };
    let mut children = vec![
        AnyView::new(
            Row::new()
                .height(38.0)
                .align_items(Align::Start)
                .child(title(
                    "Wi-Fi",
                    detail,
                    if status.connected { GOOD } else { SECONDARY },
                ))
                .child(Spacer::flex())
                .child(toggle(
                    "wifi-power",
                    status.enabled,
                    status.available,
                    ControlAction::SetWifiEnabled(!status.enabled),
                )),
        ),
        section("wifi-section", expanded, ControlAction::ToggleWifiSection),
    ];
    if expanded {
        children.extend(status.networks.iter().take(8).map(|network| {
            let detail = if network.connected {
                format!("CONNECTED · {}%", network.signal_percent)
            } else if network.saved {
                format!("SAVED · {}%", network.signal_percent)
            } else {
                format!("{}% SIGNAL", network.signal_percent)
            };
            status_row(
                format!("wifi-{}", network.id),
                nonempty(&network.name, "Hidden network"),
                detail,
                network.connected,
                (network.saved && !network.connected).then(|| ControlAction::ActivateWifi {
                    id: network.id.clone(),
                }),
            )
        }));
    }
    card(
        78.0 + usize::from(expanded) as f32 * status.networks.len().min(8) as f32 * ROW,
        children,
    )
}

fn bluetooth_view(status: &BluetoothStatus, expanded: bool) -> Card {
    let connected = status
        .devices
        .iter()
        .filter(|device| device.connected)
        .count();
    let detail = if !status.available {
        "Unavailable".into()
    } else if !status.powered {
        "Powered off".into()
    } else if connected > 0 {
        format!("{connected} connected")
    } else if status.discovering {
        "Discovering nearby devices".into()
    } else {
        format!("{} known devices", status.devices.len())
    };
    let scan = ControlAction::SetBluetoothDiscovery(!status.discovering);
    let mut children = vec![
        AnyView::new(
            Row::new()
                .height(38.0)
                .child(title(
                    "Bluetooth",
                    detail,
                    if connected > 0 { GOOD } else { SECONDARY },
                ))
                .child(Spacer::flex())
                .child(toggle(
                    "bluetooth-power",
                    status.powered,
                    status.available,
                    ControlAction::SetBluetoothPowered(!status.powered),
                )),
        ),
        AnyView::new(
            Row::new()
                .height(36.0)
                .child(if status.available && status.powered {
                    AnyView::new(
                        button(
                            scan,
                            if status.discovering {
                                "Stop scan"
                            } else {
                                "Scan nearby"
                            },
                        )
                        .id("bluetooth-scan")
                        .width(116.0)
                        .height(28.0),
                    )
                } else {
                    AnyView::new(
                        Container::new()
                            .width(116.0)
                            .height(28.0)
                            .radius(14.0)
                            .background(BORDER)
                            .child(
                                Text::new(if status.discovering {
                                    "Stop scan"
                                } else {
                                    "Scan nearby"
                                })
                                .color(SECONDARY),
                            ),
                    )
                })
                .child(Spacer::flex())
                .child(section(
                    "bluetooth-section",
                    expanded,
                    ControlAction::ToggleBluetoothSection,
                )),
        ),
    ];
    if expanded {
        children.extend(status.devices.iter().take(8).map(|device| {
            status_row(
                format!("bluetooth-{}", device.id),
                nonempty(&device.name, "Bluetooth device"),
                (if device.connected {
                    "CONNECTED"
                } else if device.paired {
                    "PAIRED"
                } else {
                    "NEARBY"
                })
                .into(),
                device.connected,
                device.paired.then(|| ControlAction::ToggleBluetoothDevice {
                    id: device.id.clone(),
                }),
            )
        }));
    }
    card(
        96.0 + usize::from(expanded) as f32 * status.devices.len().min(8) as f32 * ROW,
        children,
    )
}

fn volume(value: f32) -> ControlAction {
    ControlAction::SetAudioVolume((value.clamp(0.0, 1.0) * 100.0).round() as u8)
}

fn audio_view(status: &AudioStatus, expanded: bool) -> Card {
    let selected = status
        .devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.name.as_str())
        .unwrap_or("No audio output");
    let detail = if status.muted {
        format!("Muted · {selected}")
    } else {
        format!("{}% · {selected}", status.volume_percent)
    };
    let mut children = vec![
        title(
            "Audio",
            detail,
            if status.muted { WARNING } else { SECONDARY },
        ),
        AnyView::new(
            Slider::on_change(volume, f32::from(status.volume_percent) / 100.0)
                .colors(BORDER, ACCENT, PRIMARY)
                .id("audio-volume")
                .accessibility_label("Audio volume")
                .width_length(Length::Fill),
        ),
        section("audio-section", expanded, ControlAction::ToggleAudioSection),
    ];
    if expanded {
        children.extend(status.devices.iter().take(8).map(|device| {
            status_row(
                format!("audio-{}", device.id),
                nonempty(&device.name, "Audio device"),
                (if device.is_default {
                    "DEFAULT"
                } else {
                    "AVAILABLE"
                })
                .into(),
                device.is_default,
                Some(ControlAction::SelectAudioDevice {
                    id: device.id.clone(),
                }),
            )
        }));
    }
    card(
        116.0 + usize::from(expanded) as f32 * status.devices.len().min(8) as f32 * ROW,
        children,
    )
}

fn workspaces_view(workspaces: &[WorkspaceSummary]) -> Card {
    let mut controls = workspaces
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, workspace)| {
            AnyView::new(
                button(
                    action(ControlAction::SwitchWorkspace(workspace.id)),
                    (index + 1).to_string(),
                )
                .id(format!("workspace-{}", workspace.id))
                .width(34.0)
                .height(28.0)
                .background(if workspace.active { 0x9f3f4a } else { 0x34445f }),
            )
        })
        .collect::<Vec<_>>();
    controls.push(AnyView::new(
        button(action(ControlAction::CreateWorkspace), "+")
            .id("workspace-create")
            .width(34.0)
            .height(28.0),
    ));
    if workspaces.len() > 1
        && let Some(active) = workspaces.iter().find(|workspace| workspace.active)
    {
        controls.push(AnyView::new(
            button(action(ControlAction::RemoveWorkspace(active.id)), "−")
                .id("workspace-remove")
                .width(34.0)
                .height(28.0),
        ));
    }
    card(
        82.0,
        vec![
            AnyView::new(
                Text::new("Workspaces")
                    .height(22.0)
                    .scale(1.5)
                    .bold(true)
                    .color(PRIMARY),
            ),
            AnyView::new(Row::new().height(28.0).gap(6.0).children(controls)),
        ],
    )
}

fn session_view(pending: Option<SessionAction>) -> Card {
    if let Some(pending) = pending {
        let cancel = action(ControlAction::CancelSessionAction);
        let confirm = action(ControlAction::ConfirmSessionAction);
        return card(
            98.0,
            vec![
                AnyView::new(
                    Text::new(confirmation(pending))
                        .height(22.0)
                        .scale(1.5)
                        .bold(true)
                        .color(PRIMARY),
                ),
                AnyView::new(
                    Row::new()
                        .height(30.0)
                        .child(
                            button(cancel, "Cancel")
                                .id("session-cancel")
                                .width(104.0)
                                .height(30.0),
                        )
                        .child(Spacer::flex())
                        .child(
                            button(confirm, "Confirm")
                                .id("session-confirm")
                                .width(118.0)
                                .height(30.0)
                                .background(0x9f3f4a),
                        ),
                ),
            ],
        );
    }
    let entries = [
        ("Lock", ControlAction::SessionAction(SessionAction::Lock)),
        (
            "Suspend",
            ControlAction::RequestSessionAction(SessionAction::Suspend),
        ),
        (
            "Restart shell",
            ControlAction::RequestSessionAction(SessionAction::RestartShell),
        ),
        (
            "Log out",
            ControlAction::RequestSessionAction(SessionAction::LogOut),
        ),
        (
            "Restart",
            ControlAction::RequestSessionAction(SessionAction::Reboot),
        ),
        (
            "Shut down",
            ControlAction::RequestSessionAction(SessionAction::PowerOff),
        ),
    ];
    let controls = entries
        .into_iter()
        .enumerate()
        .map(|(index, (label, value))| {
            AnyView::new(button(action(value), label).id(format!("session-{index}")))
        });
    card(
        174.0,
        vec![
            AnyView::new(
                Text::new("Session")
                    .height(22.0)
                    .scale(1.5)
                    .bold(true)
                    .color(PRIMARY),
            ),
            AnyView::new(Grid::fixed(2).height(112.0).gap(8.0).children(controls)),
        ],
    )
}

fn confirmation(action: SessionAction) -> &'static str {
    match action {
        SessionAction::RestartShell => "Restart the Nickel shell?",
        SessionAction::Lock => "Lock this session?",
        SessionAction::Suspend => "Suspend this computer?",
        SessionAction::LogOut => "Log out of Nickel?",
        SessionAction::Reboot => "Restart this computer?",
        SessionAction::PowerOff => "Shut down this computer?",
    }
}
fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlAction, ControlCenterApp, ControlCenterHost};
    use crate::platform::{
        AudioStatus, BluetoothStatus, NetworkStatus, SessionAction, WorkspaceSummary,
    };
    use nickel_core::display_projection::ProjectionMode;
    use nickel_ui::{Application, SemanticAction, SemanticRole, SemanticValueInput};

    #[test]
    fn idle_control_center_declares_no_poll_deadline() {
        let app = ControlCenterApp::new(
            NetworkStatus::default(),
            BluetoothStatus::default(),
            AudioStatus::default(),
            Vec::new(),
        );
        assert_eq!(Application::poll_interval(&app), None);
    }

    fn build(workspaces: &[WorkspaceSummary]) -> ControlCenterHost {
        ControlCenterHost::new(
            ControlCenterApp::new(
                NetworkStatus::default(),
                BluetoothStatus::default(),
                AudioStatus::default(),
                workspaces.to_vec(),
            ),
            380,
            650,
        )
    }
    fn has_action(host: &ControlCenterHost, action: &ControlAction) -> bool {
        !host.semantic_targets_for_message(action).is_empty()
    }
    #[test]
    fn disruptive_actions_require_confirmation_but_lock_is_immediate() {
        let host = build(&[]);
        assert!(has_action(
            &host,
            &ControlAction::SessionAction(SessionAction::Lock)
        ));
        for value in [
            SessionAction::RestartShell,
            SessionAction::Suspend,
            SessionAction::LogOut,
            SessionAction::Reboot,
            SessionAction::PowerOff,
        ] {
            assert!(has_action(
                &host,
                &ControlAction::RequestSessionAction(value)
            ));
            assert!(!has_action(&host, &ControlAction::SessionAction(value)));
        }
    }

    #[test]
    fn projection_and_show_desktop_are_controller_reachable_semantic_actions() {
        let host = build(&[]);
        assert!(has_action(&host, &ControlAction::ToggleShowDesktop));
        assert!(has_action(&host, &ControlAction::ShowNotifications));
        for mode in [
            ProjectionMode::InternalOnly,
            ProjectionMode::Duplicate,
            ProjectionMode::Extend,
            ProjectionMode::ExternalOnly,
        ] {
            assert!(has_action(&host, &ControlAction::PreviewProjection(mode)));
        }
    }
    #[test]
    fn pending_action_exposes_only_cancel_and_confirm() {
        let mut host = build(&[]);
        host.application_mut()
            .request_session_action(SessionAction::PowerOff);
        host.poll();
        assert!(has_action(&host, &ControlAction::CancelSessionAction));
        assert!(has_action(&host, &ControlAction::ConfirmSessionAction));
        for action in [
            SessionAction::Lock,
            SessionAction::RestartShell,
            SessionAction::Suspend,
            SessionAction::LogOut,
            SessionAction::Reboot,
            SessionAction::PowerOff,
        ] {
            assert!(!has_action(&host, &ControlAction::SessionAction(action)));
            assert!(!has_action(
                &host,
                &ControlAction::RequestSessionAction(action)
            ));
        }
    }
    #[test]
    fn workspace_buttons_route_typed_actions() {
        let host = build(&[
            WorkspaceSummary {
                id: 4,
                active: false,
            },
            WorkspaceSummary {
                id: 9,
                active: true,
            },
        ]);
        for expected in [
            ControlAction::SwitchWorkspace(4),
            ControlAction::SwitchWorkspace(9),
            ControlAction::CreateWorkspace,
            ControlAction::RemoveWorkspace(9),
        ] {
            assert!(has_action(&host, &expected));
        }
        assert!(!has_action(&host, &ControlAction::RemoveWorkspace(4)));
    }
    #[test]
    fn volume_is_a_semantic_value_control() {
        let mut host = build(&[]);
        let slider = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::Slider))
            .unwrap();
        host.perform_semantic_action(
            slider.id,
            SemanticAction::SetValue(SemanticValueInput::Number(0.73)),
        );
        assert_eq!(
            host.application_mut().take_effects(),
            vec![ControlAction::SetAudioVolume(73)]
        );
    }
}
