//! Declarative Control Center scene and semantic interaction model.

use crate::platform::{
    AudioStatus, BluetoothStatus, NetworkStatus, SessionAction, WorkspaceSummary,
};
use nickel_core::display_projection::ProjectionMode;
use nickel_core::theme::{Appearance, ThemePalette};
use nickel_i18n::Localizer;
use nickel_ui::{
    Align, AnyView, Application, Button, Column, ComponentBuilderExt, Container, CustomPaint,
    DesktopDensity, Grid, Insets, Length, LinearGradient, ReadingDirection, Rect, Row,
    SemanticRole, SemanticTheme, SemanticTokenSet, Slider, Spacer, Switch, SwitchState, Text,
    UiHost, VerticalScroll, ViewContext, backend::PaintCommand,
};

const HEADER: f32 = 48.0;
const ROW: f32 = 46.0;

fn control_theme(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::from_tokens(SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ))
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlAction {
    ToggleWifiSection,
    WifiScroll,
    SetWifiEnabled(bool),
    ActivateWifi {
        id: String,
    },
    ToggleBluetoothSection,
    BluetoothScroll,
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    ToggleBluetoothDevice {
        id: String,
    },
    ToggleAudioSection,
    AudioScroll,
    SetAudioVolume(u8),
    // Emitted by the Linux guarded audio owner; Windows uses its native
    // volume path and does not currently construct this shell action.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    SetAudioMuted(bool),
    SelectAudioDevice {
        id: String,
    },
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
    /// Super+P opens the projection chooser directly instead of burying it in
    /// the general-purpose Control Center.
    pub projection_only: bool,
}

pub struct ControlCenterApp {
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    workspaces: Vec<WorkspaceSummary>,
    supported_projection_modes: Vec<ProjectionMode>,
    state: ControlViewState,
    direction_override: Option<ReadingDirection>,
    locale_override: Option<String>,
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
            palette: ThemePalette::from_appearance(Appearance::default()),
            network,
            bluetooth,
            audio,
            workspaces,
            supported_projection_modes: vec![
                ProjectionMode::InternalOnly,
                ProjectionMode::Duplicate,
                ProjectionMode::Extend,
                ProjectionMode::ExternalOnly,
            ],
            state: ControlViewState::default(),
            direction_override: None,
            locale_override: None,
            effects: Vec::new(),
            dirty: false,
        }
    }

    pub fn set_palette(&mut self, palette: ThemePalette) {
        if self.palette != palette {
            self.palette = palette;
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub fn set_reading_direction(&mut self, direction: Option<ReadingDirection>) {
        if self.direction_override != direction {
            self.direction_override = direction;
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub fn set_locale(&mut self, locale: Option<&str>) {
        let locale = locale.map(str::to_owned);
        if self.locale_override != locale {
            self.locale_override = locale;
            self.dirty = true;
        }
    }

    pub fn sync(
        &mut self,
        network: &NetworkStatus,
        bluetooth: &BluetoothStatus,
        audio: &AudioStatus,
        workspaces: &[WorkspaceSummary],
        supported_projection_modes: &[ProjectionMode],
    ) {
        if self.network != *network
            || self.bluetooth != *bluetooth
            || self.audio != *audio
            || self.workspaces != workspaces
            || self.supported_projection_modes != supported_projection_modes
        {
            self.network = network.clone();
            self.bluetooth = bluetooth.clone();
            self.audio = audio.clone();
            self.workspaces = workspaces.to_vec();
            self.supported_projection_modes = supported_projection_modes.to_vec();
            self.dirty = true;
        }
    }

    pub fn show_projection_chooser(&mut self) {
        self.state.projection_only = true;
        self.state.pending_projection = None;
        self.dirty = true;
    }

    pub fn show_control_center(&mut self) {
        if self.state.projection_only {
            self.state.projection_only = false;
            self.state.pending_projection = None;
            self.dirty = true;
        }
    }

    pub fn projection_preview_failed(&mut self) {
        if self.state.pending_projection.take().is_some() {
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
        control_center_view(self, context)
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

pub type ControlCenterHost = UiHost<ControlCenterApp>;

struct Card {
    view: AnyView<ControlAction>,
}

fn directional_row(row: Row<ControlAction>, direction: ReadingDirection) -> Row<ControlAction> {
    if direction == ReadingDirection::RightToLeft {
        row.reverse()
    } else {
        row
    }
}

fn control_center_view(app: &ControlCenterApp, context: ViewContext) -> AnyView<ControlAction> {
    let ControlCenterApp {
        palette,
        network,
        bluetooth,
        audio,
        workspaces,
        supported_projection_modes,
        state,
        direction_override,
        locale_override,
        ..
    } = app;
    let localizer = locale_override
        .as_deref()
        .map(|locale| Localizer::for_locale(Some(locale)))
        .unwrap_or_else(Localizer::system);
    let direction = direction_override.unwrap_or_else(|| {
        if localizer.is_right_to_left() {
            ReadingDirection::RightToLeft
        } else {
            ReadingDirection::LeftToRight
        }
    });
    let palette = *palette;
    let width = context.viewport.size.width.max(280.0);
    let height = context.viewport.size.height.max(240.0);
    let viewport_height = height - HEADER;
    if state.projection_only {
        return projection_chooser_view(
            palette,
            state.pending_projection,
            supported_projection_modes,
            width,
            height,
            direction,
            &localizer,
        );
    }
    let cards = vec![
        wifi(palette, network, state.wifi_expanded, direction, &localizer),
        bluetooth_view(
            palette,
            bluetooth,
            state.bluetooth_expanded,
            direction,
            &localizer,
        ),
        audio_view(palette, audio, state.audio_expanded, direction, &localizer),
        workspaces_view(palette, workspaces, direction, &localizer),
        card(
            palette,
            vec![AnyView::new(directional_row(
                Row::new()
                    .gap(8.0)
                    .child(
                        button(
                            palette,
                            action(ControlAction::ToggleShowDesktop),
                            localizer.text("control-center-show-desktop"),
                        )
                        .id("show-desktop"),
                    )
                    .child(
                        button(
                            palette,
                            action(ControlAction::ShowNotifications),
                            localizer.text("control-center-notifications"),
                        )
                        .id("show-notifications"),
                    ),
                direction,
            ))],
        ),
        projection_view(
            palette,
            state.pending_projection,
            supported_projection_modes,
            direction,
            &localizer,
        ),
        session_view(palette, state.pending_session_action, direction, &localizer),
    ];
    let density = DesktopDensity::COMPACT;
    let content = Column::new()
        .gap(density.related_gap)
        .padding(density.surface_inset)
        .children(cards.into_iter().map(|card| card.view));
    AnyView::new(
        Column::new()
            .width(width)
            .height(height)
            .background(LinearGradient::vertical(palette.panel, palette.background))
            .child(
                Container::new()
                    .height(HEADER)
                    .padding(Insets::symmetric(10.0, density.surface_inset))
                    .background(palette.panel)
                    .child(
                        Text::new(localizer.text("control-center-title"))
                            .scale(1.5)
                            .bold(true)
                            .color(palette.text),
                    ),
            )
            .child(
                VerticalScroll::new(ControlAction::ToggleWifiSection, 0.0)
                    .id("control-center-scroll")
                    .theme(control_theme(palette))
                    .height(viewport_height)
                    .child(content),
            ),
    )
}

fn projection_view(
    palette: ThemePalette,
    pending: Option<ProjectionMode>,
    supported: &[ProjectionMode],
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
    if pending.is_some() {
        return card(
            palette,
            vec![
                AnyView::new(
                    Text::new(localizer.text("control-center-keep-display-settings"))
                        .color(palette.text),
                ),
                AnyView::new(directional_row(
                    Row::new()
                        .gap(8.0)
                        .child(button(
                            palette,
                            action(ControlAction::CancelProjection),
                            localizer.text("control-center-revert"),
                        ))
                        .child(button(
                            palette,
                            action(ControlAction::ConfirmProjection),
                            localizer.text("control-center-keep"),
                        )),
                    direction,
                )),
            ],
        );
    }
    let modes = [
        (
            localizer.text("control-center-display-internal"),
            ProjectionMode::InternalOnly,
        ),
        (
            localizer.text("control-center-display-duplicate"),
            ProjectionMode::Duplicate,
        ),
        (
            localizer.text("control-center-display-extend"),
            ProjectionMode::Extend,
        ),
        (
            localizer.text("control-center-display-external"),
            ProjectionMode::ExternalOnly,
        ),
    ];
    card(
        palette,
        vec![AnyView::new(directional_row(
            Row::new()
                .gap(DesktopDensity::COMPACT.related_gap)
                .align_items(Align::Center)
                .child(Text::new(localizer.text("control-center-displays")).color(palette.text))
                .child(Spacer::flex())
                .children(
                    modes
                        .into_iter()
                        .filter(|(_, mode)| supported.contains(mode))
                        .map(|(label, mode)| {
                            AnyView::new(button(
                                palette,
                                action(ControlAction::PreviewProjection(mode)),
                                label,
                            ))
                        }),
                ),
            direction,
        ))],
    )
}

fn projection_chooser_view(
    palette: ThemePalette,
    pending: Option<ProjectionMode>,
    supported: &[ProjectionMode],
    width: f32,
    height: f32,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> AnyView<ControlAction> {
    let content = if supported.is_empty() {
        AnyView::new(
            Column::new()
                .gap(8.0)
                .child(
                    Text::new(localizer.text("control-center-displays"))
                        .scale(3.0)
                        .bold(true)
                        .color(palette.text),
                )
                .child(
                    Text::new(localizer.text("control-center-displays-unavailable"))
                        .color(palette.muted),
                ),
        )
    } else {
        projection_view(palette, pending, supported, direction, localizer).view
    };
    AnyView::new(
        Container::new()
            .width(width.max(280.0))
            .height(height.max(240.0))
            .padding(24.0)
            .background(LinearGradient::vertical(palette.panel, palette.background))
            .child(content),
    )
}

fn card(palette: ThemePalette, children: Vec<AnyView<ControlAction>>) -> Card {
    let density = DesktopDensity::COMPACT;
    Card {
        view: AnyView::new(
            Column::new()
                .padding(density.related_gap)
                .gap(density.related_gap)
                .background(palette.surface)
                .border(palette.surface_hover, 1.0)
                .radius(12.0)
                .children(children),
        ),
    }
}

fn title(palette: ThemePalette, name: &str, detail: String, color: u32) -> AnyView<ControlAction> {
    AnyView::new(
        Column::new()
            .height(38.0)
            .gap(1.0)
            .child(
                Text::new(name)
                    .height(22.0)
                    .scale(2.0)
                    .bold(true)
                    .color(palette.text),
            )
            .child(Text::new(detail).height(15.0).scale(1.0).color(color)),
    )
}

fn action(value: ControlAction) -> ControlAction {
    value
}

const fn translucent(color: u32, alpha: u32) -> u32 {
    (color & 0x00ff_ffff) | (alpha << 24)
}

const fn subdued_accent(accent: u32, neutral: u32) -> u32 {
    let red = (((accent >> 16) & 0xff) + 3 * ((neutral >> 16) & 0xff)) / 4;
    let green = (((accent >> 8) & 0xff) + 3 * ((neutral >> 8) & 0xff)) / 4;
    let blue = ((accent & 0xff) + 3 * (neutral & 0xff)) / 4;
    (red << 16) | (green << 8) | blue
}

fn button(
    palette: ThemePalette,
    value: ControlAction,
    label: impl Into<String>,
) -> Button<ControlAction> {
    Button::new(value, label)
        .height(DesktopDensity::COMPACT.touch_target)
        .padding(Insets {
            top: 6.0,
            right: 10.0,
            bottom: 6.0,
            left: 10.0,
        })
        .radius(7.0)
        .background(palette.surface_hover)
        .border(translucent(palette.muted, 0x38), 1.0)
        .color(palette.text)
        .center_label_vertically()
        .focus_background_tint(palette.accent)
        .controller_focus_background_tint(palette.accent)
}

fn workspace_mark_button(
    palette: ThemePalette,
    value: ControlAction,
    id: &'static str,
    label: &'static str,
    plus: bool,
    enabled: bool,
) -> Container<ControlAction> {
    let mut commands = vec![PaintCommand::RoundedFill {
        rect: Rect::new(3.0, 8.0, 12.0, 2.0),
        color: palette.text,
        radius: 1.0,
    }];
    if plus {
        commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(8.0, 3.0, 2.0, 12.0),
            color: palette.text,
            radius: 1.0,
        });
    }
    Container::new()
        .id(id)
        .width(DesktopDensity::COMPACT.touch_target)
        .height(DesktopDensity::COMPACT.touch_target)
        .shrink(0.0)
        .radius(7.0)
        .background(palette.surface_hover)
        .border(translucent(palette.muted, 0x38), 1.0)
        .interaction_backgrounds(palette.surface_hover, palette.surface)
        .focus_background_tint(palette.accent)
        .controller_focus_background_tint(palette.accent)
        .align_items(Align::Center)
        .justify_content(nickel_ui::Justify::Center)
        .semantic_role(SemanticRole::Button)
        .accessibility_label(label)
        .message(value)
        .enabled(enabled)
        .child(CustomPaint::commands(commands).width(18.0).height(18.0))
}

fn section(
    palette: ThemePalette,
    id: &str,
    expanded: bool,
    value: ControlAction,
    localizer: &Localizer,
) -> AnyView<ControlAction> {
    AnyView::new(
        Button::new(
            action(value),
            if expanded {
                localizer.text("control-center-hide-devices")
            } else {
                localizer.text("control-center-show-devices")
            },
        )
        .id(id)
        .height(DesktopDensity::COMPACT.touch_target)
        .padding(8.0)
        .background(palette.surface)
        .color(palette.muted)
        .focus_background_tint(palette.accent)
        .controller_focus_background_tint(palette.accent),
    )
}

fn toggle(
    palette: ThemePalette,
    id: &str,
    value: bool,
    enabled: bool,
    message: ControlAction,
) -> AnyView<ControlAction> {
    let state = match (value, enabled) {
        (false, true) => SwitchState::Off,
        (true, true) => SwitchState::On,
        (false, false) => SwitchState::DisabledOff,
        (true, false) => SwitchState::DisabledOn,
    };
    AnyView::new(
        Switch::with_state_action(state, enabled.then_some(message), control_theme(palette))
            .id(id)
            .accessibility_label(id),
    )
}

fn status_row(
    palette: ThemePalette,
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
        .background(if selected {
            palette.accent_soft
        } else {
            palette.surface
        })
        .radius(7.0)
        .child(
            Text::new(name)
                .height(20.0)
                .bold(selected)
                .color(palette.text),
        )
        .child(
            Text::new(detail)
                .height(15.0)
                .scale(0.8)
                .color(if selected {
                    palette.complement
                } else {
                    palette.muted
                }),
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

fn wifi(
    palette: ThemePalette,
    status: &NetworkStatus,
    expanded: bool,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
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
        if cfg!(target_os = "windows") {
            format!("{} saved", status.networks.len())
        } else {
            format!("{} nearby", status.networks.len())
        }
    };
    let mut children = vec![AnyView::new(directional_row(
        Row::new()
            .min_height(DesktopDensity::COMPACT.touch_target)
            .align_items(Align::Center)
            .child(title(
                palette,
                &localizer.text("control-center-wifi"),
                detail,
                palette.muted,
            ))
            .child(Spacer::flex())
            .child(section(
                palette,
                "wifi-section",
                expanded,
                ControlAction::ToggleWifiSection,
                localizer,
            ))
            .child(toggle(
                palette,
                "wifi-power",
                status.enabled,
                status.available,
                ControlAction::SetWifiEnabled(!status.enabled),
            )),
        direction,
    ))];
    if expanded {
        let rows = status.networks.iter().take(8).map(|network| {
            let detail = if network.connected {
                format!("CONNECTED · {}%", network.signal_percent)
            } else if network.saved {
                format!("SAVED · {}%", network.signal_percent)
            } else {
                format!("{}% SIGNAL", network.signal_percent)
            };
            status_row(
                palette,
                format!("wifi-{}", network.id),
                nonempty(&network.name, "Hidden network"),
                detail,
                network.connected,
                (network.saved && !network.connected).then(|| ControlAction::ActivateWifi {
                    id: network.id.clone(),
                }),
            )
        });
        children.push(AnyView::new(
            VerticalScroll::new(ControlAction::WifiScroll, 0.0)
                .id("wifi-devices-scroll")
                .theme(control_theme(palette))
                .max_height(80.0)
                .child(Column::new().gap(2.0).children(rows)),
        ));
    }
    card(palette, children)
}

fn bluetooth_view(
    palette: ThemePalette,
    status: &BluetoothStatus,
    expanded: bool,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
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
    let mut children = vec![AnyView::new(directional_row(
        Row::new()
            .min_height(DesktopDensity::COMPACT.touch_target)
            .align_items(Align::Center)
            .child(title(
                palette,
                &localizer.text("control-center-bluetooth"),
                detail,
                palette.muted,
            ))
            .child(Spacer::flex())
            .child(section(
                palette,
                "bluetooth-section",
                expanded,
                ControlAction::ToggleBluetoothSection,
                localizer,
            ))
            .child(toggle(
                palette,
                "bluetooth-power",
                status.powered,
                status.available,
                ControlAction::SetBluetoothPowered(!status.powered),
            )),
        direction,
    ))];
    if expanded {
        children.push(AnyView::new(directional_row(
            Row::new()
                .min_height(DesktopDensity::COMPACT.touch_target)
                .child(if status.available && status.powered {
                    AnyView::new(
                        button(
                            palette,
                            scan,
                            if status.discovering {
                                "Stop scan"
                            } else {
                                "Scan nearby"
                            },
                        )
                        .id("bluetooth-scan")
                        .width(116.0)
                        .height(DesktopDensity::COMPACT.touch_target),
                    )
                } else {
                    AnyView::new(
                        Container::new()
                            .width(116.0)
                            .height(DesktopDensity::COMPACT.touch_target)
                            .radius(14.0)
                            .background(palette.surface_hover)
                            .child(
                                Text::new(if status.discovering {
                                    "Stop scan"
                                } else {
                                    "Scan nearby"
                                })
                                .color(palette.muted),
                            ),
                    )
                })
                .child(Spacer::flex()),
            direction,
        )));
        let rows = status.devices.iter().take(8).map(|device| {
            status_row(
                palette,
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
                (if cfg!(target_os = "windows") {
                    !device.paired
                } else {
                    device.paired
                })
                .then(|| ControlAction::ToggleBluetoothDevice {
                    id: device.id.clone(),
                }),
            )
        });
        children.push(AnyView::new(
            VerticalScroll::new(ControlAction::BluetoothScroll, 0.0)
                .id("bluetooth-devices-scroll")
                .theme(control_theme(palette))
                .max_height(80.0)
                .child(Column::new().gap(2.0).children(rows)),
        ));
    }
    card(palette, children)
}

fn volume(value: f32) -> ControlAction {
    ControlAction::SetAudioVolume((value.clamp(0.0, 1.0) * 100.0).round() as u8)
}

fn audio_view(
    palette: ThemePalette,
    status: &AudioStatus,
    expanded: bool,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
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
        AnyView::new(directional_row(
            Row::new()
                .min_height(DesktopDensity::COMPACT.touch_target)
                .align_items(Align::Center)
                .child(title(
                    palette,
                    &localizer.text("control-center-audio"),
                    detail,
                    if status.muted {
                        palette.complement
                    } else {
                        palette.muted
                    },
                ))
                .child(Spacer::flex())
                .child(section(
                    palette,
                    "audio-section",
                    expanded,
                    ControlAction::ToggleAudioSection,
                    localizer,
                )),
            direction,
        )),
        AnyView::new(
            Slider::on_change(volume, f32::from(status.volume_percent) / 100.0)
                .colors(palette.surface_hover, palette.accent, palette.muted)
                .thumb_border(translucent(palette.muted, 0x60))
                .id("audio-volume")
                .accessibility_label("Audio volume")
                .width_length(Length::Fill),
        ),
    ];
    if expanded {
        let rows = status.devices.iter().take(8).map(|device| {
            status_row(
                palette,
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
        });
        children.push(AnyView::new(
            VerticalScroll::new(ControlAction::AudioScroll, 0.0)
                .id("audio-devices-scroll")
                .theme(control_theme(palette))
                .max_height(80.0)
                .child(Column::new().gap(2.0).children(rows)),
        ));
    }
    card(palette, children)
}

fn workspaces_view(
    palette: ThemePalette,
    workspaces: &[WorkspaceSummary],
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
    let workspace_controls = workspaces
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, workspace)| {
            AnyView::new(
                button(
                    palette,
                    action(ControlAction::SwitchWorkspace(workspace.id)),
                    (index + 1).to_string(),
                )
                .id(format!("workspace-{}", workspace.id))
                .width(DesktopDensity::COMPACT.touch_target)
                .height(DesktopDensity::COMPACT.touch_target)
                .shrink(0.0)
                .background(if workspace.active {
                    palette.accent
                } else {
                    palette.surface_hover
                })
                .border(
                    if workspace.active {
                        translucent(subdued_accent(palette.accent, palette.muted), 0x78)
                    } else {
                        translucent(palette.muted, 0x38)
                    },
                    1.0,
                ),
            )
        })
        .collect::<Vec<_>>();
    let can_remove = workspaces.len() > 1;
    let active_workspace = workspaces
        .iter()
        .find(|workspace| workspace.active)
        .map_or(0, |workspace| workspace.id);
    let create = AnyView::new(workspace_mark_button(
        palette,
        action(ControlAction::CreateWorkspace),
        "workspace-create",
        "Add workspace",
        true,
        true,
    ));
    let remove = AnyView::new(workspace_mark_button(
        palette,
        action(ControlAction::RemoveWorkspace(active_workspace)),
        "workspace-remove",
        "Remove workspace",
        false,
        can_remove,
    ));
    let controls = Row::new()
        .fill_width()
        .min_height(DesktopDensity::COMPACT.touch_target)
        .gap(DesktopDensity::COMPACT.related_gap)
        .align_items(Align::Center)
        .children(workspace_controls)
        .child(Spacer::flex())
        .child(create)
        .child(remove);
    card(
        palette,
        vec![
            AnyView::new(
                Text::new(localizer.text("control-center-workspaces"))
                    .height(22.0)
                    .bold(true)
                    .color(palette.text),
            ),
            AnyView::new(directional_row(controls, direction)),
        ],
    )
}

fn session_view(
    palette: ThemePalette,
    pending: Option<SessionAction>,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
    if let Some(pending) = pending {
        let cancel = action(ControlAction::CancelSessionAction);
        let confirm = action(ControlAction::ConfirmSessionAction);
        return card(
            palette,
            vec![
                AnyView::new(
                    Text::new(confirmation(pending))
                        .height(22.0)
                        .scale(1.5)
                        .bold(true)
                        .color(palette.text),
                ),
                AnyView::new(directional_row(
                    Row::new()
                        .height(DesktopDensity::COMPACT.touch_target)
                        .child(
                            button(palette, cancel, "Cancel")
                                .id("session-cancel")
                                .width(104.0)
                                .height(DesktopDensity::COMPACT.touch_target),
                        )
                        .child(Spacer::flex())
                        .child(
                            button(palette, confirm, "Confirm")
                                .id("session-confirm")
                                .width(118.0)
                                .height(DesktopDensity::COMPACT.touch_target)
                                .background(control_theme(palette).text.danger),
                        ),
                    direction,
                )),
            ],
        );
    }
    let entries = [
        (
            localizer.text("control-center-lock"),
            ControlAction::SessionAction(SessionAction::Lock),
        ),
        (
            localizer.text("control-center-suspend"),
            ControlAction::RequestSessionAction(SessionAction::Suspend),
        ),
        (
            localizer.text("control-center-restart-shell"),
            ControlAction::RequestSessionAction(SessionAction::RestartShell),
        ),
        (
            localizer.text("control-center-log-out"),
            ControlAction::RequestSessionAction(SessionAction::LogOut),
        ),
        (
            localizer.text("control-center-restart"),
            ControlAction::RequestSessionAction(SessionAction::Reboot),
        ),
        (
            localizer.text("control-center-shut-down"),
            ControlAction::RequestSessionAction(SessionAction::PowerOff),
        ),
    ];
    let controls = entries
        .into_iter()
        .enumerate()
        .map(|(index, (label, value))| {
            AnyView::new(button(palette, action(value), label).id(format!("session-{index}")))
        });
    card(
        palette,
        vec![
            AnyView::new(
                Text::new(localizer.text("control-center-session"))
                    .height(22.0)
                    .scale(1.5)
                    .bold(true)
                    .color(palette.text),
            ),
            AnyView::new(
                Grid::fixed(3)
                    .gap(DesktopDensity::COMPACT.related_gap)
                    .children(controls),
            ),
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
    use super::{ControlAction, ControlCenterApp, ControlCenterHost, HEADER};
    use crate::platform::{
        AudioDeviceStatus, AudioStatus, BluetoothDeviceStatus, BluetoothStatus, NetworkStatus,
        SessionAction, WifiNetworkStatus, WorkspaceSummary,
    };
    use nickel_core::display_projection::ProjectionMode;
    use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};
    use nickel_ui::{
        ActionKind, Application, ReadingDirection, SemanticAction, SemanticRole, SemanticValueInput,
    };
    use nickel_ui_testkit::{ActivationVia, Scenario, Selector};

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
    fn projection_shortcut_view_contains_only_topology_supported_modes() {
        let mut host = build(&[]);
        host.application_mut().sync(
            &NetworkStatus::default(),
            &BluetoothStatus::default(),
            &AudioStatus::default(),
            &[],
            &[ProjectionMode::Duplicate, ProjectionMode::Extend],
        );
        host.application_mut().show_projection_chooser();
        host.poll();

        assert!(has_action(
            &host,
            &ControlAction::PreviewProjection(ProjectionMode::Duplicate)
        ));
        assert!(has_action(
            &host,
            &ControlAction::PreviewProjection(ProjectionMode::Extend)
        ));
        assert!(!has_action(
            &host,
            &ControlAction::PreviewProjection(ProjectionMode::InternalOnly)
        ));
        assert!(!has_action(
            &host,
            &ControlAction::PreviewProjection(ProjectionMode::ExternalOnly)
        ));
        assert!(!has_action(&host, &ControlAction::ToggleShowDesktop));
    }

    #[test]
    fn failed_projection_preview_returns_to_the_mode_chooser() {
        let mut host = build(&[]);
        host.application_mut().show_projection_chooser();
        Application::update(
            host.application_mut(),
            ControlAction::PreviewProjection(ProjectionMode::Extend),
        );
        host.application_mut().projection_preview_failed();
        host.poll();

        assert!(!has_action(&host, &ControlAction::ConfirmProjection));
        assert!(!has_action(&host, &ControlAction::CancelProjection));
        assert!(has_action(
            &host,
            &ControlAction::PreviewProjection(ProjectionMode::Extend)
        ));
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
    fn workspace_create_and_remove_controls_keep_fixed_positions() {
        let one = build(&[WorkspaceSummary {
            id: 4,
            active: true,
        }]);
        let three = build(&[
            WorkspaceSummary {
                id: 4,
                active: true,
            },
            WorkspaceSummary {
                id: 9,
                active: false,
            },
            WorkspaceSummary {
                id: 12,
                active: false,
            },
        ]);
        let control = |host: &ControlCenterHost, name: &str| {
            host.semantic_nodes()
                .into_iter()
                .find(|node| node.name.as_deref() == Some(name))
                .expect("workspace control should remain present")
        };

        let one_create = control(&one, "Add workspace");
        let one_remove = control(&one, "Remove workspace");
        let three_create = control(&three, "Add workspace");
        let three_remove = control(&three, "Remove workspace");
        assert_eq!(one_create.bounds, three_create.bounds);
        assert_eq!(one_remove.bounds, three_remove.bounds);
        assert!(!one_remove.enabled);
        assert!(three_remove.enabled);
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

    #[test]
    fn collapsed_control_center_fits_four_hundred_by_seven_twenty_without_scrolling() {
        let host = ControlCenterHost::new(
            ControlCenterApp::new(
                NetworkStatus::default(),
                BluetoothStatus::default(),
                AudioStatus::default(),
                (1..=4)
                    .map(|id| WorkspaceSummary {
                        id,
                        active: id == 1,
                    })
                    .collect(),
            ),
            400,
            720,
        );

        let extent = host
            .scroll_extent(&ControlAction::ToggleWifiSection)
            .expect("Control Center owns one bounded top-level scroll region");
        assert!(
            !extent.can_scroll(),
            "collapsed Control Center must fit without scrolling: {extent:?}"
        );
        for action in [
            ControlAction::ToggleWifiSection,
            ControlAction::ToggleBluetoothSection,
            ControlAction::ToggleAudioSection,
            ControlAction::ToggleShowDesktop,
            ControlAction::ShowNotifications,
            ControlAction::SessionAction(SessionAction::Lock),
            ControlAction::RequestSessionAction(SessionAction::PowerOff),
        ] {
            let targets = host.semantic_targets_for_message(&action);
            assert!(!targets.is_empty(), "{action:?} must remain reachable");
            assert!(targets.iter().any(|target| {
                target.bounds.origin.y >= HEADER
                    && target.bounds.origin.y + target.bounds.size.height <= 720.0
            }));
        }
    }

    #[test]
    fn expanded_device_list_owns_bounded_scroll_and_keeps_collapse_reachable() {
        let mut network = NetworkStatus {
            available: true,
            enabled: true,
            ..NetworkStatus::default()
        };
        network.networks = (0..8)
            .map(|index| WifiNetworkStatus {
                id: format!("wifi-{index}"),
                name: format!("Network {index}"),
                signal_percent: 80,
                saved: true,
                ..WifiNetworkStatus::default()
            })
            .collect();
        let mut app = ControlCenterApp::new(
            network,
            BluetoothStatus::default(),
            AudioStatus::default(),
            vec![WorkspaceSummary {
                id: 1,
                active: true,
            }],
        );
        app.state.wifi_expanded = true;
        let host = ControlCenterHost::new(app, 400, 720);

        assert!(
            host.scroll_extent(&ControlAction::WifiScroll)
                .is_some_and(|extent| extent.can_scroll()),
            "the expanded device region, not an unbounded card, owns overflow"
        );
        assert!(
            host.scroll_extent(&ControlAction::ToggleWifiSection)
                .is_some(),
            "the existing top-level scroll owner remains available for expanded content"
        );
        assert!(
            host.semantic_targets_for_message(&ControlAction::ToggleWifiSection)
                .iter()
                .any(|target| target.bounds.origin.y + target.bounds.size.height <= 720.0)
        );
    }

    #[test]
    fn every_expanded_device_list_owns_bounded_overflow_and_preserves_disclosure_focus() {
        let network = NetworkStatus {
            available: true,
            enabled: true,
            networks: (0..8)
                .map(|index| WifiNetworkStatus {
                    id: format!("wifi-{index}"),
                    name: format!("Network {index}"),
                    signal_percent: 80,
                    saved: true,
                    ..WifiNetworkStatus::default()
                })
                .collect(),
            ..NetworkStatus::default()
        };
        let bluetooth = BluetoothStatus {
            available: true,
            powered: true,
            devices: (0..8)
                .map(|index| BluetoothDeviceStatus {
                    id: format!("bluetooth-{index}"),
                    name: format!("Bluetooth device {index}"),
                    paired: true,
                    ..BluetoothDeviceStatus::default()
                })
                .collect(),
            ..BluetoothStatus::default()
        };
        let audio = AudioStatus {
            available: true,
            devices: (0..8)
                .map(|index| AudioDeviceStatus {
                    id: format!("audio-{index}"),
                    name: format!("Audio device {index}"),
                    is_default: index == 0,
                })
                .collect(),
            ..AudioStatus::default()
        };

        for (disclosure, toggle, scroll) in [
            (
                "wifi-section",
                ControlAction::ToggleWifiSection,
                ControlAction::WifiScroll,
            ),
            (
                "bluetooth-section",
                ControlAction::ToggleBluetoothSection,
                ControlAction::BluetoothScroll,
            ),
            (
                "audio-section",
                ControlAction::ToggleAudioSection,
                ControlAction::AudioScroll,
            ),
        ] {
            let mut scenario = Scenario::new(
                ControlCenterApp::new(
                    network.clone(),
                    bluetooth.clone(),
                    audio.clone(),
                    vec![WorkspaceSummary {
                        id: 1,
                        active: true,
                    }],
                ),
                400,
                720,
            );
            let disclosure_id = scenario
                .host()
                .semantic_targets_for_message(&toggle)
                .into_iter()
                .find(|target| target.id.as_str().ends_with(disclosure))
                .expect("disclosure target")
                .id;
            let selector = Selector::id(disclosure_id.clone());
            scenario.keyboard_activate(&selector).unwrap();

            assert_eq!(
                scenario.host().inspect().keyboard_focus,
                Some(disclosure_id.clone()),
                "expansion must retain focus on {disclosure}"
            );
            let extent = scenario
                .host()
                .scroll_extent(&scroll)
                .expect("expanded device scroll");
            assert!(
                extent.can_scroll() && extent.viewport.height <= 80.0,
                "{disclosure} must give overflow to its bounded device list: {extent:?}"
            );
            let disclosure_target = scenario
                .host()
                .semantic_nodes()
                .into_iter()
                .find(|target| target.id == disclosure_id)
                .expect("expanded disclosure remains semantic");
            assert!(
                disclosure_target.bounds.origin.y >= HEADER
                    && disclosure_target.bounds.origin.y + disclosure_target.bounds.size.height
                        <= 720.0,
                "expanded {disclosure} must keep its collapse action visible"
            );
            scenario.keyboard_activate(&selector).unwrap();
            assert_eq!(
                scenario.host().inspect().keyboard_focus,
                Some(disclosure_id),
                "collapse must retain focus on {disclosure}"
            );
        }
    }

    #[test]
    fn control_center_primary_action_activates_through_every_supported_modality() {
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Touch,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ] {
            let mut scenario = Scenario::new(
                ControlCenterApp::new(
                    NetworkStatus::default(),
                    BluetoothStatus::default(),
                    AudioStatus::default(),
                    vec![WorkspaceSummary {
                        id: 1,
                        active: true,
                    }],
                ),
                400,
                720,
            );
            let target = scenario
                .host()
                .unique_semantic_target_for_message(&ControlAction::ToggleShowDesktop)
                .expect("Show desktop target");
            scenario
                .invoke_via(via, &Selector::id(target.id), ActionKind::Activate)
                .unwrap_or_else(|error| panic!("{via:?} activation failed: {error}"));
            assert_eq!(
                scenario.host_mut().application_mut().take_effects(),
                [ControlAction::ToggleShowDesktop],
                "{via:?} must route the same typed production action"
            );
        }
    }

    #[test]
    fn collapsed_interactive_targets_are_touch_sized_and_do_not_overlap() {
        let host = ControlCenterHost::new(
            ControlCenterApp::new(
                NetworkStatus {
                    available: true,
                    enabled: true,
                    ..NetworkStatus::default()
                },
                BluetoothStatus {
                    available: true,
                    powered: true,
                    ..BluetoothStatus::default()
                },
                AudioStatus {
                    available: true,
                    ..AudioStatus::default()
                },
                (1..=4)
                    .map(|id| WorkspaceSummary {
                        id,
                        active: id == 1,
                    })
                    .collect(),
            ),
            400,
            720,
        );
        let interactive = host
            .semantic_nodes()
            .into_iter()
            .filter(|node| matches!(node.role, Some(SemanticRole::Button | SemanticRole::Switch)))
            .collect::<Vec<_>>();

        for node in &interactive {
            assert!(
                node.bounds.size.width >= 44.0 && node.bounds.size.height >= 44.0,
                "touch target {:?} is undersized: {:?}",
                node.id,
                node.bounds
            );
        }
        for (index, first) in interactive.iter().enumerate() {
            for second in &interactive[index + 1..] {
                let overlap_width = (first.bounds.origin.x + first.bounds.size.width)
                    .min(second.bounds.origin.x + second.bounds.size.width)
                    - first.bounds.origin.x.max(second.bounds.origin.x);
                let overlap_height = (first.bounds.origin.y + first.bounds.size.height)
                    .min(second.bounds.origin.y + second.bounds.size.height)
                    - first.bounds.origin.y.max(second.bounds.origin.y);
                assert!(
                    overlap_width <= 0.0 || overlap_height <= 0.0,
                    "interactive targets {:?} and {:?} overlap",
                    first.id,
                    second.id
                );
            }
        }
    }

    #[test]
    fn localized_empty_loading_failure_and_ordinary_states_remain_bounded() {
        let states = [
            (
                NetworkStatus::default(),
                BluetoothStatus::default(),
                AudioStatus::default(),
            ),
            (
                NetworkStatus {
                    available: true,
                    enabled: true,
                    networks: vec![WifiNetworkStatus {
                        id: "nearby".into(),
                        name: "Nearby network".into(),
                        signal_percent: 72,
                        ..WifiNetworkStatus::default()
                    }],
                    ..NetworkStatus::default()
                },
                BluetoothStatus {
                    available: true,
                    powered: true,
                    discovering: true,
                    ..BluetoothStatus::default()
                },
                AudioStatus {
                    available: true,
                    volume_percent: 45,
                    ..AudioStatus::default()
                },
            ),
            (
                NetworkStatus {
                    available: true,
                    enabled: true,
                    connected: true,
                    name: "Nickel network".into(),
                    signal_percent: 91,
                    ..NetworkStatus::default()
                },
                BluetoothStatus {
                    available: true,
                    powered: true,
                    devices: vec![BluetoothDeviceStatus {
                        id: "headphones".into(),
                        name: "Headphones".into(),
                        paired: true,
                        connected: true,
                    }],
                    ..BluetoothStatus::default()
                },
                AudioStatus {
                    available: true,
                    volume_percent: 63,
                    devices: vec![AudioDeviceStatus {
                        id: "speakers".into(),
                        name: "Speakers".into(),
                        is_default: true,
                    }],
                    ..AudioStatus::default()
                },
            ),
        ];

        for locale in ["en-US", "de", "zh", "es", "ar"] {
            for (network, bluetooth, audio) in &states {
                let mut app = ControlCenterApp::new(
                    network.clone(),
                    bluetooth.clone(),
                    audio.clone(),
                    vec![WorkspaceSummary {
                        id: 1,
                        active: true,
                    }],
                );
                app.set_locale(Some(locale));
                let host = ControlCenterHost::new(app, 400, 720);
                assert!(
                    !host
                        .scroll_extent(&ControlAction::ToggleWifiSection)
                        .expect("top-level scroll")
                        .can_scroll(),
                    "collapsed locale/state combination must fit: {locale}"
                );
                assert!(host.semantic_nodes().iter().all(|node| {
                    node.bounds.origin.x >= 0.0
                        && node.bounds.origin.y >= 0.0
                        && node.bounds.origin.x + node.bounds.size.width <= 400.0
                        && node.bounds.origin.y + node.bounds.size.height <= 720.0
                }));
            }
        }
    }

    #[test]
    fn control_center_theme_direction_and_viewport_matrix_is_bounded() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            for direction in [ReadingDirection::LeftToRight, ReadingDirection::RightToLeft] {
                for scale in [1.0, 1.25, 2.0] {
                    for (width, height) in [(400, 720), (520, 640)] {
                        let mut app = ControlCenterApp::new(
                            NetworkStatus::default(),
                            BluetoothStatus::default(),
                            AudioStatus::default(),
                            vec![WorkspaceSummary {
                                id: 1,
                                active: true,
                            }],
                        );
                        app.set_palette(ThemePalette::from_appearance(Appearance {
                            mode,
                            ..Appearance::default()
                        }));
                        app.set_reading_direction(Some(direction));
                        let mut host = ControlCenterHost::new(app, width, height);
                        host.set_scale_factor(scale);
                        for node in host.semantic_nodes() {
                            assert!(node.bounds.origin.x.is_finite());
                            assert!(node.bounds.origin.y.is_finite());
                            assert!(node.bounds.size.width >= 0.0);
                            assert!(node.bounds.size.height >= 0.0);
                            assert!(node.bounds.origin.x + node.bounds.size.width <= width as f32);
                        }
                        assert!(
                            host.semantic_targets_for_message(&ControlAction::SessionAction(
                                SessionAction::Lock
                            ))
                            .iter()
                            .any(|target| target.bounds.origin.y < height as f32)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn right_to_left_control_rows_mirror_disclosure_and_power_order() {
        let positions = |direction| {
            let mut app = ControlCenterApp::new(
                NetworkStatus {
                    available: true,
                    enabled: true,
                    ..NetworkStatus::default()
                },
                BluetoothStatus::default(),
                AudioStatus::default(),
                Vec::new(),
            );
            app.set_reading_direction(Some(direction));
            let host = ControlCenterHost::new(app, 400, 720);
            let disclosure = host
                .semantic_targets_for_message(&ControlAction::ToggleWifiSection)
                .into_iter()
                .find(|target| target.id.as_str().ends_with("wifi-section"))
                .expect("Wi-Fi disclosure");
            let power = host
                .unique_semantic_target_for_message(&ControlAction::SetWifiEnabled(false))
                .expect("Wi-Fi power");
            (disclosure.bounds.origin.x, power.bounds.origin.x)
        };
        let ltr = positions(ReadingDirection::LeftToRight);
        let rtl = positions(ReadingDirection::RightToLeft);
        assert!(ltr.0 < ltr.1, "LTR disclosure precedes power: {ltr:?}");
        assert!(rtl.1 < rtl.0, "RTL power precedes disclosure: {rtl:?}");
    }

    #[test]
    fn control_center_uses_localized_labels_and_locale_direction() {
        let mut spanish = ControlCenterApp::new(
            NetworkStatus::default(),
            BluetoothStatus::default(),
            AudioStatus::default(),
            Vec::new(),
        );
        spanish.set_locale(Some("es"));
        let spanish = ControlCenterHost::new(spanish, 400, 720);
        assert!(
            spanish
                .semantic_nodes()
                .iter()
                .any(|node| node.name.as_deref() == Some("Mostrar escritorio"))
        );

        let positions = |locale| {
            let mut app = ControlCenterApp::new(
                NetworkStatus {
                    available: true,
                    enabled: true,
                    ..NetworkStatus::default()
                },
                BluetoothStatus::default(),
                AudioStatus::default(),
                Vec::new(),
            );
            app.set_locale(Some(locale));
            let host = ControlCenterHost::new(app, 400, 720);
            let disclosure = host
                .semantic_targets_for_message(&ControlAction::ToggleWifiSection)
                .into_iter()
                .find(|target| target.id.as_str().ends_with("wifi-section"))
                .unwrap();
            let power = host
                .unique_semantic_target_for_message(&ControlAction::SetWifiEnabled(false))
                .unwrap();
            (disclosure.bounds.origin.x, power.bounds.origin.x)
        };
        assert!(positions("en-US").0 < positions("en-US").1);
        assert!(positions("ar").1 < positions("ar").0);
    }
}
