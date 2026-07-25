use std::sync::Arc;

use nickel_components::{
    Column, ComponentGpu, Container, Header, Insets, LinearGradient, Rect, Row, Text, UiTree,
};
use winit::window::Window;

use crate::platform;

pub const WIDTH: u32 = 380;
pub const HEIGHT: u32 = 344;

const BACKGROUND_TOP: u32 = 0x202b43ff;
const BACKGROUND_BOTTOM: u32 = 0x111827ff;
const CARD: u32 = 0x2b3852ff;
const CARD_BORDER: u32 = 0x42516cff;
const PRIMARY: u32 = 0xf4f7ffff;
const SECONDARY: u32 = 0xaebbd1ff;
const ACCENT: u32 = 0x65b8ffff;

pub struct ControlCenterGpu {
    gpu: ComponentGpu,
    size: (u32, u32),
    network: platform::NetworkStatus,
}

impl ControlCenterGpu {
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        Ok(Self {
            gpu: ComponentGpu::new(window, WIDTH, HEIGHT)?,
            size: (WIDTH, HEIGHT),
            network: platform::network_status(),
        })
    }

    pub fn refresh(&mut self) {
        self.network = platform::network_status();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.gpu.resize(width, height);
    }

    pub fn render(&mut self) {
        let ui = build_ui(&self.network, self.size.0 as f32, self.size.1 as f32);
        if let Err(error) = self.gpu.render(ui.commands()) {
            eprintln!("failed to render Control Center: {error}");
        }
    }
}

fn build_ui(network: &platform::NetworkStatus, width: f32, height: f32) -> UiTree {
    let (network_name, network_detail) = if network.connected {
        (
            if network.name.is_empty() {
                "Connected".to_owned()
            } else {
                network.name.clone()
            },
            format!("CONNECTED  ·  {}% SIGNAL", network.signal_percent),
        )
    } else if network.available {
        ("Wi-Fi".to_owned(), "NOT CONNECTED".to_owned())
    } else {
        ("Wi-Fi".to_owned(), "UNAVAILABLE".to_owned())
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
                .height(92.0)
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(Text::new(network_name).scale(2.6).color(PRIMARY))
                        .child(Text::new(network_detail).scale(1.45).color(ACCENT)),
                ),
        )
        .child(
            Row::new()
                .gap(12.0)
                .height(88.0)
                .child(status_card("Audio", "COMING NEXT", 164.0))
                .child(status_card("Bluetooth", "COMING NEXT", 164.0)),
        )
        .child(
            Container::new()
                .background(CARD)
                .border(CARD_BORDER, 1.0)
                .padding(Insets::all(14.0))
                .height(62.0)
                .child(
                    Column::new()
                        .gap(5.0)
                        .child(Text::new("Notifications").scale(2.0).color(PRIMARY))
                        .child(
                            Text::new("NO NEW NOTIFICATIONS")
                                .scale(1.3)
                                .color(SECONDARY),
                        ),
                ),
        );
    UiTree::layout(root, Rect::new(0.0, 0.0, width, height))
}

fn status_card(title: &str, detail: &str, width: f32) -> Container {
    Container::new()
        .background(CARD)
        .border(CARD_BORDER, 1.0)
        .padding(Insets::all(14.0))
        .width(width)
        .child(
            Column::new()
                .gap(8.0)
                .child(Text::new(title).scale(2.0).color(PRIMARY))
                .child(Text::new(detail).scale(1.25).color(SECONDARY)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_network_is_rendered() {
        let ui = build_ui(
            &platform::NetworkStatus {
                available: true,
                connected: true,
                name: "SukiAlan".into(),
                signal_percent: 82,
            },
            WIDTH as f32,
            HEIGHT as f32,
        );

        assert!(ui.commands().iter().any(|command| matches!(
            command,
            nickel_components::PaintCommand::Text { text, .. } if text == "SukiAlan"
        )));
    }
}
