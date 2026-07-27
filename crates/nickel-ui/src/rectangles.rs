use std::mem;

use bytemuck::{Pod, Zeroable};
use nickel_core::theme::ThemePalette;

use crate::layout;

const MAX_VERTICES: usize = 512;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

pub struct RectangleRenderer {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    vertex_count: u32,
}

impl RectangleRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Nickel rectangle shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(input.position, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Nickel rectangle pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Nickel rectangle vertices"),
            size: (MAX_VERTICES * mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            vertices,
            vertex_count: 0,
        }
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        surface_size: (u32, u32),
        hovered_row: Option<usize>,
        selected_row: Option<usize>,
        selected_sidebar: usize,
        scrollbar: Option<layout::Scrollbar>,
        controller_connected: bool,
        navigation_pane: nickel_components::NavigationPane,
        palette: ThemePalette,
    ) {
        let mut vertices = Vec::with_capacity(MAX_VERTICES);
        add_vertical_gradient(
            &mut vertices,
            surface_size,
            [0.0, 0.0, surface_size.0 as f32, surface_size.1 as f32],
            color_rgba(palette.panel),
            color_rgba(palette.background),
        );
        add_rectangle(
            &mut vertices,
            surface_size,
            [0.0, 0.0, layout::SIDEBAR_WIDTH, surface_size.1 as f32],
            color_rgba(palette.panel),
        );
        add_rectangle(
            &mut vertices,
            surface_size,
            [
                layout::CONTENT_LEFT,
                22.0,
                surface_size.0 as f32 - layout::CONTENT_RIGHT_INSET,
                70.0,
            ],
            color_rgba(palette.surface),
        );
        if controller_connected {
            for (index, left) in [14.0, 60.0].into_iter().enumerate() {
                let selected = matches!(
                    (index, navigation_pane),
                    (0, nickel_components::NavigationPane::Sidebar)
                        | (1, nickel_components::NavigationPane::Content)
                );
                add_rectangle(
                    &mut vertices,
                    surface_size,
                    [
                        left,
                        surface_size.1 as f32 - 52.0,
                        left + 36.0,
                        surface_size.1 as f32 - 22.0,
                    ],
                    color_rgba(if selected {
                        palette.accent_soft
                    } else {
                        palette.surface
                    }),
                );
            }
        }
        add_rectangle(
            &mut vertices,
            surface_size,
            [
                layout::CONTENT_LEFT,
                68.0,
                surface_size.0 as f32 - layout::CONTENT_RIGHT_INSET,
                70.0,
            ],
            color_rgba(palette.accent),
        );
        let selected_sidebar = layout::sidebar_item_bounds(selected_sidebar);
        add_rectangle(
            &mut vertices,
            surface_size,
            [
                selected_sidebar.x,
                selected_sidebar.y,
                selected_sidebar.right(),
                selected_sidebar.bottom(),
            ],
            color_rgba(palette.accent_soft),
        );
        add_rectangle(
            &mut vertices,
            surface_size,
            [
                layout::SIDEBAR_WIDTH,
                surface_size.1 as f32 - 62.0,
                surface_size.0 as f32,
                surface_size.1 as f32,
            ],
            color_rgba(palette.surface),
        );
        if let Some(index) = selected_row {
            let row = layout::ResultRow::allocate(index, surface_size.0);
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    row.outer.x,
                    row.outer.y,
                    row.outer.right(),
                    row.outer.bottom(),
                ],
                color_rgba(palette.accent),
            );
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    row.outer.x + 2.0,
                    row.outer.y + 2.0,
                    row.outer.right() - 2.0,
                    row.outer.bottom() - 2.0,
                ],
                color_rgba(palette.accent_soft),
            );
        }
        if let Some(index) = hovered_row {
            let row = layout::ResultRow::allocate(index, surface_size.0);
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    row.outer.x,
                    row.outer.y,
                    row.outer.right(),
                    row.outer.bottom(),
                ],
                color_rgba(palette.text),
            );
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    row.outer.x + 2.0,
                    row.outer.y + 2.0,
                    row.outer.right() - 2.0,
                    row.outer.bottom() - 2.0,
                ],
                color_rgba(palette.surface_hover),
            );
        }
        if let Some(scrollbar) = scrollbar {
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    scrollbar.track.x,
                    scrollbar.track.y,
                    scrollbar.track.right(),
                    scrollbar.track.bottom(),
                ],
                color_rgba(palette.surface_hover),
            );
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    scrollbar.thumb.x,
                    scrollbar.thumb.y,
                    scrollbar.thumb.right(),
                    scrollbar.thumb.bottom(),
                ],
                color_rgba(palette.muted),
            );
        }
        self.vertex_count = vertices.len() as u32;
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
        }
    }

    pub fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if self.vertex_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }

    pub fn update_raw(
        &mut self,
        queue: &wgpu::Queue,
        surface_size: (u32, u32),
        rectangles: &[([f32; 4], [f32; 4])],
    ) {
        let mut vertices = Vec::with_capacity(rectangles.len() * 6);
        for (bounds, color) in rectangles.iter().take(MAX_VERTICES / 6) {
            add_rectangle(&mut vertices, surface_size, *bounds, *color);
        }
        self.vertex_count = vertices.len() as u32;
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
        }
    }
}

fn color_rgba(color: u32) -> [f32; 4] {
    [
        ((color >> 16) & 0xff) as f32 / 255.0,
        ((color >> 8) & 0xff) as f32 / 255.0,
        (color & 0xff) as f32 / 255.0,
        1.0,
    ]
}

fn add_vertical_gradient(
    vertices: &mut Vec<Vertex>,
    surface: (u32, u32),
    rect: [f32; 4],
    top: [f32; 4],
    bottom: [f32; 4],
) {
    let [left, top_edge, right, bottom_edge] = rect;
    let left = left / surface.0 as f32 * 2.0 - 1.0;
    let right = right / surface.0 as f32 * 2.0 - 1.0;
    let top_edge = 1.0 - top_edge / surface.1 as f32 * 2.0;
    let bottom_edge = 1.0 - bottom_edge / surface.1 as f32 * 2.0;
    vertices.extend_from_slice(&[
        Vertex {
            position: [left, top_edge],
            color: top,
        },
        Vertex {
            position: [left, bottom_edge],
            color: bottom,
        },
        Vertex {
            position: [right, top_edge],
            color: top,
        },
        Vertex {
            position: [right, top_edge],
            color: top,
        },
        Vertex {
            position: [left, bottom_edge],
            color: bottom,
        },
        Vertex {
            position: [right, bottom_edge],
            color: bottom,
        },
    ]);
}

fn add_rectangle(
    vertices: &mut Vec<Vertex>,
    surface_size: (u32, u32),
    bounds: [f32; 4],
    color: [f32; 4],
) {
    let [left, top, right, bottom] = bounds;
    let x1 = left / surface_size.0 as f32 * 2.0 - 1.0;
    let x2 = right / surface_size.0 as f32 * 2.0 - 1.0;
    let y1 = 1.0 - top / surface_size.1 as f32 * 2.0;
    let y2 = 1.0 - bottom / surface_size.1 as f32 * 2.0;
    for position in [[x1, y1], [x1, y2], [x2, y2], [x1, y1], [x2, y2], [x2, y1]] {
        vertices.push(Vertex { position, color });
    }
}
