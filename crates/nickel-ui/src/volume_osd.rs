use std::sync::Arc;

use nickel_components::{
    Column, ComponentGpu, Container, Insets, Rect, Row, Text, TextAlign, UiTree,
};
use winit::window::Window;

use crate::graphics::SharedGraphics;

pub const WIDTH: u32 = 120;
pub const HEIGHT: u32 = 58;

const BACKGROUND: u32 = 0x20242d;
const BORDER: u32 = 0x4b5260;
const TRACK: u32 = 0x555b68;
const FILL: u32 = 0x9b72d0;
const TEXT: u32 = 0xf4f5f7;

pub struct VolumeOsdGpu {
    gpu: ComponentGpu,
}

impl VolumeOsdGpu {
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
        })
    }

    pub fn render(&mut self, volume_percent: u8, muted: bool) {
        let fill_width = if muted {
            0.0
        } else {
            f32::from(volume_percent.min(100)) * 0.98
        };
        let label = if muted {
            "Muted".to_owned()
        } else {
            format!("{volume_percent}%")
        };
        let root = Column::new()
            .background(BACKGROUND)
            .padding(Insets::all(10.0))
            .gap(7.0)
            .child(
                Text::new(label)
                    .scale(1.6)
                    .height(20.0)
                    .align(TextAlign::Center)
                    .color(TEXT),
            )
            .child(
                Container::new()
                    .background(TRACK)
                    .border(BORDER, 1.0)
                    .padding(Insets::all(1.0))
                    .height(8.0)
                    .child(
                        Row::new()
                            .height(6.0)
                            .child(Container::new().background(FILL).width(fill_width)),
                    ),
            );
        let ui = UiTree::layout(root, Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32));
        if let Err(error) = self.gpu.render(ui.commands()) {
            tracing::warn!(%error, "failed to render volume indicator");
        }
    }
}
