use std::{cell::RefCell, num::NonZeroU32};

use nickel_ui::backend::PaintCommand;
use nickel_ui::{DamageRegion, PresenterCacheDiagnostics, SdlComponentRenderer};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use sdl3::video::Window;

#[derive(Clone, Copy, Debug)]
struct DisplayHandleSource(RawDisplayHandle);

// SAFETY: SDL owns the native display connection and outlives the shared
// softbuffer context. The copied handle is only consumed on the shell thread.
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
// window outlives the lightweight softbuffer surface.
unsafe impl Send for WindowHandleSource {}
unsafe impl Sync for WindowHandleSource {}

impl HasWindowHandle for WindowHandleSource {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the owning SDL window remains alive until after this surface.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// The shell owns one rasterizer and one native display context regardless of
/// how many windows it exposes. Per-window presenters contain no renderer.
pub struct SharedSdlGraphics {
    context: softbuffer::Context<DisplayHandleSource>,
    renderer: RefCell<SdlComponentRenderer>,
}

impl SharedSdlGraphics {
    pub fn new(window: &Window) -> Result<Self, String> {
        let display = window
            .display_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let context = softbuffer::Context::new(DisplayHandleSource(display))
            .map_err(|error| error.to_string())?;
        tracing::info!("shared-memory shell renderer initialized");
        Ok(Self {
            context,
            renderer: RefCell::new(SdlComponentRenderer::new_pixel_buffer(1, 1, 1.0)),
        })
    }

    pub fn cache_diagnostics(&self) -> PresenterCacheDiagnostics {
        self.renderer.borrow().cache_diagnostics()
    }
}

pub struct SdlGpuPresenter {
    surface: softbuffer::Surface<DisplayHandleSource, WindowHandleSource>,
}

impl SdlGpuPresenter {
    pub fn new(window: &Window, graphics: &SharedSdlGraphics) -> Result<Self, String> {
        let handle = window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let surface = softbuffer::Surface::new(&graphics.context, WindowHandleSource(handle))
            .map_err(|error| error.to_string())?;
        Ok(Self { surface })
    }

    pub fn present(
        &mut self,
        window: &Window,
        graphics: &SharedSdlGraphics,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let (width, height) = window.size_in_pixels();
        let width = NonZeroU32::new(width.max(1)).expect("nonzero shell width");
        let height = NonZeroU32::new(height.max(1)).expect("nonzero shell height");
        let logical_width = window.size().0.max(1);
        let scale = width.get() as f32 / logical_width as f32;
        let mut renderer = graphics.renderer.borrow_mut();
        renderer.resize(width.get(), height.get(), scale);
        // The rasterizer is shared by multiple windows, so its prior display
        // list belongs to a potentially different surface.
        renderer.invalidate();
        let damage = renderer.render(commands);
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
        for (target, pixel) in buffer.iter_mut().zip(renderer.pixels()) {
            *target = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        buffer.present().map_err(|error| error.to_string())?;
        Ok(damage)
    }
}
