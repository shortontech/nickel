//! Bounded, fence-backed window rendering shared by compositor capture consumers.
//! Submission never waits for completion; the renderer owner polls the fence before mapping.
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Frame, Offscreen, Renderer,
            element::{
                AsRenderElements,
                surface::WaylandSurfaceRenderElement,
                utils::{ConstrainAlign, ConstrainScaleBehavior, constrain_render_elements},
            },
            gles::{GlesMapping, GlesRenderer, GlesTexture},
            sync::SyncPoint,
            utils::draw_render_elements,
        },
    },
    utils::{Buffer, Rectangle, Scale, Transform},
};

pub(crate) struct SubmittedWindowCapture {
    // Retain the draw target until the readback completes or is retired.
    pub(crate) texture: GlesTexture,
    pub(crate) mapping: GlesMapping,
    pub(crate) fence: SyncPoint,
    pub(crate) dimensions: (u16, u16),
}

fn valid_capture_dimensions((width, height): (u16, u16)) -> bool {
    width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && u32::from(width) * u32::from(height) <= 16_777_216
}

pub(crate) fn capture_supported(renderer: &GlesRenderer) -> bool {
    use smithay::backend::renderer::gles::Capability;
    renderer.capabilities().contains(&Capability::ExportFence)
        && renderer.capabilities().contains(&Capability::Fencing)
}

pub(crate) fn submit_window_capture(
    renderer: &mut GlesRenderer,
    window: &smithay::desktop::Window,
    dimensions: (u16, u16),
) -> Option<SubmittedWindowCapture> {
    let geometry = window.geometry();
    if !capture_supported(renderer)
        || !valid_capture_dimensions(dimensions)
        || geometry.size.w <= 0
        || geometry.size.h <= 0
    {
        return None;
    }
    let elements = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
        renderer,
        (-geometry.loc.x, -geometry.loc.y).into(),
        Scale::from(1.0),
        1.0,
    );
    let damage = Rectangle::from_size((i32::from(dimensions.0), i32::from(dimensions.1)).into());
    let reference = Rectangle::from_size(geometry.size.to_physical(1));
    let elements = constrain_render_elements(
        elements,
        (0, 0),
        damage,
        reference,
        ConstrainScaleBehavior::Fit,
        ConstrainAlign::TOP | ConstrainAlign::BOTTOM | ConstrainAlign::LEFT | ConstrainAlign::RIGHT,
        1.0,
    )
    .collect::<Vec<_>>();
    submit_elements_capture(renderer, &elements, dimensions, 1.0)
}

pub(crate) fn submit_elements_capture<
    E: smithay::backend::renderer::element::RenderElement<GlesRenderer>,
>(
    renderer: &mut GlesRenderer,
    elements: &[E],
    dimensions: (u16, u16),
    scale: f64,
) -> Option<SubmittedWindowCapture> {
    (|| {
        // Do not knowingly enter the renderer's synchronous no-fence fallback.
        // Fencing also guards shared-context texture import/draw paths that can
        // otherwise call glFinish before the preview's completion fence exists.
        // The patched try_finish path rejects runtime fence-export failure
        // without falling back to a synchronous completion wait.
        if !capture_supported(renderer) {
            return None;
        }
        if !valid_capture_dimensions(dimensions) || !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let width = i32::from(dimensions.0);
        let height = i32::from(dimensions.1);
        let mut texture = <GlesRenderer as Offscreen<GlesTexture>>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            (width, height).into(),
        )
        .ok()?;
        let mut framebuffer = renderer.bind(&mut texture).ok()?;
        let damage = Rectangle::from_size((width, height).into());
        let frame = renderer
            .render(&mut framebuffer, (width, height).into(), Transform::Normal)
            .ok()?;
        let _submitted = crate::session::preview_submission::finish_preview_submission(
            frame,
            |frame| {
                frame.clear(Color32F::new(0.03, 0.04, 0.06, 1.0), &[damage])?;
                draw_render_elements(frame, scale, elements, &[damage]).map(|_| ())
            },
            smithay::backend::renderer::gles::GlesFrame::try_finish,
        )
        .ok()?;
        let region = Rectangle::<i32, Buffer>::from_size((width, height).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
            .ok()?;
        drop(framebuffer);
        // The fence must follow ReadPixels, not merely the thumbnail draw. Map
        // is deferred until this fence signals in a later event-loop turn.
        let display = renderer.egl_context().display().clone();
        let fence = renderer
            .with_context(|gl| {
                let fence = smithay::backend::egl::fence::EGLFence::create(&display).ok()?;
                // SAFETY: with_context made this renderer's GL context current;
                // Flush submits work without waiting or changing GL binding state.
                unsafe { gl.Flush() };
                Some(smithay::backend::renderer::sync::SyncPoint::from(fence))
            })
            .ok()??;
        Some(SubmittedWindowCapture {
            texture,
            mapping,
            fence,
            dimensions,
        })
    })()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_allocation_accepts_4k_but_rejects_empty_and_excessive_buffers() {
        assert!(valid_capture_dimensions((3840, 2160)));
        assert!(valid_capture_dimensions((240, 135)));
        for dimensions in [(0, 1), (1, 0), (8193, 1), (1, 8193), (8192, 8192)] {
            assert!(!valid_capture_dimensions(dimensions));
        }
    }
}
