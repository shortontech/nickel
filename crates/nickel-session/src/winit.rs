use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Frame, ImportAll, ImportMem, Offscreen, Renderer,
            damage::OutputDamageTracker,
            element::{
                AsRenderElements, Kind,
                memory::MemoryRenderBufferRenderElement,
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
                utils::{ConstrainAlign, ConstrainScaleBehavior, constrain_render_elements},
            },
            gles::{GlesRenderer, GlesTarget, GlesTexture},
            utils::draw_render_elements,
        },
        winit::{self, WinitEvent},
    },
    desktop::Window,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{calloop::EventLoop, wayland_server::Resource},
    utils::{Buffer, Rectangle, Scale, Transform},
    wayland::seat::WaylandFocus,
};

use nickel_core::{
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
};

use crate::{NickelSession, state::PreviewFrame};

const PREVIEW_CAPTURE_INTERVAL: Duration = Duration::from_millis(200);

fn preview_retry_delay(last_capture: Instant, now: Instant) -> Duration {
    PREVIEW_CAPTURE_INTERVAL.saturating_sub(now.saturating_duration_since(last_capture))
}

smithay::backend::renderer::element::render_elements! {
    WinitFrameElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Memory=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

fn advance_output_capture(
    pending: &mut Option<PathBuf>,
    requested: Option<PathBuf>,
) -> (Option<PathBuf>, bool) {
    let ready = pending.take();
    let request_another_frame = requested.is_some();
    *pending = requested;
    (ready, request_another_frame)
}

fn flatten_frame_groups<E>(groups: Vec<Vec<E>>) -> Vec<E> {
    groups.into_iter().flatten().collect()
}

pub fn init_winit(
    event_loop: &mut EventLoop<NickelSession>,
    data: &mut NickelSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = data.display_handle.clone();
    let state = data;

    let renderer_owner = nickel_core::resource_owner::try_acquire_smithay_renderer_owner()?;
    let (mut backend, winit) = winit::init()?;
    state.set_winit_redraw_window(backend.window());
    let startup_frame_pump_until = Instant::now() + Duration::from_secs(3);

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Smithay".into(),
            model: "Winit".into(),
            serial_number: String::new(),
        },
    );
    let _global = output.create_global::<NickelSession>(&display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));
    state.primary_output_name = Some(output.name());

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut last_preview_capture = Instant::now() - Duration::from_secs(1);
    let mut last_preview_highlight = None;
    let mut pending_output_capture = None;
    let frame_icons = crate::window_frame::FrameIcons::load();

    // SAFETY: startup is single-threaded and no child process is spawned until
    // after this function returns.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    event_loop
        .handle()
        .insert_source(winit, move |event, _, data| {
            // Keep lifecycle accounting adjacent to the backend captured by
            // this source; both are released when the event source drops.
            let _ = &renderer_owner;
            let mut display = data.display_handle.clone();
            let state = data;

            match event {
                WinitEvent::Resized { size, .. } => {
                    let mode = Mode {
                        size,
                        refresh: 60_000,
                    };
                    output.set_preferred(mode);
                    output.change_current_state(Some(mode), None, None, None);
                    damage_tracker = OutputDamageTracker::from_output(&output);
                    state.space.refresh();
                    state.relayout_shell_surfaces();
                    let _ = display.flush_clients();
                    backend.window().request_redraw();
                    eprintln!("nickel-session: output resized to {}x{}", size.w, size.h);
                }
                WinitEvent::Input(event) => {
                    let _ = state.process_input_event(event);
                    let _ = display.flush_clients();
                    backend.window().request_redraw();
                }
                WinitEvent::Focus(false) => {
                    state.release_pressed_keys_on_host_focus_loss();
                }
                WinitEvent::Focus(true) => {}
                WinitEvent::Redraw => {
                    backend
                        .window()
                        .set_cursor(smithay::reexports::winit::cursor::Cursor::Icon(
                            frame_cursor_icon(state.frame_cursor),
                        ));
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);
                    if state.preview_highlight.is_some()
                        || state.preview_highlight != last_preview_highlight
                    {
                        damage_tracker = OutputDamageTracker::from_output(&output);
                    }
                    last_preview_highlight = state.preview_highlight;
                    let image_copy_requested = state.has_pending_image_copy_frames(&output);
                    let requested_output_capture = state.output_capture_path.take();
                    if requested_output_capture.is_some() {
                        state.output_capture_name = None;
                    }
                    let (output_capture_path, request_another_frame) =
                        advance_output_capture(
                            &mut pending_output_capture,
                            requested_output_capture,
                        );
                    if request_another_frame {
                        // Capture the following fully rendered frame. This prevents a surface
                        // commit queued beside the capture request (notably an overlay unmap)
                        // from exposing a compositor transition buffer to the screenshot.
                        backend.window().request_redraw();
                    }
                    let capture_requested = image_copy_requested || output_capture_path.is_some();
                    if capture_requested {
                        // Buffer-age preservation is sufficient for presentation, but capture
                        // reads the current backbuffer before it becomes the frontbuffer. Force a
                        // complete repaint so undefined regions from a newly selected backbuffer
                        // can never leak into screenshots or portal frames.
                        damage_tracker = OutputDamageTracker::from_output(&output);
                    }

                    let captured_frame = {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        smithay::desktop::space::render_output::<
                            _,
                            WaylandSurfaceRenderElement<GlesRenderer>,
                            _,
                            _,
                        >(
                            &output,
                            renderer,
                            &mut framebuffer,
                            1.0,
                            0,
                            [&state.space],
                            &[],
                            &mut damage_tracker,
                            [0.1, 0.1, 0.1, 1.0],
                        )
                        .unwrap();

                        let frame_palette = ThemePalette::from_appearance(
                            ShellSettings::load_default().resolve_appearance(Appearance::default()),
                        );
                        let window_elements = window_frame_elements(
                            state,
                            renderer,
                            &output,
                            frame_icons.as_ref(),
                            &frame_palette,
                        );
                        let mut overlay_elements = Vec::new();
                        if !state.locked
                            && let Some(window) = state.preview_highlight.and_then(|highlight| {
                                state.space.elements().find(|window| {
                                    window
                                        .wl_surface()
                                        .and_then(|surface| {
                                            state.surface_windows.get(&surface.id())
                                        })
                                        .copied()
                                        == Some(highlight)
                                })
                            })
                        {
                            let shell_surfaces = state
                                .shell_windows()
                                .filter_map(|shell| {
                                    shell.toplevel().map(|surface| surface.wl_surface().id())
                                })
                                .collect::<Vec<_>>();
                            let desktop_surfaces = state
                                .desktop_windows
                                .iter()
                                .filter_map(|desktop| {
                                    desktop.toplevel().map(|surface| surface.wl_surface().id())
                                })
                                .collect::<Vec<_>>();
                            for shell in state.space.elements().rev().filter(|shell| {
                                shell.toplevel().is_some_and(|surface| {
                                    shell_surfaces.contains(&surface.wl_surface().id())
                                        && !desktop_surfaces.contains(&surface.wl_surface().id())
                                })
                            }) {
                                let Some(location) = state.space.element_location(shell) else {
                                    continue;
                                };
                                let render_location = location - shell.geometry().loc;
                                overlay_elements.extend(
                                    shell
                                        .render_elements::<
                                            WaylandSurfaceRenderElement<GlesRenderer>,
                                        >(
                                            renderer,
                                            render_location.to_physical_precise_round(1),
                                            Scale::from(1.0),
                                            1.0,
                                        )
                                    .into_iter()
                                    .map(WinitFrameElement::from),
                                );
                            }
                            let location = state.space.element_location(window).unwrap_or_default();
                            let render_location = location - window.geometry().loc;
                            overlay_elements.extend(
                                window
                                    .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                                        renderer,
                                        render_location.to_physical_precise_round(1),
                                        Scale::from(1.0),
                                        1.0,
                                    )
                                    .into_iter()
                                    .map(WinitFrameElement::from),
                            );
                            let dim_buffer =
                                SolidColorBuffer::new(size.to_logical(1), [0.0, 0.0, 0.0, 0.62]);
                            overlay_elements.push(WinitFrameElement::from(
                                SolidColorRenderElement::from_buffer(
                                    &dim_buffer,
                                    (0, 0),
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                ),
                            ));
                        }
                        overlay_elements.extend(window_elements);
                        if state.dimmed && !state.locked {
                            let dim =
                                SolidColorBuffer::new(size.to_logical(1), [0.0, 0.0, 0.0, 0.48]);
                            overlay_elements.insert(
                                0,
                                WinitFrameElement::from(SolidColorRenderElement::from_buffer(
                                    &dim,
                                    (0, 0),
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                )),
                            );
                        }
                        if state.locked {
                            let cover = SolidColorBuffer::new(
                                size.to_logical(1),
                                [0.015, 0.02, 0.035, 1.0],
                            );
                            // Render elements are front-to-back. The lock
                            // surface stays before this opaque cover; every
                            // ordinary client remains behind it.
                            overlay_elements.push(WinitFrameElement::from(
                                SolidColorRenderElement::from_buffer(
                                    &cover,
                                    (0, 0),
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                ),
                            ));
                        }

                        if !state.locked
                            && let Some(icon) = state.dnd_icon.as_ref()
                        {
                            let pointer = state.seat.get_pointer().unwrap().current_location();
                            let location = crate::state::drag_icon_location(
                                pointer,
                                Rectangle::from_size(size.to_logical(1)),
                            )
                            .expect("nested pointer remains inside its only output")
                            .to_physical(1);
                            let icon_elements = render_elements_from_surface_tree::<
                                _,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                icon,
                                location,
                                Scale::from(1.0),
                                1.0,
                                Kind::Cursor,
                            )
                            .into_iter()
                            .map(WinitFrameElement::from);
                            overlay_elements.splice(0..0, icon_elements);
                        }

                        let recovery_visible = state.shell_recovery_visible();
                        if !recovery_visible {
                            state.recovery_ui.release_raster();
                        }
                        let recovery_banner = recovery_visible.then(|| {
                                let panel = state.recovery_ui.render_buffer();
                                let panel_geometry = crate::recovery_ui::RecoveryUi::panel_geometry(
                                    crate::shell_layout::Geometry {
                                        x: 0,
                                        y: 0,
                                        width: size.w,
                                        height: size.h,
                                    },
                                );
                                MemoryRenderBufferRenderElement::from_buffer(
                                    renderer,
                                    (f64::from(panel_geometry.x), f64::from(panel_geometry.y)),
                                    &panel,
                                    None,
                                    None,
                                    Some((panel_geometry.width, panel_geometry.height).into()),
                                    Kind::Unspecified,
                                )
                                .map_err(|error| {
                                    tracing::error!(?error, "failed to import recovery panel")
                                })
                                .ok()
                                .map(WinitFrameElement::from)
                            }).flatten();

                        if !overlay_elements.is_empty() || recovery_banner.is_some() {
                            if let Some(banner) = recovery_banner {
                                overlay_elements.insert(0, banner);
                            }
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements(&mut frame, 1.0, &overlay_elements, &[damage])
                                .unwrap();
                            let sync = frame.finish().unwrap();
                            if capture_requested {
                                sync.wait().unwrap();
                            }
                        }
                        capture_requested.then(|| {
                            let space_elements =
                                smithay::desktop::space::space_render_elements(
                                    renderer,
                                    [&state.space],
                                    &output,
                                    1.0,
                                )
                                .map_err(|error| error.to_string())?;
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .map_err(|error| error.to_string())?;
                            frame
                                .clear([0.1, 0.1, 0.1, 1.0].into(), &[damage])
                                .map_err(|error| error.to_string())?;
                            draw_render_elements(&mut frame, 1.0, &space_elements, &[damage])
                                .map_err(|error| error.to_string())?;
                            draw_render_elements(&mut frame, 1.0, &overlay_elements, &[damage])
                                .map_err(|error| error.to_string())?;
                            frame
                                .finish()
                                .map_err(|error| error.to_string())?
                                .wait()
                                .map_err(|error| error.to_string())?;
                            let captured = capture_bound_framebuffer(
                                renderer,
                                &framebuffer,
                                size,
                                output_capture_path.is_some(),
                                |mapped, flipped| {
                                    if image_copy_requested {
                                        state.complete_image_copy_frames(
                                            &output,
                                            mapped,
                                            size.w as usize,
                                            size.h as usize,
                                            flipped,
                                        );
                                    }
                                },
                            );
                            // Mapping a GLES framebuffer changes the active EGL target. Restore
                            // the presentation target before the winit backend swaps it.
                            let rebind = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .and_then(|frame| frame.finish());
                            if let Err(error) = rebind {
                                return Err(error.to_string());
                            }
                            captured
                        })
                    };
                    backend.submit(Some(&[damage])).unwrap();

                    if image_copy_requested
                        && captured_frame
                            .as_ref()
                            .expect("requested capture has a framebuffer result")
                            .is_err()
                    {
                        match captured_frame
                            .as_ref()
                            .expect("requested capture has a framebuffer result")
                        {
                            Ok(_) => unreachable!("successful portal delivery completed while mapped"),
                            Err(error) => {
                                tracing::warn!(%error, "failed to capture nested portal frame");
                                state.fail_image_copy_frames(
                                    &output,
                                    smithay::wayland::image_copy_capture::CaptureFailureReason::Unknown,
                                );
                            }
                        }
                    }

                    if let Some(started) = state.launcher_show_requested_at.take()
                        && std::env::var_os("NICKEL_PERF_METRICS").is_some()
                    {
                        eprintln!(
                            "launcher_open_to_visible_ms={:.3}",
                            started.elapsed().as_secs_f64() * 1_000.0
                        );
                    }

                    if let Some(path) = output_capture_path {
                        let captured_frame = captured_frame
                            .as_ref()
                            .expect("requested capture has a framebuffer result");
                        let result = save_output_capture(
                            captured_frame
                                .as_ref()
                                .map(|rgba| rgba.as_deref().expect("file capture retains encoder input"))
                                .map_err(String::as_str),
                            size,
                            &path,
                        );
                        state.complete_output_capture(&path, result);
                    }

                    if !state.locked && last_preview_capture.elapsed() >= PREVIEW_CAPTURE_INTERVAL
                    {
                        let wave = state.begin_preview_render_wave();
                        let windows = state.preview_capture_candidates(wave);
                        let (renderer, _) = backend.bind().unwrap();
                        for (id, window) in windows {
                            let (rgba, previous_dimensions) = state.take_preview_capture_buffer(id);
                            let mut rgba = rgba;
                            if let Some((width, height)) =
                                capture_preview(renderer, &window, &mut rgba)
                            {
                                state.store_preview(
                                    id,
                                    PreviewFrame {
                                        width,
                                        height,
                                        rgba,
                                    },
                                );
                            } else {
                                state.preview_capture_failed(id, rgba, previous_dimensions);
                            }
                        }
                        last_preview_capture = Instant::now();
                        state.schedule_preview_retry_after(preview_retry_delay(
                            last_preview_capture,
                            Instant::now(),
                        ));
                    }

                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });
                    if let Some(icon) = state.dnd_icon.as_ref() {
                        smithay::desktop::utils::send_frames_surface_tree(
                            icon,
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        );
                    }

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = display.flush_clients();

                    // Keep a bounded bootstrap pump so the initial XDG
                    // configure is flushed before clients can submit buffers.
                    // After startup, commits and explicit state changes request
                    // frames and an unchanged desktop stays idle.
                    if Instant::now() < startup_frame_pump_until {
                        backend.window().request_redraw();
                    }
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
            };
        })?;

    Ok(())
}

fn frame_cursor_icon(cursor: crate::window_frame::FrameCursor) -> ::winit::window::CursorIcon {
    use crate::window_frame::FrameCursor;
    use ::winit::window::CursorIcon;

    match cursor {
        FrameCursor::Arrow => CursorIcon::Default,
        FrameCursor::North => CursorIcon::NResize,
        FrameCursor::NorthEast => CursorIcon::NeResize,
        FrameCursor::East => CursorIcon::EResize,
        FrameCursor::SouthEast => CursorIcon::SeResize,
        FrameCursor::South => CursorIcon::SResize,
        FrameCursor::SouthWest => CursorIcon::SwResize,
        FrameCursor::West => CursorIcon::WResize,
        FrameCursor::NorthWest => CursorIcon::NwResize,
    }
}

fn window_frame_elements(
    state: &NickelSession,
    renderer: &mut GlesRenderer,
    output: &Output,
    icons: Option<&crate::window_frame::FrameIcons>,
    palette: &ThemePalette,
) -> Vec<WinitFrameElement<GlesRenderer>> {
    crate::window_frame::retain_titlebars_for_windows(
        state.surface_windows.values().map(|id| id.0),
    );
    let Some(output_geometry) = state.space.output_geometry(output) else {
        return Vec::new();
    };
    let shell_surfaces = state
        .shell_windows()
        .filter_map(|window| window.toplevel().map(|surface| surface.wl_surface().id()))
        .collect::<Vec<_>>();
    let mut groups = Vec::new();
    for window in state.space.elements().rev() {
        if state.locked && !state.lock_windows.contains(window) {
            continue;
        }
        let Some(bounds) = state.space.element_bbox(window) else {
            continue;
        };
        if !output_geometry.overlaps(bounds) {
            continue;
        }
        let Some(location) = state.space.element_location(window) else {
            continue;
        };
        let render_location = location - window.geometry().loc - output_geometry.loc;
        let has_client = !window
            .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                render_location.to_physical_precise_round(1),
                Scale::from(1.0),
                1.0,
            )
            .is_empty();
        if !has_client {
            continue;
        }
        let mut frame = Vec::new();
        let Some(surface) = window.wl_surface() else {
            continue;
        };
        if shell_surfaces.contains(&surface.id())
            || state.is_fullscreen_window(window)
            || !state.is_server_decorated(window)
        {
            continue;
        }
        let registry_id = state.surface_windows.get(&surface.id()).copied();
        let active = registry_id.is_some_and(|id| state.windows.is_active(id));
        let title = registry_id
            .and_then(|id| state.windows.title(id))
            .unwrap_or_default();
        let foreground = if active { palette.text } else { palette.muted };
        // Popups expand element_bbox, but they do not resize their owner's frame.
        let Some(frame_bounds) = state.space.element_geometry(window) else {
            continue;
        };
        let titlebar_geometry =
            crate::window_frame::titlebar_geometry(crate::shell_layout::Geometry {
                x: frame_bounds.loc.x,
                y: frame_bounds.loc.y,
                width: frame_bounds.size.w,
                height: frame_bounds.size.h,
            });
        if let Some(titlebar) = crate::window_frame::render_titlebar_for(
            registry_id.map(|id| id.0),
            titlebar_geometry.width,
            title,
            palette.panel,
            foreground,
        ) && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (
                f64::from(titlebar_geometry.x - output_geometry.loc.x),
                f64::from(titlebar_geometry.y - output_geometry.loc.y),
            ),
            &titlebar,
            None,
            None,
            Some((titlebar_geometry.width, titlebar_geometry.height).into()),
            Kind::Unspecified,
        ) {
            frame.push(WinitFrameElement::from(element));
        }
        if let Some(icons) = icons {
            let icon_y =
                frame_bounds.loc.y - output_geometry.loc.y - crate::window_frame::TITLEBAR_HEIGHT
                    + 8;
            let icon_x = frame_bounds.loc.x - output_geometry.loc.x + frame_bounds.size.w;
            let maximized = state.is_maximized_window(window);
            for (buffer, offset) in [
                (&icons.close, 35),
                (
                    if maximized {
                        &icons.restore
                    } else {
                        &icons.maximize
                    },
                    81,
                ),
                (&icons.minimize, 127),
            ] {
                if let Ok(icon) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    ((icon_x - offset) as f64, icon_y as f64),
                    buffer,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    frame.insert(0, WinitFrameElement::from(icon));
                }
            }
        }
        groups.push(frame);
    }
    flatten_frame_groups(groups)
}

fn save_output_capture(
    rgba: Result<&[u8], &str>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    path: &Path,
) -> nickel_session_protocol::CaptureResult {
    let result = (|| -> Result<(), String> {
        let rgba = rgba.map_err(str::to_owned)?;
        image::save_buffer(
            path,
            rgba,
            size.w as u32,
            size.h as u32,
            image::ColorType::Rgba8,
        )
        .map_err(|error| error.to_string())
    })();
    match result {
        Ok(()) => nickel_session_protocol::CaptureResult::Saved {
            backend: nickel_session_protocol::CaptureBackend::Nested,
        },
        Err(message) => nickel_session_protocol::CaptureResult::Failed { message },
    }
}

fn capture_bound_framebuffer(
    renderer: &mut GlesRenderer,
    framebuffer: &GlesTarget<'_>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    retain_normalized: bool,
    mut consume_mapped: impl FnMut(&[u8], bool),
) -> Result<Option<Vec<u8>>, String> {
    if size.w <= 0 || size.h <= 0 {
        return Err("output has no drawable size".into());
    }
    let buffer_size = smithay::utils::Size::<i32, Buffer>::from((size.w, size.h));
    let region = Rectangle::<i32, Buffer>::from_size(buffer_size);
    let mapping = renderer
        .copy_framebuffer(framebuffer, region, Fourcc::Abgr8888)
        .map_err(|error| error.to_string())?;
    let mapped = renderer
        .map_texture(&mapping)
        .map_err(|error| error.to_string())?;
    // The nested output uses `Flipped180` for presentation, so restore its
    // mapped framebuffer rows to top-down image order unconditionally.
    consume_mapped(mapped, false);
    if retain_normalized {
        normalize_capture_rows(mapped, size.w as usize, size.h as usize, false).map(Some)
    } else {
        Ok(None)
    }
}

fn capture_preview(
    renderer: &mut GlesRenderer,
    window: &Window,
    rgba: &mut Vec<u8>,
) -> Option<(u16, u16)> {
    (|| {
        let geometry = window.geometry();
        let dimensions =
            crate::state::preview_capture_dimensions(geometry.size.w, geometry.size.h)?;
        let width = i32::from(dimensions.0);
        let height = i32::from(dimensions.1);
        let mut texture = Offscreen::<GlesTexture>::create_buffer(
            renderer,
            Fourcc::Abgr8888,
            (width, height).into(),
        )
        .ok()?;
        let mut framebuffer = renderer.bind(&mut texture).ok()?;
        let elements = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            (-geometry.loc.x, -geometry.loc.y).into(),
            Scale::from(1.0),
            1.0,
        );
        let damage = Rectangle::from_size((width, height).into());
        let reference = Rectangle::from_size(geometry.size.to_physical(1));
        let elements = constrain_render_elements(
            elements,
            (0, 0),
            damage,
            reference,
            ConstrainScaleBehavior::Fit,
            ConstrainAlign::TOP
                | ConstrainAlign::BOTTOM
                | ConstrainAlign::LEFT
                | ConstrainAlign::RIGHT,
            1.0,
        )
        .collect::<Vec<_>>();
        let mut frame = renderer
            .render(&mut framebuffer, (width, height).into(), Transform::Normal)
            .ok()?;
        frame
            .clear(Color32F::new(0.03, 0.04, 0.06, 1.0), &[damage])
            .ok()?;
        draw_render_elements(&mut frame, 1.0, &elements, &[damage]).ok()?;
        frame.finish().ok()?.wait().ok()?;
        let buffer_region = Rectangle::<i32, Buffer>::from_size((width, height).into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, buffer_region, Fourcc::Abgr8888)
            .ok()?;
        let mapped = renderer.map_texture(&mapping).ok()?;
        if !crate::state::preview_mapping_has_exact_size(mapped, dimensions.0, dimensions.1) {
            return None;
        }
        let replacement = crate::state::reuse_preview_pixels(std::mem::take(rgba), mapped);
        *rgba = replacement;
        Some(dimensions)
    })()
}

fn normalize_capture_rows(
    mapped: &[u8],
    width: usize,
    height: usize,
    flipped: bool,
) -> Result<Vec<u8>, String> {
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| "capture row size overflowed".to_owned())?;
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "capture buffer size overflowed".to_owned())?;
    if mapped.len() < expected {
        return Err(format!(
            "renderer returned {} bytes for a {} byte output",
            mapped.len(),
            expected
        ));
    }
    let mut rgba = vec![0; expected];
    for destination_y in 0..height {
        let source_y = if flipped {
            destination_y
        } else {
            height - 1 - destination_y
        };
        rgba[destination_y * row_bytes..(destination_y + 1) * row_bytes]
            .copy_from_slice(&mapped[source_y * row_bytes..(source_y + 1) * row_bytes]);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    use super::{
        PREVIEW_CAPTURE_INTERVAL, advance_output_capture, flatten_frame_groups, preview_retry_delay,
    };

    #[test]
    fn window_frames_preserve_front_to_back_stacking_order() {
        let elements = flatten_frame_groups(vec![
            vec!["foreground-titlebar", "foreground-icons"],
            vec!["background-titlebar", "background-icons"],
        ]);

        assert_eq!(
            elements,
            [
                "foreground-titlebar",
                "foreground-icons",
                "background-titlebar",
                "background-icons",
            ]
        );
    }

    #[test]
    fn output_capture_waits_for_the_frame_after_its_request() {
        let mut pending = None;
        let request = PathBuf::from("capture.png");
        assert_eq!(
            advance_output_capture(&mut pending, Some(request.clone())),
            (None, true)
        );
        assert_eq!(pending, Some(request.clone()));
        assert_eq!(
            advance_output_capture(&mut pending, None),
            (Some(request), false)
        );
        assert_eq!(pending, None);
    }

    #[test]
    fn nested_preview_retry_deadline_matches_capture_eligibility() {
        let last_capture = Instant::now();
        assert_eq!(
            preview_retry_delay(last_capture, last_capture + Duration::from_millis(16)),
            PREVIEW_CAPTURE_INTERVAL - Duration::from_millis(16)
        );
        assert_eq!(
            preview_retry_delay(last_capture, last_capture + PREVIEW_CAPTURE_INTERVAL),
            Duration::ZERO
        );
    }
}
