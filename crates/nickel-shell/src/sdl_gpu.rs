use std::{num::NonZeroU32, sync::Arc};

use nickel_ui::{DamageRegion, PaintCommand, SdlComponentRenderer};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use sdl3::video::Window;

#[derive(Clone, Copy, Debug)]
struct DisplayHandleSource(RawDisplayHandle);

// SAFETY: SDL owns the native display connection and outlives the shared
// softbuffer context. The copied handle is only consumed by softbuffer.
unsafe impl Send for DisplayHandleSource {}
unsafe impl Sync for DisplayHandleSource {}

impl HasDisplayHandle for DisplayHandleSource {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: SDL keeps this display handle valid for the shell lifetime.
        Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
    }
}

#[derive(Clone, Copy, Debug)]
struct WindowHandleSource(RawWindowHandle);

// SAFETY: presentation remains on the shell thread and the corresponding SDL
// window outlives the presenter containing this copied handle.
unsafe impl Send for WindowHandleSource {}
unsafe impl Sync for WindowHandleSource {}

impl HasWindowHandle for WindowHandleSource {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the owning SDL window remains alive until after this surface.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

pub struct SharedSdlGraphics {
    context: softbuffer::Context<DisplayHandleSource>,
}

impl SharedSdlGraphics {
    pub fn new(window: &Window) -> Result<Arc<Self>, String> {
        let display = window
            .display_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let context = softbuffer::Context::new(DisplayHandleSource(display))
            .map_err(|error| error.to_string())?;
        tracing::info!("shared-memory shell presenter initialized");
        Ok(Arc::new(Self { context }))
    }
}

pub struct SdlGpuPresenter {
    surface: softbuffer::Surface<DisplayHandleSource, WindowHandleSource>,
    renderer: SdlComponentRenderer,
}

impl SdlGpuPresenter {
    pub fn new(window: &Window, graphics: Arc<SharedSdlGraphics>) -> Result<Self, String> {
        let handle = window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let surface = softbuffer::Surface::new(&graphics.context, WindowHandleSource(handle))
            .map_err(|error| error.to_string())?;
        let (width, height) = window.size_in_pixels();
        let logical_width = window.size().0.max(1);
        let scale = width as f32 / logical_width as f32;
        Ok(Self {
            surface,
            renderer: SdlComponentRenderer::new_pixel_buffer(width, height, scale),
        })
    }

    pub fn present(
        &mut self,
        window: &Window,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let (width, height) = window.size_in_pixels();
        let width = NonZeroU32::new(width.max(1)).expect("nonzero shell width");
        let height = NonZeroU32::new(height.max(1)).expect("nonzero shell height");
        let logical_width = window.size().0.max(1);
        let scale = width.get() as f32 / logical_width as f32;
        self.renderer.resize(width.get(), height.get(), scale);
        let damage = self.renderer.render(commands);
        if damage.is_empty() {
            return Ok(damage);
        }
        self.surface
            .resize(width, height)
            .map_err(|error| error.to_string())?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;
        for (target, pixel) in buffer.iter_mut().zip(self.renderer.pixels()) {
            *target = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        buffer.present().map_err(|error| error.to_string())?;
        Ok(damage)
    }
}
