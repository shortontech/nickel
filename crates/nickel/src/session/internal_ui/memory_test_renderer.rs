//! Import-only renderer for exercising Smithay's actual memory-buffer ownership
//! and per-context damage tracking without requiring a graphics device.
use std::convert::Infallible;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Color32F, ContextId, DebugFlags, Frame, ImportMem, Renderer, RendererSuper, Texture,
            TextureFilter, sync::SyncPoint,
        },
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

#[derive(Clone, Debug)]
pub(super) struct MemoryTexture(Size<i32, Buffer>);

impl Texture for MemoryTexture {
    fn width(&self) -> u32 {
        self.0.w as u32
    }
    fn height(&self) -> u32 {
        self.0.h as u32
    }
    fn format(&self) -> Option<Fourcc> {
        Some(Fourcc::Abgr8888)
    }
}

#[derive(Debug)]
pub(super) struct MemoryTestRenderer {
    context: ContextId<MemoryTexture>,
    pub imports: usize,
    pub updates: Vec<Rectangle<i32, Buffer>>,
}

impl Default for MemoryTestRenderer {
    fn default() -> Self {
        Self {
            context: ContextId::new(),
            imports: 0,
            updates: Vec::new(),
        }
    }
}

impl RendererSuper for MemoryTestRenderer {
    type Error = Infallible;
    type TextureId = MemoryTexture;
    type Framebuffer<'buffer> = MemoryTexture;
    type Frame<'frame, 'buffer>
        = Self
    where
        'buffer: 'frame,
        Self: 'frame;
}

impl Renderer for MemoryTestRenderer {
    fn context_id(&self) -> ContextId<MemoryTexture> {
        self.context.clone()
    }
    fn downscale_filter(&mut self, _: TextureFilter) -> Result<(), Infallible> {
        Ok(())
    }
    fn upscale_filter(&mut self, _: TextureFilter) -> Result<(), Infallible> {
        Ok(())
    }
    fn set_debug_flags(&mut self, _: DebugFlags) {}
    fn debug_flags(&self) -> DebugFlags {
        DebugFlags::empty()
    }
    fn render<'frame, 'buffer>(
        &'frame mut self,
        _: &'frame mut MemoryTexture,
        _: Size<i32, Physical>,
        _: Transform,
    ) -> Result<Self, Infallible>
    where
        'buffer: 'frame,
    {
        unreachable!("import tests do not draw frames")
    }
    fn wait(&mut self, _: &SyncPoint) -> Result<(), Infallible> {
        unreachable!("import tests do not submit GPU work")
    }
}

impl ImportMem for MemoryTestRenderer {
    fn import_memory(
        &mut self,
        data: &[u8],
        format: Fourcc,
        size: Size<i32, Buffer>,
        flipped: bool,
    ) -> Result<MemoryTexture, Infallible> {
        assert_eq!(format, Fourcc::Abgr8888);
        assert!(!flipped);
        assert_eq!(data.len(), size.w as usize * size.h as usize * 4);
        self.imports += 1;
        Ok(MemoryTexture(size))
    }
    fn update_memory(
        &mut self,
        texture: &MemoryTexture,
        data: &[u8],
        region: Rectangle<i32, Buffer>,
    ) -> Result<(), Infallible> {
        assert_eq!(
            data.len(),
            texture.width() as usize * texture.height() as usize * 4
        );
        assert!(Rectangle::from_size(texture.0).contains_rect(region));
        self.updates.push(region);
        Ok(())
    }
    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new(std::iter::once(Fourcc::Abgr8888))
    }
}

impl Frame for MemoryTestRenderer {
    type Error = Infallible;
    type TextureId = MemoryTexture;
    fn context_id(&self) -> ContextId<MemoryTexture> {
        self.context.clone()
    }
    fn clear(&mut self, _: Color32F, _: &[Rectangle<i32, Physical>]) -> Result<(), Infallible> {
        unreachable!("import tests do not draw frames")
    }
    fn draw_solid(
        &mut self,
        _: Rectangle<i32, Physical>,
        _: &[Rectangle<i32, Physical>],
        _: Color32F,
    ) -> Result<(), Infallible> {
        unreachable!("import tests do not draw frames")
    }
    fn render_texture_from_to(
        &mut self,
        _: &MemoryTexture,
        _: Rectangle<f64, Buffer>,
        _: Rectangle<i32, Physical>,
        _: &[Rectangle<i32, Physical>],
        _: &[Rectangle<i32, Physical>],
        _: Transform,
        _: f32,
    ) -> Result<(), Infallible> {
        unreachable!("import tests do not draw frames")
    }
    fn transformation(&self) -> Transform {
        Transform::Normal
    }
    fn output_size(&self) -> Size<i32, Physical> {
        Size::default()
    }
    fn wait(&mut self, _: &SyncPoint) -> Result<(), Infallible> {
        unreachable!("import tests do not submit GPU work")
    }
    fn finish(self) -> Result<SyncPoint, Infallible> {
        unreachable!("import tests do not submit GPU work")
    }
}
