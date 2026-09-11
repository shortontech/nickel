use super::*;
use crate::session::window_capture::{
    SubmittedWindowCapture, submit_elements_capture, submit_window_capture,
};
use nickel_remote_control::{DesktopPermit, capture::CapturedWindow};
use smithay::{
    backend::renderer::{
        ExportMem, ImportAll, ImportMem, element::surface::WaylandSurfaceRenderElement,
        gles::GlesRenderer,
    },
    desktop::{Window, space::space_render_elements},
};

smithay::backend::renderer::element::render_elements! {
    OutputCaptureElement<R, E> where R: ImportAll + ImportMem;
    Space=smithay::desktop::space::SpaceRenderElements<R, E>,
    Internal=crate::session::internal_ui::InternalUiRenderElement<R>,
}
type OutputCaptureRenderElement =
    OutputCaptureElement<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>;

type CaptureReply = std::sync::mpsc::SyncSender<Result<CapturedWindow, String>>;

#[derive(Clone)]
pub(super) enum CaptureTarget {
    Window(WindowId),
    Surface(nickel_remote_control::leases::ResourceId),
    Output(nickel_remote_control::leases::ResourceId),
}

impl CaptureTarget {
    fn identity(&self) -> (String, u64) {
        match self {
            Self::Window(id) => (id.0.to_string(), id.0),
            Self::Surface(id) => (id.id.clone(), id.generation),
            Self::Output(id) => (id.id.clone(), id.generation),
        }
    }
}

pub(super) struct RemoteCaptureWork {
    target: CaptureTarget,
    permit: DesktopPermit,
    reply: CaptureReply,
    submission: Option<SubmittedWindowCapture>,
    context: usize,
    renderer_generation: (u64, u64),
    capture_generation: u64,
    submitted_at_us: u64,
}

impl NickelSession {
    fn output_contains_unexcluded_protected_content(
        &self,
        output: &smithay::output::Output,
    ) -> bool {
        let protected_internal = self
            .internal_ui
            .ids_for_output(&output.name())
            .any(|surface| {
                self.internal_ui.placement(surface).is_none_or(|placement| {
                    placement.role != crate::session::InternalSurfaceRole::TrustedControl
                        && self.internal_ui.remote_access_protected(surface)
                })
            });
        if protected_internal {
            return true;
        }

        self.space.elements().any(|window| {
            if !self
                .space
                .outputs_for_element(window)
                .iter()
                .any(|candidate| candidate == output)
            {
                return false;
            }
            let id = window
                .wl_surface()
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .copied()
                .or_else(|| {
                    window
                        .x11_surface()
                        .and_then(|surface| self.x11_windows.get(&surface.window_id()))
                        .copied()
                });
            id.is_none_or(|id| self.remote_window_is_protected(id))
        })
    }

    pub(super) fn output_capture_evidence(
        &self,
        identity: &nickel_remote_control::leases::ResourceId,
    ) -> Result<smithay::output::Output, String> {
        if self.locked || self.shell_recovery_visible() {
            return Err("output capture is protected".into());
        }
        let (output, generation) = self
            .remote_output_generations
            .get(&identity.id)
            .ok_or("output generation has retired")?;
        if *generation != identity.generation
            || !self.space.outputs().any(|current| current == output)
        {
            return Err("output generation has retired".into());
        }
        let mode = output.current_mode().ok_or("output mode is unavailable")?;
        let width = u32::try_from(mode.size.w).map_err(|_| "output width exceeds limit")?;
        let height = u32::try_from(mode.size.h).map_err(|_| "output height exceeds limit")?;
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || width.saturating_mul(height) > 16_777_216
        {
            return Err("output capture dimensions exceed limit".into());
        }
        if self.output_contains_unexcluded_protected_content(output) {
            return Err("output capture contains protected content".into());
        }
        Ok(output.clone())
    }

    pub(super) fn with_output_capture_authority<T>(
        &self,
        identity: &nickel_remote_control::leases::ResourceId,
        permit: &DesktopPermit,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.output_capture_evidence(identity)?;
        permit.with_resource(
            &nickel_remote_control::leases::ResourceEvidence {
                window: None,
                surface: None,
                output: Some(identity),
                verified_application: None,
                authorized_surface_ancestors: &[],
                protected: false,
            },
            effect,
        )
    }

    pub(super) fn surface_capture_evidence(
        &self,
        identity: &nickel_remote_control::leases::ResourceId,
    ) -> Result<
        (
            nickel_ui::InternalSurfaceId,
            Option<nickel_remote_control::leases::ResourceId>,
        ),
        String,
    > {
        if self.locked || self.shell_recovery_visible() {
            return Err("shell capture is protected".into());
        }
        let runtime = self
            .internal_ui
            .resolve_surface_identity(&identity.id, identity.generation)
            .ok_or("shell surface generation has retired")?;
        let shell = self.internal_shell.as_ref().ok_or("shell is unavailable")?;
        let entry = shell
            .surfaces()
            .iter()
            .find(|entry| self.internal_shell_surfaces.get(&entry.id) == Some(&runtime))
            .ok_or("surface is not an ordinary shell surface")?;
        if shell.remote_access_protected(entry.id)
            || !self.internal_ui.is_visible(runtime)
            || self.internal_ui.remote_access_protected(runtime)
        {
            return Err("shell surface is hidden or protected".into());
        }
        let placement = self
            .internal_ui
            .placement(runtime)
            .ok_or("shell surface is unavailable")?;
        let output = placement.output.as_ref().and_then(|name| {
            let (native, generation) = self.remote_output_generations.get(name)?;
            if !self.space.outputs().any(|current| current == native) {
                return None;
            }
            let area = self.output_geometry_named(name)?;
            let (x, y, w, h) = placement.geometry;
            (w > 0
                && h > 0
                && i64::from(x) >= i64::from(area.x)
                && i64::from(y) >= i64::from(area.y)
                && i64::from(x) + i64::from(w) <= i64::from(area.x) + i64::from(area.width)
                && i64::from(y) + i64::from(h) <= i64::from(area.y) + i64::from(area.height))
            .then(|| nickel_remote_control::leases::ResourceId {
                id: name.clone(),
                generation: *generation,
            })
        });
        Ok((runtime, output))
    }

    pub(super) fn with_surface_capture_authority<T>(
        &self,
        identity: &nickel_remote_control::leases::ResourceId,
        permit: &DesktopPermit,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let (_, output) = self.surface_capture_evidence(identity)?;
        let ancestors = self.remote_surface_ancestors(identity);
        permit.with_resource(
            &nickel_remote_control::leases::ResourceEvidence {
                window: None,
                surface: Some(identity),
                output: output.as_ref(),
                verified_application: None,
                authorized_surface_ancestors: &ancestors,
                protected: false,
            },
            effect,
        )
    }

    fn with_capture_target_authority<T>(
        &self,
        target: &CaptureTarget,
        permit: &DesktopPermit,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        match target {
            CaptureTarget::Window(id) => self.with_capture_authority(*id, permit, effect),
            CaptureTarget::Surface(id) => self.with_surface_capture_authority(id, permit, effect),
            CaptureTarget::Output(id) => self.with_output_capture_authority(id, permit, effect),
        }
    }

    pub(super) fn list_remote_surfaces(
        &self,
        permit: &DesktopPermit,
    ) -> Result<Vec<nickel_remote_control::diagnostics::ShellSurfaceDiagnostic>, String> {
        permit.check_live()?;
        let (surfaces, _) = self.remote_shell_surface_diagnostics();
        let visible = surfaces
            .into_iter()
            .filter(|surface| {
                let identity = nickel_remote_control::leases::ResourceId {
                    id: surface.id.clone(),
                    generation: surface.generation,
                };
                self.with_surface_capture_authority(&identity, permit, || Ok(()))
                    .is_ok()
            })
            .collect();
        permit.check_live()?;
        Ok(visible)
    }

    pub(super) fn with_capture_authority<T>(
        &self,
        id: WindowId,
        permit: &DesktopPermit,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        if self.registry_native_window(id).is_none() {
            return Err("native window is unavailable".into());
        }
        let identity = nickel_remote_control::leases::ResourceId {
            id: id.0.to_string(),
            generation: id.0,
        };
        // This operation returns the entire client area. An output lease cannot
        // expose the portion of a straddling window on another display.
        let output = self.remote_window_output(id).filter(|output| {
            let Some(area) = self.output_geometry_named(&output.id) else {
                return false;
            };
            let Some(bounds) = self.remote_window_geometry(id) else {
                return false;
            };
            i64::from(bounds.x) >= i64::from(area.x)
                && i64::from(bounds.y) >= i64::from(area.y)
                && i64::from(bounds.x) + i64::from(bounds.width)
                    <= i64::from(area.x) + i64::from(area.width)
                && i64::from(bounds.y) + i64::from(bounds.height)
                    <= i64::from(area.y) + i64::from(area.height)
        });
        let application = self.remote_verified_application(id);
        permit.with_resource(
            &nickel_remote_control::leases::ResourceEvidence {
                window: Some(&identity),
                surface: None,
                output: output.as_ref(),
                verified_application: application.as_deref(),
                authorized_surface_ancestors: &[],
                protected: self.remote_window_is_protected(id),
            },
            effect,
        )
    }

    pub(super) fn enqueue_remote_capture(
        &mut self,
        permit: DesktopPermit,
        target: CaptureTarget,
        reply: CaptureReply,
    ) {
        self.revalidate_remote_capture();
        let result = (|| {
            self.with_capture_target_authority(&target, &permit, || Ok(()))?;
            if self.remote_capture_work.is_some() {
                return Err("compositor capture is busy".into());
            }
            Ok(target)
        })();
        match result {
            Err(error) => {
                let _ = reply.try_send(Err(error));
            }
            Ok(target) => {
                self.remote_capture_work = Some(RemoteCaptureWork {
                    target,
                    permit,
                    reply,
                    submission: None,
                    context: 0,
                    renderer_generation: (0, 0),
                    capture_generation: 0,
                    submitted_at_us: 0,
                });
                self.request_output_redraw();
                #[cfg(feature = "backend-udev")]
                self.schedule_native_preview_work();
            }
        }
    }

    pub(crate) fn revalidate_remote_capture(&mut self) {
        if self.remote_capture_work.as_ref().is_some_and(|work| {
            self.with_capture_target_authority(&work.target, &work.permit, || Ok(()))
                .is_err()
        }) {
            self.cancel_remote_capture("capture authority expired or changed");
        }
    }

    pub(crate) fn remote_capture_pending(&self) -> bool {
        self.remote_capture_work.is_some()
    }

    pub(crate) fn cancel_remote_capture(&mut self, reason: &str) {
        if let Some(work) = self.remote_capture_work.take() {
            let _ = work.reply.try_send(Err(reason.into()));
        }
    }

    pub(crate) fn poll_remote_capture(
        &mut self,
        renderer: &mut GlesRenderer,
        renderer_generation: (u64, u64),
    ) {
        let Some(mut work) = self.remote_capture_work.take() else {
            return;
        };
        let result = (|| {
            self.with_capture_target_authority(&work.target, &work.permit, || Ok(()))?;
            if !crate::session::window_capture::capture_supported(renderer) {
                return Err("capture fencing is unavailable".into());
            }
            let context = renderer.egl_context().get_context_handle() as usize;
            if let Some(submission) = &work.submission {
                if work.context != context || work.renderer_generation != renderer_generation {
                    return Err("capture renderer changed".into());
                }
                if !submission.fence.is_reached() {
                    return Ok(None);
                }
                let frame =
                    self.with_capture_target_authority(&work.target, &work.permit, || {
                        let (width, height) = submission.dimensions;
                        let mapped = renderer
                            .map_texture(&submission.mapping)
                            .map_err(|_| "capture mapping failed")?;
                        let row = usize::from(width) * 4;
                        if mapped.len() != row * usize::from(height) {
                            return Err("capture mapping dimensions changed".into());
                        }
                        // Transform::Normal offscreen rendering produces the same top-to-bottom
                        // mapped rows used by native previews. GlesMapping::flipped is relative
                        // to a lower-left origin; reversing these rows would invert the PNG.
                        let rgba = mapped.to_vec();
                        Ok(CapturedWindow {
                            window_id: work.target.identity().0,
                            generation: work.target.identity().1,
                            capture_generation: work.capture_generation,
                            submitted_at_us: work.submitted_at_us,
                            completed_at_us: self
                                .start_time
                                .elapsed()
                                .as_micros()
                                .min(u64::MAX as u128)
                                as u64,
                            width,
                            height,
                            rgba,
                        })
                    })?;
                Ok(Some(frame))
            } else {
                work.submission = Some(match &work.target {
                    CaptureTarget::Window(id) => {
                        let window = self
                            .registry_native_window(*id)
                            .ok_or("native window disappeared")?;
                        let size = window.geometry().size;
                        let dimensions = (
                            u16::try_from(size.w).map_err(|_| "capture width exceeds limit")?,
                            u16::try_from(size.h).map_err(|_| "capture height exceeds limit")?,
                        );
                        self.with_capture_authority(*id, &work.permit, || {
                            submit_window_capture(renderer, &window, dimensions).ok_or_else(|| {
                                "capture submission unavailable or dimensions exceed limit"
                                    .to_owned()
                            })
                        })?
                    }
                    CaptureTarget::Surface(identity) => {
                        let (runtime, output) = self.surface_capture_evidence(identity)?;
                        let (_, _, width, height) = self
                            .internal_ui
                            .placement(runtime)
                            .ok_or("surface disappeared")?
                            .geometry;
                        let scale = f64::from(
                            self.internal_ui
                                .scale_factor(runtime)
                                .ok_or("surface scale unavailable")?,
                        );
                        let width = (f64::from(width) * scale).ceil();
                        let height = (f64::from(height) * scale).ceil();
                        if !scale.is_finite()
                            || scale <= 0.0
                            || width > 8192.0
                            || height > 8192.0
                            || width * height > 16_777_216.0
                        {
                            return Err("capture dimensions exceed limit".into());
                        }
                        // The owner cannot mutate visibility/placement while this closure runs;
                        // the permit lock excludes concurrent revocation throughout submission.
                        let ancestors = self.remote_surface_ancestors(identity);
                        work.permit.with_resource(
                            &nickel_remote_control::leases::ResourceEvidence {
                                window: None,
                                surface: Some(identity),
                                output: output.as_ref(),
                                verified_application: None,
                                authorized_surface_ancestors: &ancestors,
                                protected: false,
                            },
                            || {
                                let elements = self.internal_ui.capture_elements(renderer, runtime);
                                submit_elements_capture(
                                    renderer,
                                    &elements,
                                    (width as u16, height as u16),
                                    scale,
                                )
                                .ok_or_else(|| "surface capture submission unavailable".to_owned())
                            },
                        )?
                    }
                    CaptureTarget::Output(identity) => {
                        let output = self.output_capture_evidence(identity)?;
                        let mode = output.current_mode().ok_or("output mode is unavailable")?;
                        let dimensions = (
                            u16::try_from(mode.size.w)
                                .map_err(|_| "output capture width exceeds limit")?,
                            u16::try_from(mode.size.h)
                                .map_err(|_| "output capture height exceeds limit")?,
                        );
                        let scale = output.current_scale().fractional_scale();
                        let output_geometry = self
                            .space
                            .output_geometry(&output)
                            .ok_or("output geometry is unavailable")?;
                        let mut elements = self
                            .internal_ui
                            .render_elements_without_trusted(
                                renderer,
                                &output.name(),
                                output_geometry.loc,
                                Some(crate::session::InternalSurfaceLayer::Overlay),
                            )
                            .into_iter()
                            .map(OutputCaptureRenderElement::from)
                            .collect::<Vec<_>>();
                        if self.internal_applications_are_foremost() {
                            elements.extend(
                                self.internal_ui
                                    .render_elements_for_layer(
                                        renderer,
                                        &output.name(),
                                        output_geometry.loc,
                                        Some(crate::session::InternalSurfaceLayer::Application),
                                    )
                                    .into_iter()
                                    .map(OutputCaptureRenderElement::from),
                            );
                        }
                        elements.extend(
                            space_render_elements::<GlesRenderer, Window, _>(
                                renderer,
                                [&self.space],
                                &output,
                                scale as f32,
                            )
                            .map_err(|_| "output scene capture is unavailable")?
                            .into_iter()
                            .map(OutputCaptureRenderElement::from),
                        );
                        if !self.internal_applications_are_foremost() {
                            elements.extend(
                                self.internal_ui
                                    .render_elements_for_layer(
                                        renderer,
                                        &output.name(),
                                        output_geometry.loc,
                                        Some(crate::session::InternalSurfaceLayer::Application),
                                    )
                                    .into_iter()
                                    .map(OutputCaptureRenderElement::from),
                            );
                        }
                        elements.extend(
                            self.internal_ui
                                .render_elements_for_layer(
                                    renderer,
                                    &output.name(),
                                    output_geometry.loc,
                                    Some(crate::session::InternalSurfaceLayer::Background),
                                )
                                .into_iter()
                                .map(OutputCaptureRenderElement::from),
                        );
                        // `render_elements_without_trusted` is the production compositor
                        // boundary that excludes TrustedControl and protected overlay pixels.
                        self.with_output_capture_authority(identity, &work.permit, || {
                            submit_elements_capture(renderer, &elements, dimensions, scale)
                                .ok_or_else(|| "output capture submission unavailable".to_owned())
                        })?
                    }
                });
                self.remote_observation_generation =
                    self.remote_observation_generation.saturating_add(1);
                work.capture_generation = self.remote_observation_generation;
                work.submitted_at_us =
                    self.start_time.elapsed().as_micros().min(u64::MAX as u128) as u64;
                work.context = context;
                work.renderer_generation = renderer_generation;
                Ok(None)
            }
        })();
        match result {
            Ok(Some(frame)) => {
                let _ = work.reply.try_send(Ok(frame));
            }
            Err(error) => {
                let _ = work.reply.try_send(Err(error));
            }
            Ok(None) => {
                self.remote_capture_work = Some(work);
                self.request_output_redraw();
            }
        }
    }
}
