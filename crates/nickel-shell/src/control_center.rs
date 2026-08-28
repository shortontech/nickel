use std::sync::Arc;

use nickel_ui::{
    Component, ComponentGpu, Insets, LinearGradient, Point, Rect, UiStateStore, UiTree, component,
    ui,
};
use winit::window::Window;

use crate::{graphics::SharedGraphics, platform};

pub const WIDTH: u32 = 380;
pub const HEIGHT: u32 = 650;

const BACKGROUND_TOP: u32 = 0x202b43;
const BACKGROUND_BOTTOM: u32 = 0x111827;
const CARD: u32 = 0x2b3852;
const CARD_BORDER: u32 = 0x42516c;
const PRIMARY: u32 = 0xf4f7ff;
const SECONDARY: u32 = 0xaebbd1;
const ACCENT: u32 = 0x65b8ff;

#[derive(Clone, Debug, PartialEq)]
enum ControlMessage {
    WifiPower,
    WifiDropdown,
    WifiNetwork(usize),
    AudioVolume(f32),
    AudioDropdown,
    AudioDevice(usize),
    BluetoothPower,
    BluetoothDiscovery,
    BluetoothDropdown,
    BluetoothDevice(usize),
}

pub struct ControlCenterGpu {
    gpu: ComponentGpu,
    size: (u32, u32),
    network: platform::NetworkStatus,
    bluetooth: platform::BluetoothStatus,
    audio: platform::AudioStatus,
    ui: UiTree<ControlMessage>,
    ui_state: UiStateStore,
    cursor: Point,
}

impl ControlCenterGpu {
    pub fn new(window: Arc<Window>, graphics: Arc<SharedGraphics>) -> Result<Self, String> {
        let surface = graphics.create_surface(window)?;
        Ok(Self {
            gpu: ComponentGpu::with_shared_graphics(
                surface,
                &graphics.adapter,
                &graphics.device,
                &graphics.queue,
                WIDTH,
                HEIGHT,
            )?,
            size: (WIDTH, HEIGHT),
            network: platform::network_status(),
            bluetooth: platform::bluetooth_status(),
            audio: platform::audio_status(),
            ui: UiTree::default(),
            ui_state: UiStateStore::default(),
            cursor: Point { x: 0.0, y: 0.0 },
        })
    }

    pub fn refresh(&mut self) {
        self.network = platform::network_status();
        self.bluetooth = platform::bluetooth_status();
        self.audio = platform::audio_status();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.gpu.resize(width, height);
    }

    pub fn render(&mut self) {
        self.ui = build_ui(
            ControlCenterView {
                network: &self.network,
                bluetooth: &self.bluetooth,
                audio: &self.audio,
            },
            (self.size.0 as f32, self.size.1 as f32),
            &mut self.ui_state,
        );
        if let Err(error) = self.gpu.render(self.ui.commands()) {
            eprintln!("failed to render Control Center: {error}");
        }
    }

    pub fn cursor_moved(&mut self, x: f32, y: f32) {
        self.cursor = Point { x, y };
        if self.is_volume_dragging()
            && let Some(fraction) = self
                .ui
                .horizontal_fraction_for_id(&UiId::from("audio-volume"), x)
        {
            self.set_volume_fraction(fraction);
        }
    }

    pub fn pointer_pressed(&mut self) -> bool {
        let Some(message) = self.ui.message_at_owned(self.cursor)
        else {
            self.close_dropdowns(None);
            return true;
        };
        let target = self.ui.id_at(self.cursor).cloned();
        self.ui_state.set_pressed(target.clone());
        self.ui_state.set_capture(target);
        if message == ControlMessage::WifiPower {
            if platform::set_wifi_enabled(!self.network.enabled) {
                self.network.enabled = !self.network.enabled;
            }
            return true;
        }
        if message == ControlMessage::WifiDropdown {
            self.toggle_dropdown(&ControlMessage::WifiDropdown);
            return true;
        }
        if let ControlMessage::WifiNetwork(index) = message {
            if let Some(network) = self.network.networks.get(index)
                && network.saved
                && !network.connected
            {
                let _ = platform::activate_wifi_network(&network.id);
            }
            self.set_dropdown(&ControlMessage::WifiDropdown, false);
            return true;
        }
        if let ControlMessage::AudioVolume(fraction) = message {
            self.set_volume_fraction(fraction);
            return true;
        }
        if message == ControlMessage::AudioDropdown {
            self.toggle_dropdown(&ControlMessage::AudioDropdown);
            return true;
        }
        if let ControlMessage::AudioDevice(index) = message {
            if let Some(device) = self.audio.devices.get(index)
                && platform::select_audio_device(&device.id)
            {
                self.audio = platform::audio_status();
            }
            self.set_dropdown(&ControlMessage::AudioDropdown, false);
            return true;
        }
        if message == ControlMessage::BluetoothPower {
            if platform::set_bluetooth_powered(!self.bluetooth.powered) {
                self.bluetooth.powered = !self.bluetooth.powered;
            }
            return true;
        }
        if message == ControlMessage::BluetoothDiscovery {
            if platform::set_bluetooth_discovery(!self.bluetooth.discovering) {
                self.bluetooth.discovering = !self.bluetooth.discovering;
            }
            return true;
        }
        if message == ControlMessage::BluetoothDropdown {
            self.toggle_dropdown(&ControlMessage::BluetoothDropdown);
            return true;
        }
        if let ControlMessage::BluetoothDevice(index) = message {
            if let Some(device) = self.bluetooth.devices.get(index)
                && device.paired
            {
                let _ = platform::toggle_bluetooth_device(&device.id);
            }
            self.set_dropdown(&ControlMessage::BluetoothDropdown, false);
            return true;
        }
        false
    }

    pub fn pointer_released(&mut self) -> bool {
        let was_dragging = self.is_volume_dragging();
        self.ui_state.set_pressed(None);
        self.ui_state.set_capture(None);
        was_dragging
    }

    pub fn is_volume_dragging(&self) -> bool {
        self.ui_state.captured() == Some(&UiId::from("audio-volume"))
    }

    fn dropdown_open(&self, message: &ControlMessage) -> bool {
        self.ui
            .id_for_message(message)
            .and_then(|id| self.ui_state.state(id))
            .is_some_and(|state| state.dropdown_open)
    }

    fn set_dropdown(&mut self, message: &ControlMessage, open: bool) {
        if let Some(id) = self.ui.id_for_message(message).cloned() {
            self.ui_state.set_dropdown_open(id, open);
        }
    }

    fn close_dropdowns(&mut self, except: Option<&ControlMessage>) {
        for message in [
            ControlMessage::WifiDropdown,
            ControlMessage::AudioDropdown,
            ControlMessage::BluetoothDropdown,
        ] {
            if except != Some(&message) {
                self.set_dropdown(&message, false);
            }
        }
    }

    fn toggle_dropdown(&mut self, message: &ControlMessage) {
        let open = !self.dropdown_open(message);
        self.close_dropdowns(Some(message));
        self.set_dropdown(message, open);
    }

    fn set_volume_fraction(&mut self, fraction: f32) {
        let volume = (fraction.clamp(0.0, 1.0) * 100.0).round() as u8;
        if volume != self.audio.volume_percent && platform::set_audio_volume(volume) {
            self.audio.volume_percent = volume;
        }
    }
}

struct ControlCenterView<'a> {
    network: &'a platform::NetworkStatus,
    bluetooth: &'a platform::BluetoothStatus,
    audio: &'a platform::AudioStatus,
}

fn audio_volume_message(value: f32) -> ControlMessage {
    ControlMessage::AudioVolume(value)
}

#[component]
fn ActionButton(
    message: ControlMessage,
    label: &str,
    width: f32,
) -> impl Component<ControlMessage> {
    ui! {
        <Container background={0x34445f} border={(CARD_BORDER, 1.0)}
            padding={Insets { top: 5.0, right: 8.0, bottom: 5.0, left: 8.0 }}
            width={width} on_press={message}>
            <Text scale={1.15} color={PRIMARY}>{label}</Text>
        </Container>
    }
}

fn build_ui(
    view: ControlCenterView<'_>,
    size: (f32, f32),
    state: &mut UiStateStore,
) -> UiTree<ControlMessage> {
    let ControlCenterView {
        network,
        bluetooth,
        audio,
    } = view;
    let (network_name, network_detail) = if network.connected {
        (
            if network.name.is_empty() {
                "Connected".to_owned()
            } else {
                network.name.clone()
            },
            format!("CONNECTED  ·  {}% SIGNAL", network.signal_percent),
        )
    } else if network.available && network.enabled {
        ("Wi-Fi".to_owned(), "AVAILABLE · NOT CONNECTED".to_owned())
    } else if network.available {
        ("Wi-Fi".to_owned(), "POWERED OFF".to_owned())
    } else {
        ("Wi-Fi".to_owned(), "UNAVAILABLE".to_owned())
    };
    let selected_audio = audio
        .devices
        .iter()
        .enumerate()
        .find(|(_, device)| device.is_default)
        .map(|(index, device)| (device.name.clone(), ControlMessage::AudioDevice(index)))
        .map(|(name, _)| name)
        .unwrap_or_else(|| "No audio output".into());
    let audio_devices: Vec<_> = audio
        .devices
        .iter()
        .enumerate()
        .map(|(index, device)| (device.name.clone(), ControlMessage::AudioDevice(index)))
        .collect();
    let selected_network = network
        .networks
        .iter()
        .find(|candidate| candidate.connected)
        .map(|candidate| candidate.name.clone())
        .unwrap_or_else(|| "Select a saved network".into());
    let wifi_networks = network
        .networks
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let saved = if candidate.saved { "" } else { " · NEW" };
            (
                format!(
                    "{} · {}%{}",
                    candidate.name, candidate.signal_percent, saved
                ),
                ControlMessage::WifiNetwork(index),
            )
        })
        .collect::<Vec<_>>();
    let selected_bluetooth = bluetooth
        .devices
        .iter()
        .find(|device| device.connected)
        .map(|device| device.name.clone())
        .unwrap_or_else(|| "Select a paired device".into());
    let bluetooth_devices = bluetooth
        .devices
        .iter()
        .enumerate()
        .map(|(index, device)| {
            let state = if device.connected {
                "CONNECTED"
            } else if device.paired {
                "PAIRED"
            } else {
                "NEW"
            };
            (
                format!("{} · {state}", device.name),
                ControlMessage::BluetoothDevice(index),
            )
        })
        .collect::<Vec<_>>();
    let bluetooth_detail = if !bluetooth.available {
        "UNAVAILABLE"
    } else if !bluetooth.powered {
        "POWERED OFF"
    } else if bluetooth.discovering {
        "SCANNING"
    } else {
        "READY"
    };

    let root = ui! {
        <Column background={LinearGradient::vertical(BACKGROUND_TOP, BACKGROUND_BOTTOM)}
            padding={Insets::all(20.0)} gap={14.0}>
            <Header color={PRIMARY}>{"Control Center"}</Header>
            <Container background={CARD} border={(CARD_BORDER, 1.0)} padding={Insets::all(16.0)}>
                <Column gap={8.0}>
                    <Row height={25.0}>
                        <Text scale={2.3} color={PRIMARY}>{network_name}</Text>
                        <ActionButton message={ControlMessage::WifiPower}
                            label={if network.enabled { "ON" } else { "OFF" }} width={54.0} />
                    </Row>
                    <Text scale={1.35} color={ACCENT}>{network_detail}</Text>
                    <Dropdown id={"wifi-dropdown"} on_toggle={ControlMessage::WifiDropdown}
                        selected={selected_network} options={wifi_networks}
                        colors_triplet={(0x27344c, 0x34445f, PRIMARY)} />
                </Column>
            </Container>
            <Container background={CARD} border={(CARD_BORDER, 1.0)} padding={Insets::all(14.0)}>
                <Column gap={8.0}>
                    <Row height={22.0}>
                        <Text scale={2.0} color={PRIMARY}>{"Audio"}</Text>
                        <Text scale={1.5} color={SECONDARY}>{format!("{}%", audio.volume_percent)}</Text>
                    </Row>
                    <Slider id={"audio-volume"} value={f32::from(audio.volume_percent) / 100.0}
                        on_change={audio_volume_message} colors_triplet={(0x354158, ACCENT, PRIMARY)} />
                    <Dropdown id={"audio-dropdown"} on_toggle={ControlMessage::AudioDropdown}
                        selected={selected_audio} options={audio_devices}
                        colors_triplet={(0x27344c, 0x34445f, PRIMARY)} />
                </Column>
            </Container>
            <Container background={CARD} border={(CARD_BORDER, 1.0)} padding={Insets::all(14.0)}>
                <Column gap={8.0}>
                    <Row height={25.0}>
                        <Text scale={2.0} color={PRIMARY}>{"Bluetooth"}</Text>
                        <ActionButton message={ControlMessage::BluetoothDiscovery}
                            label={if bluetooth.discovering { "STOP" } else { "SCAN" }} width={64.0} />
                        <ActionButton message={ControlMessage::BluetoothPower}
                            label={if bluetooth.powered { "ON" } else { "OFF" }} width={54.0} />
                    </Row>
                    <Text scale={1.25} color={SECONDARY}>{bluetooth_detail}</Text>
                    <Dropdown id={"bluetooth-dropdown"} on_toggle={ControlMessage::BluetoothDropdown}
                        selected={selected_bluetooth} options={bluetooth_devices}
                        colors_triplet={(0x27344c, 0x34445f, PRIMARY)} />
                </Column>
            </Container>
        </Column>
    };
    UiTree::layout_with_state(root, Rect::new(0.0, 0.0, size.0, size.1), state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_network_exposes_status_and_power_action() {
        let network = platform::NetworkStatus {
            available: true,
            enabled: true,
            connected: true,
            name: "SukiAlan".into(),
            signal_percent: 82,
            networks: Vec::new(),
        };
        let bluetooth = platform::BluetoothStatus::default();
        let audio = platform::AudioStatus::default();
        let mut state = UiStateStore::default();
        let ui = build_ui(
            ControlCenterView {
                network: &network,
                bluetooth: &bluetooth,
                audio: &audio,
            },
            (WIDTH as f32, HEIGHT as f32),
            &mut state,
        );

        assert!(ui.commands().iter().any(|command| matches!(
            command,
            nickel_ui::PaintCommand::Text { text, .. } if text == "SukiAlan"
        )));
        assert!(ui.commands().iter().any(|command| matches!(
            command,
            nickel_ui::PaintCommand::Text { text, .. } if text == "CONNECTED  ·  82% SIGNAL"
        )));
        let power = ui
            .message_rect(&ControlMessage::WifiPower)
            .expect("connected Wi-Fi should expose its power action");
        assert_eq!(
            ui.message_at(Point {
                x: power.origin.x + power.size.width * 0.5,
                y: power.origin.y + power.size.height * 0.5,
            }),
            Some(&ControlMessage::WifiPower)
        );
    }
}
