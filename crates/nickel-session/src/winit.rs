use std::{
    path::Path,
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
                surface::WaylandSurfaceRenderElement,
                utils::{ConstrainAlign, ConstrainScaleBehavior, constrain_render_elements},
            },
            gles::{GlesRenderer, GlesTexture},
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

smithay::backend::renderer::element::render_elements! {
    WinitFrameElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Memory=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

struct WindowRenderGroup<E> {
    client: Vec<E>,
    frame: Vec<E>,
}

fn flatten_window_groups<E>(groups: Vec<WindowRenderGroup<E>>) -> Vec<E> {
    groups
        .into_iter()
        .flat_map(|group| group.client.into_iter().chain(group.frame))
        .collect()
}

pub fn init_winit(
    event_loop: &mut EventLoop<NickelSession>,
    data: &mut NickelSession,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = data.display_handle.clone();
    let state = data;

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
    let frame_icons = crate::window_frame::FrameIcons::load();

    // SAFETY: startup is single-threaded and no child process is spawned until
    // after this function returns.
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &state.socket_name) };

    event_loop
        .handle()
        .insert_source(winit, move |event, _, data| {
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

                    {
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
                        let window_elements = composited_window_elements(
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

                        let recovery_banner = state.shell_recovery_visible().then(|| {
                            let banner_width = size.w.clamp(1, 560);
                            let banner_height = size.h.clamp(1, 112);
                            let banner_buffer = SolidColorBuffer::new(
                                (banner_width, banner_height),
                                [0.45, 0.06, 0.08, 1.0],
                            );
                            SolidColorRenderElement::from_buffer(
                                &banner_buffer,
                                ((size.w - banner_width) / 2, (size.h - banner_height) / 2),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            )
                        });

                        if !overlay_elements.is_empty() || recovery_banner.is_some() {
                            if let Some(banner) = recovery_banner {
                                overlay_elements.insert(0, WinitFrameElement::from(banner));
                            }
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements(&mut frame, 1.0, &overlay_elements, &[damage])
                                .unwrap();
                            let _ = frame.finish().unwrap();
                        }
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    if let Some(started) = state.launcher_show_requested_at.take()
                        && std::env::var_os("NICKEL_PERF_METRICS").is_some()
                    {
                        eprintln!(
                            "launcher_open_to_visible_ms={:.3}",
                            started.elapsed().as_secs_f64() * 1_000.0
                        );
                    }

                    let capture_response = state.output_capture_path.take().map(|path| {
                        let result = capture_output(&mut backend, size, &path);
                        (path, result)
                    });

                    if let Some((path, result)) = capture_response {
                        state.complete_output_capture(&path, result);
                    }

                    if !state.locked && last_preview_capture.elapsed() >= Duration::from_millis(200)
                    {
                        let windows: Vec<_> = state
                            .space
                            .elements()
                            .filter(|window| !state.shell_windows().any(|shell| shell == *window))
                            .filter_map(|window| {
                                let id = state.surface_windows.get(&window.wl_surface()?.id())?;
                                state
                                    .preview_requests
                                    .contains(id)
                                    .then(|| (*id, window.clone()))
                            })
                            .collect();
                        let (renderer, _) = backend.bind().unwrap();
                        for (id, window) in windows {
                            if let Some(frame) = capture_preview(renderer, &window) {
                                state.preview_frames.insert(id, frame);
                                state.notify_preview_frame(id);
                            }
                        }
                        last_preview_capture = Instant::now();
                    }

                    state.space.elements().for_each(|window| {
                        window.send_frame(
                            &output,
                            state.start_time.elapsed(),
                            Some(Duration::ZERO),
                            |_, _| Some(output.clone()),
                        )
                    });

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

fn composited_window_elements(
    state: &NickelSession,
    renderer: &mut GlesRenderer,
    output: &Output,
    icons: Option<&crate::window_frame::FrameIcons>,
    palette: &ThemePalette,
) -> Vec<WinitFrameElement<GlesRenderer>> {
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
        let client = window
            .render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                render_location.to_physical_precise_round(1),
                Scale::from(1.0),
                1.0,
            )
            .into_iter()
            .map(WinitFrameElement::from)
            .collect::<Vec<_>>();
        if client.is_empty() {
            continue;
        }
        let mut frame = Vec::new();
        let Some(surface) = window.wl_surface() else {
            groups.push(WindowRenderGroup { client, frame });
            continue;
        };
        if shell_surfaces.contains(&surface.id())
            || state.is_fullscreen_window(window)
            || !state.is_server_decorated(window)
        {
            groups.push(WindowRenderGroup { client, frame });
            continue;
        }
        let registry_id = state.surface_windows.get(&surface.id()).copied();
        let active = registry_id.is_some_and(|id| state.windows.is_active(id));
        let title = registry_id
            .and_then(|id| state.windows.title(id))
            .unwrap_or_default();
        let foreground = if active { palette.text } else { palette.muted };
        if let Some(titlebar) =
            crate::window_frame::render_titlebar(bounds.size.w, title, palette.panel, foreground)
            && let Ok(element) = MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (
                    f64::from(bounds.loc.x - output_geometry.loc.x),
                    f64::from(
                        bounds.loc.y - output_geometry.loc.y - crate::window_frame::TITLEBAR_HEIGHT,
                    ),
                ),
                &titlebar,
                None,
                None,
                Some((bounds.size.w, crate::window_frame::TITLEBAR_HEIGHT).into()),
                Kind::Unspecified,
            )
        {
            frame.push(WinitFrameElement::from(element));
        }
        if let Some(icons) = icons {
            let icon_y =
                bounds.loc.y - output_geometry.loc.y - crate::window_frame::TITLEBAR_HEIGHT + 8;
            let icon_x = bounds.loc.x - output_geometry.loc.x + bounds.size.w;
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
        groups.push(WindowRenderGroup { client, frame });
    }
    flatten_window_groups(groups)
}

fn capture_output(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    path: &Path,
) -> nickel_session_protocol::CaptureResult {
    let result = (|| -> Result<(), String> {
        let (renderer, framebuffer) = backend.bind().map_err(|error| error.to_string())?;
        let buffer_size = smithay::utils::Size::<i32, Buffer>::from((size.w, size.h));
        let region = Rectangle::<i32, Buffer>::from_size(buffer_size);
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
            .map_err(|error| error.to_string())?;
        let mapped = renderer
            .map_texture(&mapping)
            .map_err(|error| error.to_string())?;
        // The nested output uses `Flipped180` for presentation, so restore its
        // mapped framebuffer rows to top-down image order unconditionally.
        let rgba = normalize_capture_rows(mapped, size.w as usize, size.h as usize, false)?;
        image::save_buffer(
            path,
            &rgba,
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

fn capture_preview(renderer: &mut GlesRenderer, window: &Window) -> Option<PreviewFrame> {
    const WIDTH: i32 = 240;
    const HEIGHT: i32 = 135;
    let geometry = window.geometry();
    if geometry.size.w <= 0 || geometry.size.h <= 0 {
        return None;
    }
    let mut texture =
        Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, (WIDTH, HEIGHT).into())
            .ok()?;
    let mut framebuffer = renderer.bind(&mut texture).ok()?;
    let elements = window.render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
        renderer,
        (-geometry.loc.x, -geometry.loc.y).into(),
        Scale::from(1.0),
        1.0,
    );
    let damage = Rectangle::from_size((WIDTH, HEIGHT).into());
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
    let mut frame = renderer
        .render(&mut framebuffer, (WIDTH, HEIGHT).into(), Transform::Normal)
        .ok()?;
    frame
        .clear(Color32F::new(0.03, 0.04, 0.06, 1.0), &[damage])
        .ok()?;
    draw_render_elements(&mut frame, 1.0, &elements, &[damage]).ok()?;
    frame.finish().ok()?.wait().ok()?;
    let buffer_region = Rectangle::<i32, Buffer>::from_size((WIDTH, HEIGHT).into());
    let mapping = renderer
        .copy_framebuffer(&framebuffer, buffer_region, Fourcc::Abgr8888)
        .ok()?;
    let rgba = renderer.map_texture(&mapping).ok()?.to_vec();
    Some(PreviewFrame {
        width: WIDTH as u16,
        height: HEIGHT as u16,
        rgba,
    })
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
    use super::{WindowRenderGroup, flatten_window_groups};

    #[test]
    fn window_frames_stay_with_their_clients_in_stacking_order() {
        let elements = flatten_window_groups(vec![
            WindowRenderGroup {
                client: vec!["foreground-client"],
                frame: vec!["foreground-frame"],
            },
            WindowRenderGroup {
                client: vec!["background-client"],
                frame: vec!["background-frame"],
            },
        ]);

        assert_eq!(
            elements,
            [
                "foreground-client",
                "foreground-frame",
                "background-client",
                "background-frame",
            ]
        );
    }
}
