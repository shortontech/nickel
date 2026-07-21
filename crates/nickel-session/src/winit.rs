use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Frame, Offscreen, Renderer, TextureMapping,
            damage::OutputDamageTracker,
            element::{
                Kind,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
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

use crate::{CalloopData, NickelSession, state::PreviewFrame};

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
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let size = backend.window_size();
                    let damage = Rectangle::from_size(size);

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
                    }
                    backend.submit(Some(&[damage])).unwrap();

                    if last_preview_capture.elapsed() >= Duration::from_millis(200) {
                        let shell_windows = [
                            state.launcher_window.as_ref(),
                            state.panel_window.as_ref(),
                            state.context_menu_window.as_ref(),
                        ];
                        let windows: Vec<_> = state
                            .space
                            .elements()
                            .filter(|window| {
                                !shell_windows
                                    .into_iter()
                                    .flatten()
                                    .any(|shell| shell == *window)
                            })
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

fn capture_preview(renderer: &mut GlesRenderer, window: &Window) -> Option<PreviewFrame> {
    const WIDTH: i32 = 240;
    const HEIGHT: i32 = 135;
    let geometry = window.geometry();
    if geometry.size.w <= 0 || geometry.size.h <= 0 {
        return None;
    }
    let scale = (f64::from(WIDTH) / f64::from(geometry.size.w))
        .min(f64::from(HEIGHT) / f64::from(geometry.size.h));
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
        scale,
        1.0,
        Kind::Unspecified,
    );
    let damage = Rectangle::from_size((WIDTH, HEIGHT).into());
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
    let mut rgba = renderer.map_texture(&mapping).ok()?.to_vec();
    if mapping.flipped() {
        let stride = WIDTH as usize * 4;
        for row in 0..HEIGHT as usize / 2 {
            let opposite = HEIGHT as usize - 1 - row;
            for column in 0..stride {
                rgba.swap(row * stride + column, opposite * stride + column);
            }
        }
    }
    Some(PreviewFrame {
        width: WIDTH as u16,
        height: HEIGHT as u16,
        rgba,
    })
}
