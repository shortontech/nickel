//! Runtime-neutral shared-memory presentation for shell surfaces.

use std::{cell::RefCell, num::NonZeroU32};

use nickel_ui::backend::PaintCommand;
use nickel_ui::{DamageRegion, PresenterCacheDiagnostics, SdlComponentRenderer};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};

#[derive(Clone, Copy, Debug)]
struct DisplayHandleSource(RawDisplayHandle);

// SAFETY: the runtime owns the native display connection and, by the
// `SharedGraphics::new` contract, outlives the softbuffer context. The copied
// handle is only consumed on the shell thread.
unsafe impl Send for DisplayHandleSource {}
unsafe impl Sync for DisplayHandleSource {}

impl HasDisplayHandle for DisplayHandleSource {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: `SharedGraphics::new` requires the runtime to keep this
        // display handle valid for the presentation lifetime.
        Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
    }
}

#[derive(Clone, Copy, Debug)]
struct WindowHandleSource(RawWindowHandle);

// SAFETY: presentation remains on the shell thread and, by the
// `SoftbufferPresenter::new` contract, the corresponding native window
// outlives the lightweight softbuffer surface.
unsafe impl Send for WindowHandleSource {}
unsafe impl Sync for WindowHandleSource {}

impl HasWindowHandle for WindowHandleSource {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the constructor contract keeps the owning native window
        // alive until after this surface.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// Pixel and logical dimensions needed to rasterize a shell surface.
///
/// Keeping geometry explicit prevents the presentation layer from depending on
/// a particular window runtime's sizing API.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationGeometry {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub logical_width: u32,
    pub logical_height: u32,
}

impl PresentationGeometry {
    fn nonzero_pixel_size(self) -> (NonZeroU32, NonZeroU32) {
        (
            NonZeroU32::new(self.pixel_width.max(1)).expect("nonzero shell width"),
            NonZeroU32::new(self.pixel_height.max(1)).expect("nonzero shell height"),
        )
    }

    fn scale(self) -> f32 {
        self.pixel_width.max(1) as f32 / self.logical_width.max(1) as f32
    }
}

/// The shell owns one rasterizer and one native display context regardless of
/// how many windows it exposes. Per-window presenters contain no renderer.
pub struct SharedGraphics {
    context: softbuffer::Context<DisplayHandleSource>,
    renderer: RefCell<SdlComponentRenderer>,
}

impl SharedGraphics {
    /// Creates display-scoped presentation state.
    ///
    /// # Safety
    ///
    /// The native display that supplied `display` must remain valid until this
    /// value and every presenter created from it have been dropped.
    pub unsafe fn new(display: DisplayHandle<'_>) -> Result<Self, String> {
        let context = softbuffer::Context::new(DisplayHandleSource(display.as_raw()))
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

pub struct SoftbufferPresenter {
    surface: softbuffer::Surface<DisplayHandleSource, WindowHandleSource>,
}

impl SoftbufferPresenter {
    /// Creates a surface for `window`.
    ///
    /// # Safety
    ///
    /// The native window that supplied the handle must remain valid until this
    /// presenter is dropped. It must also belong to `graphics`' display.
    pub unsafe fn new(window: WindowHandle<'_>, graphics: &SharedGraphics) -> Result<Self, String> {
        let surface =
            softbuffer::Surface::new(&graphics.context, WindowHandleSource(window.as_raw()))
                .map_err(|error| error.to_string())?;
        Ok(Self { surface })
    }

    pub fn present(
        &mut self,
        geometry: PresentationGeometry,
        graphics: &SharedGraphics,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let (width, height) = geometry.nonzero_pixel_size();
        let scale = geometry.scale();
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

#[cfg(test)]
mod tests {
    use super::PresentationGeometry;

    #[test]
    fn geometry_preserves_fractional_runtime_scale() {
        let geometry = PresentationGeometry {
            pixel_width: 1500,
            pixel_height: 900,
            logical_width: 1000,
            logical_height: 600,
        };

        assert_eq!(geometry.scale(), 1.5);
        assert_eq!(geometry.nonzero_pixel_size().0.get(), 1500);
        assert_eq!(geometry.nonzero_pixel_size().1.get(), 900);
    }

    #[test]
    fn zero_sized_runtime_geometry_is_safe_to_present() {
        let geometry = PresentationGeometry {
            pixel_width: 0,
            pixel_height: 0,
            logical_width: 0,
            logical_height: 0,
        };

        assert_eq!(geometry.scale(), 1.0);
        assert_eq!(geometry.nonzero_pixel_size().0.get(), 1);
        assert_eq!(geometry.nonzero_pixel_size().1.get(), 1);
    }
}
