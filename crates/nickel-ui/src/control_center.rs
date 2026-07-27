use std::sync::Arc;

use nickel_components::{
    Column, ComponentGpu, Container, Dropdown, Header, Insets, LinearGradient, Point, Rect, Row,
    Slider, Text, UiTree,
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

pub struct ControlCenterGpu {
    gpu: ComponentGpu,
    size: (u32, u32),
    network: platform::NetworkStatus,
    bluetooth: platform::BluetoothStatus,
    audio: platform::AudioStatus,
    ui: UiTree,
    cursor: Point,
    audio_dropdown_open: bool,
    wifi_dropdown_open: bool,
    bluetooth_dropdown_open: bool,
    volume_dragging: bool,
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
            cursor: Point { x: 0.0, y: 0.0 },
            audio_dropdown_open: false,
            wifi_dropdown_open: false,
            bluetooth_dropdown_open: false,
            volume_dragging: false,
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
                wifi_dropdown_open: self.wifi_dropdown_open,
                audio_dropdown_open: self.audio_dropdown_open,
                bluetooth_dropdown_open: self.bluetooth_dropdown_open,
            },
            (self.size.0 as f32, self.size.1 as f32),
        );
        if let Err(error) = self.gpu.render(self.ui.commands()) {
            eprintln!("failed to render Control Center: {error}");
        }
    }

    pub fn cursor_moved(&mut self, x: f32, y: f32) {
        self.cursor = Point { x, y };
        if self.volume_dragging
            && let Some(fraction) = self.ui.horizontal_fraction_for_action("audio-volume", x)
        {
            self.set_volume_fraction(fraction);
        }
    }

    pub fn pointer_pressed(&mut self) -> bool {
        let Some((action, fraction)) = self
            .ui
            .action_at_with_horizontal_fraction(self.cursor)
            .map(|(action, fraction)| (action.to_owned(), fraction))
        else {
            self.audio_dropdown_open = false;
            self.wifi_dropdown_open = false;
            self.bluetooth_dropdown_open = false;
            return true;
        };
        if action == "wifi-power" {
            if platform::set_wifi_enabled(!self.network.enabled) {
                self.network.enabled = !self.network.enabled;
            }
            return true;
        }
        if action == "wifi-network" {
            self.wifi_dropdown_open = !self.wifi_dropdown_open;
            self.audio_dropdown_open = false;
            self.bluetooth_dropdown_open = false;
            return true;
        }
        if let Some(index) = action
            .strip_prefix("wifi-network:option:")
            .and_then(|index| index.parse::<usize>().ok())
        {
            if let Some(network) = self.network.networks.get(index)
                && network.saved
                && !network.connected
            {
                let _ = platform::activate_wifi_network(&network.id);
            }
            self.wifi_dropdown_open = false;
            return true;
        }
        if action == "audio-volume" {
            self.volume_dragging = true;
            self.set_volume_fraction(fraction);
            return true;
        }
        if action == "audio-device" {
            self.audio_dropdown_open = !self.audio_dropdown_open;
            self.wifi_dropdown_open = false;
            self.bluetooth_dropdown_open = false;
            return true;
        }
        if let Some(index) = action
            .strip_prefix("audio-device:option:")
            .and_then(|index| index.parse::<usize>().ok())
        {
            if let Some(device) = self.audio.devices.get(index)
                && platform::select_audio_device(&device.id)
            {
                self.audio = platform::audio_status();
            }
            self.audio_dropdown_open = false;
            return true;
        }
        if action == "bluetooth-power" {
            if platform::set_bluetooth_powered(!self.bluetooth.powered) {
                self.bluetooth.powered = !self.bluetooth.powered;
            }
            return true;
        }
        if action == "bluetooth-discovery" {
            if platform::set_bluetooth_discovery(!self.bluetooth.discovering) {
                self.bluetooth.discovering = !self.bluetooth.discovering;
            }
            return true;
        }
        if action == "bluetooth-device" {
            self.bluetooth_dropdown_open = !self.bluetooth_dropdown_open;
            self.wifi_dropdown_open = false;
            self.audio_dropdown_open = false;
            return true;
        }
        if let Some(index) = action
            .strip_prefix("bluetooth-device:option:")
            .and_then(|index| index.parse::<usize>().ok())
        {
            if let Some(device) = self.bluetooth.devices.get(index)
                && device.paired
            {
                let _ = platform::toggle_bluetooth_device(&device.id);
            }
            self.bluetooth_dropdown_open = false;
            return true;
        }
        false
    }

    pub fn pointer_released(&mut self) -> bool {
        std::mem::take(&mut self.volume_dragging)
    }

    pub fn is_volume_dragging(&self) -> bool {
        self.volume_dragging
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
    wifi_dropdown_open: bool,
    audio_dropdown_open: bool,
    bluetooth_dropdown_open: bool,
}

fn build_ui(view: ControlCenterView<'_>, size: (f32, f32)) -> UiTree {
    let ControlCenterView {
        network,
        bluetooth,
        audio,
        wifi_dropdown_open,
        audio_dropdown_open,
        bluetooth_dropdown_open,
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
        .find(|device| device.is_default)
        .map(|device| device.name.clone())
        .unwrap_or_else(|| "No audio output".into());
    let audio_devices: Vec<_> = audio
        .devices
        .iter()
        .map(|device| device.name.clone())
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
        .map(|candidate| {
            let saved = if candidate.saved { "" } else { " · NEW" };
            format!(
                "{} · {}%{}",
                candidate.name, candidate.signal_percent, saved
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
        .map(|device| {
            let state = if device.connected {
                "CONNECTED"
            } else if device.paired {
                "PAIRED"
            } else {
                "NEW"
            };
            format!("{} · {state}", device.name)
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

    let root = Column::new()
        .background(LinearGradient::vertical(BACKGROUND_TOP, BACKGROUND_BOTTOM))
        .padding(Insets::all(20.0))
        .gap(14.0)
        .child(Header::new("Control Center").color(PRIMARY))
        .child(
            Container::new()
                .background(CARD)
                .border(CARD_BORDER, 1.0)
                .padding(Insets::all(16.0))
                .height(if wifi_dropdown_open {
                    130.0 + wifi_networks.len() as f32 * 36.0
                } else {
                    130.0
                })
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(
                            Row::new()
                                .height(25.0)
                                .child(Text::new(network_name).scale(2.3).color(PRIMARY))
                                .child(action_button(
                                    "wifi-power",
                                    if network.enabled { "ON" } else { "OFF" },
                                    54.0,
                                )),
                        )
                        .child(Text::new(network_detail).scale(1.35).color(ACCENT))
                        .child(
                            Dropdown::new("wifi-network", selected_network, wifi_networks)
                                .expanded(wifi_dropdown_open)
                                .colors(0x27344c, 0x34445f, PRIMARY),
                        ),
                ),
        )
        .child(
            Container::new()
                .background(CARD)
                .border(CARD_BORDER, 1.0)
                .padding(Insets::all(14.0))
                .height(if audio_dropdown_open {
                    116.0 + audio_devices.len() as f32 * 36.0
                } else {
                    116.0
                })
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(
                            Row::new()
                                .height(22.0)
                                .child(Text::new("Audio").scale(2.0).color(PRIMARY))
                                .child(
                                    Text::new(format!("{}%", audio.volume_percent))
                                        .scale(1.5)
                                        .color(SECONDARY),
                                ),
                        )
                        .child(
                            Slider::new("audio-volume", f32::from(audio.volume_percent) / 100.0)
                                .colors(0x354158, ACCENT, PRIMARY),
                        )
                        .child(
                            Dropdown::new("audio-device", selected_audio, audio_devices)
                                .expanded(audio_dropdown_open)
                                .colors(0x27344c, 0x34445f, PRIMARY),
                        ),
                ),
        )
        .child(
            Container::new()
                .background(CARD)
                .border(CARD_BORDER, 1.0)
                .padding(Insets::all(14.0))
                .height(if bluetooth_dropdown_open {
                    130.0 + bluetooth_devices.len() as f32 * 36.0
                } else {
                    130.0
                })
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(
                            Row::new()
                                .height(25.0)
                                .child(Text::new("Bluetooth").scale(2.0).color(PRIMARY))
                                .child(action_button(
                                    "bluetooth-discovery",
                                    if bluetooth.discovering {
                                        "STOP"
                                    } else {
                                        "SCAN"
                                    },
                                    64.0,
                                ))
                                .child(action_button(
                                    "bluetooth-power",
                                    if bluetooth.powered { "ON" } else { "OFF" },
                                    54.0,
                                )),
                        )
                        .child(Text::new(bluetooth_detail).scale(1.25).color(SECONDARY))
                        .child(
                            Dropdown::new(
                                "bluetooth-device",
                                selected_bluetooth,
                                bluetooth_devices,
                            )
                            .expanded(bluetooth_dropdown_open)
                            .colors(0x27344c, 0x34445f, PRIMARY),
                        ),
                ),
        );
    UiTree::layout(root, Rect::new(0.0, 0.0, size.0, size.1))
}

fn action_button(action: &str, label: &str, width: f32) -> Container {
    Container::new()
        .background(0x34445f)
        .border(CARD_BORDER, 1.0)
        .padding(Insets {
            top: 5.0,
            right: 8.0,
            bottom: 5.0,
            left: 8.0,
        })
        .width(width)
        .action(action)
        .child(Text::new(label).scale(1.15).color(PRIMARY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_network_is_rendered() {
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
        let ui = build_ui(
            ControlCenterView {
                network: &network,
                bluetooth: &bluetooth,
                audio: &audio,
                wifi_dropdown_open: false,
                audio_dropdown_open: false,
                bluetooth_dropdown_open: false,
            },
            (WIDTH as f32, HEIGHT as f32),
        );

        assert!(ui.commands().iter().any(|command| matches!(
            command,
            nickel_components::PaintCommand::Text { text, .. } if text == "SukiAlan"
        )));
    }
}
