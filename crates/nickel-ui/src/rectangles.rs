use std::mem;

use bytemuck::{Pod, Zeroable};

use crate::layout;

const MAX_VERTICES: usize = 12;

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

    pub fn update_hover(
        &mut self,
        queue: &wgpu::Queue,
        surface_size: (u32, u32),
        hovered: Option<usize>,
    ) {
        let mut vertices = Vec::with_capacity(MAX_VERTICES);
        if let Some(index) = hovered {
            let top = layout::RESULT_TOP + index as f32 * layout::RESULT_STRIDE;
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    layout::RESULT_LEFT,
                    top,
                    surface_size.0 as f32 - layout::RESULT_RIGHT_INSET,
                    top + layout::RESULT_HEIGHT,
                ],
                [0.18, 0.42, 0.78, 1.0],
            );
            add_rectangle(
                &mut vertices,
                surface_size,
                [
                    layout::RESULT_LEFT + 2.0,
                    top + 2.0,
                    surface_size.0 as f32 - layout::RESULT_RIGHT_INSET - 2.0,
                    top + layout::RESULT_HEIGHT - 2.0,
                ],
                [0.055, 0.07, 0.105, 1.0],
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
