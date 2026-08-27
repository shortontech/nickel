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
                Kind,
                memory::MemoryRenderBufferRenderElement,
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
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
    utils::{Buffer, Rectangle, Transform},
};

use nickel_core::{
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
};

use crate::{CalloopData, NickelSession, state::PreviewFrame};

smithay::backend::renderer::element::render_elements! {
    WinitFrameElement<R> where R: ImportAll + ImportMem;
    Memory=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

pub fn init_winit(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &mut data.display_handle;
    let state = &mut data.state;

    let (mut backend, winit) = winit::init()?;

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
        },
    );
    let _global = output.create_global::<NickelSession>(display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.space.map_output(&output, (0, 0));

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
            let display = &mut data.display_handle;
            let state = &mut data.state;

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
                }
                WinitEvent::Redraw => {
                    backend
                        .window()
                        .set_cursor(frame_cursor_icon(state.frame_cursor));
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
                        let frame_elements = frame_elements(
                            state,
                            renderer,
                            &output,
                            frame_icons.as_ref(),
                            &frame_palette,
                        );
                        if !frame_elements.is_empty() {
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements(&mut frame, 1.0, &frame_elements, &[damage])
                                .unwrap();
                            let _ = frame.finish().unwrap();
                        }

                        if let Some(highlight) = state.preview_highlight
                            && let Some(window) = state.space.elements().find(|window| {
                                window
                                    .toplevel()
                                    .and_then(|surface| {
                                        state.surface_windows.get(&surface.wl_surface().id())
                                    })
                                    .copied()
                                    == Some(highlight)
                            })
                        {
                            let dim_buffer =
                                SolidColorBuffer::new(size.to_logical(1), [0.0, 0.0, 0.0, 0.62]);
                            let dim = SolidColorRenderElement::from_buffer(
                                &dim_buffer,
                                (0, 0),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            );
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements::<GlesRenderer, _, _>(
                                &mut frame,
                                1.0,
                                &[dim],
                                &[damage],
                            )
                            .unwrap();
                            let _ = frame.finish().unwrap();

                            let location = state.space.element_location(window).unwrap_or_default();
                            let selected = render_elements_from_surface_tree::<
                                GlesRenderer,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                            >(
                                renderer,
                                window.toplevel().unwrap().wl_surface(),
                                location.to_physical(1),
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            );
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements(&mut frame, 1.0, &selected, &[damage]).unwrap();
                            let _ = frame.finish().unwrap();

                            let mut shell_elements = Vec::new();
                            for shell in state.shell_windows() {
                                let Some(location) = state.space.element_location(shell) else {
                                    continue;
                                };
                                let Some(surface) = shell.toplevel() else {
                                    continue;
                                };
                                shell_elements.extend(render_elements_from_surface_tree::<
                                    GlesRenderer,
                                    WaylandSurfaceRenderElement<GlesRenderer>,
                                >(
                                    renderer,
                                    surface.wl_surface(),
                                    location.to_physical(1),
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                ));
                            }
                            let mut frame = renderer
                                .render(&mut framebuffer, size, output.current_transform())
                                .unwrap();
                            draw_render_elements(&mut frame, 1.0, &shell_elements, &[damage])
                                .unwrap();
                            let _ = frame.finish().unwrap();
                        }
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    let capture_response = state
                        .output_capture_path
                        .take()
                        .map(|path| capture_output(&mut backend, size, &path));

                    if let Some(response) = capture_response
                        && let Some(reply_path) = state.output_capture_reply_path.take()
                        && let Ok(socket) = std::os::unix::net::UnixDatagram::unbound()
                    {
                        let _ = socket.send_to(response.as_bytes(), reply_path);
                    }

                    if last_preview_capture.elapsed() >= Duration::from_millis(200) {
                        let windows: Vec<_> = state
                            .space
                            .elements()
                            .filter(|window| !state.shell_windows().any(|shell| shell == *window))
                            .filter_map(|window| {
                                let id = state
                                    .surface_windows
                                    .get(&window.toplevel()?.wl_surface().id())?;
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

                    // Ask for redraw to schedule new frame.
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
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

fn frame_elements(
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
    let mut elements = Vec::new();
    for window in state.space.elements().rev() {
        let Some(bounds) = state.space.element_bbox(window) else {
            continue;
        };
        if !output_geometry.overlaps(bounds) {
            continue;
        }
        let Some(surface) = window.toplevel().map(|top| top.wl_surface()) else {
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
        let frame_index = elements.len();
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
            elements.push(WinitFrameElement::from(element));
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
                    elements.insert(frame_index, WinitFrameElement::from(icon));
                }
            }
        }
    }
    elements
}

fn capture_output(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
    size: smithay::utils::Size<i32, smithay::utils::Physical>,
    path: &Path,
) -> String {
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
        Ok(()) => "ok\tnested".to_owned(),
        Err(error) => format!("error\t{error}"),
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
    let elements = render_elements_from_surface_tree::<
        GlesRenderer,
        WaylandSurfaceRenderElement<GlesRenderer>,
    >(
        renderer,
        window.toplevel()?.wl_surface(),
        (0, 0),
        1.0,
        1.0,
        Kind::Unspecified,
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
