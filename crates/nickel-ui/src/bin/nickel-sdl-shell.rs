use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[allow(dead_code)]
mod desktop {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum WallpaperPosition {
        Center,
        Tile,
        Stretch,
        Fit,
        Span,
        #[default]
        Fill,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Wallpaper {
        pub image: Option<image::RgbaImage>,
        pub color: [u8; 3],
        pub position: WallpaperPosition,
    }
}
#[path = "../icons.rs"]
#[allow(clippy::needless_borrow, dead_code)]
mod icons;
#[path = "../launcher.rs"]
#[allow(clippy::manual_is_multiple_of, dead_code)]
mod launcher;
#[path = "../model.rs"]
#[allow(dead_code)]
mod model;
#[path = "../places.rs"]
#[allow(dead_code)]
mod places;
#[path = "../platform/mod.rs"]
#[allow(dead_code, unused_imports)]
mod platform;
#[path = "../sdl_control_view.rs"]
mod sdl_control_view;
#[path = "../sdl_launcher_view.rs"]
mod sdl_launcher_view;
#[path = "../sdl_live_shell.rs"]
mod sdl_live_shell;
#[path = "../sdl_shell.rs"]
#[allow(dead_code)]
mod sdl_shell;

use nickel_components::SdlComponentRenderer;
use sdl_live_shell::LiveShell;
use sdl_shell::{SdlShell, ShellEvent, SurfaceId, SurfaceRole};

fn render_all(
    shell: &mut SdlShell,
    renderers: &mut HashMap<SurfaceId, SdlComponentRenderer>,
    state: &mut LiveShell,
) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .map(|surface| {
            let (width, height) = surface.window().size_in_pixels();
            let (logical_width, logical_height) = surface.window().size();
            (
                surface.id(),
                surface.role(),
                width,
                height,
                logical_width,
                logical_height,
                surface.window().display_scale(),
            )
        })
        .collect::<Vec<_>>();
    for (id, role, width, height, logical_width, logical_height, scale) in surfaces {
        if !state.surface_visible(role) {
            continue;
        }
        let renderer = renderers
            .entry(id)
            .or_insert_with(|| SdlComponentRenderer::new(width, height, scale));
        renderer.resize(width, height, scale);
        shell.present(
            id,
            renderer,
            &state.scene(role, logical_width, logical_height),
        )?;
    }
    Ok(())
}

fn render_role(
    shell: &mut SdlShell,
    renderers: &mut HashMap<SurfaceId, SdlComponentRenderer>,
    state: &mut LiveShell,
    wanted: SurfaceRole,
) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .filter(|surface| surface.role() == wanted)
        .map(|surface| {
            let (width, height) = surface.window().size_in_pixels();
            let (logical_width, logical_height) = surface.window().size();
            (
                surface.id(),
                surface.role(),
                width,
                height,
                logical_width,
                logical_height,
                surface.window().display_scale(),
            )
        })
        .collect::<Vec<_>>();
    for (id, role, width, height, logical_width, logical_height, scale) in surfaces {
        if !state.surface_visible(role) {
            continue;
        }
        let renderer = renderers
            .entry(id)
            .or_insert_with(|| SdlComponentRenderer::new(width, height, scale));
        renderer.resize(width, height, scale);
        shell.present(
            id,
            renderer,
            &state.scene(role, logical_width, logical_height),
        )?;
    }
    Ok(())
}

fn sync_visibility(shell: &mut SdlShell, state: &LiveShell) {
    let surfaces = shell
        .surfaces()
        .map(|surface| (surface.id(), surface.role()))
        .collect::<Vec<_>>();
    for (id, role) in surfaces {
        if state.surface_visible(role) {
            shell.show(id);
        } else {
            shell.hide(id);
        }
    }
}

fn main() -> Result<(), String> {
    nickel_logging::init("nickel-sdl-shell").map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut shell = SdlShell::new(started)?;
    shell.create_shell_surfaces()?;
    let mut state = LiveShell::new();
    let mut renderers = HashMap::<SurfaceId, SdlComponentRenderer>::new();
    sync_visibility(&mut shell, &state);
    render_all(&mut shell, &mut renderers, &mut state)?;
    println!(
        "time_to_first_shell_ms={:.3}",
        started.elapsed().as_secs_f64() * 1_000.0
    );

    tracing::info!(
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "SDL Nickel shell presented"
    );
    let mut refresh_deadline = Instant::now() + Duration::from_millis(100);
    let mut launcher_hover_deadline: Option<Instant> = None;
    loop {
        let next_deadline = launcher_hover_deadline
            .map(|deadline| deadline.min(refresh_deadline))
            .unwrap_or(refresh_deadline);
        let timeout = next_deadline.saturating_duration_since(Instant::now());
        match shell.wait_event_timeout(timeout) {
            Some(ShellEvent::Quit) | Some(ShellEvent::CloseRequested(_)) => break,
            Some(ShellEvent::PointerButton {
                surface,
                pressed: true,
                x,
                ..
            }) if shell
                .surface(surface)
                .is_some_and(|entry| entry.role() == SurfaceRole::Panel) =>
            {
                let width = shell
                    .surface(surface)
                    .map(|entry| entry.window().size().0)
                    .unwrap_or_default();
                if state.panel_click(x, width) {
                    sync_visibility(&mut shell, &state);
                    render_role(
                        &mut shell,
                        &mut renderers,
                        &mut state,
                        SurfaceRole::Launcher,
                    )?;
                    render_role(
                        &mut shell,
                        &mut renderers,
                        &mut state,
                        SurfaceRole::ControlCenter,
                    )?;
                }
            }
            Some(ShellEvent::PointerButton {
                surface,
                pressed: true,
                x,
                y,
                ..
            }) if shell.surface(surface).is_some_and(|entry| {
                matches!(
                    entry.role(),
                    SurfaceRole::Launcher | SurfaceRole::ControlCenter
                )
            }) =>
            {
                let role = shell
                    .surface(surface)
                    .map(|entry| entry.role())
                    .unwrap_or(SurfaceRole::Desktop);
                let (width, height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                let changed = match role {
                    SurfaceRole::Launcher => state.launcher_click(x, y),
                    SurfaceRole::ControlCenter => state.control_click(x, y, width, height),
                    _ => false,
                };
                if changed {
                    sync_visibility(&mut shell, &state);
                    render_role(&mut shell, &mut renderers, &mut state, role)?;
                }
            }
            // SDL reports an initial focus loss while a newly shown Wayland
            // surface is waiting for the compositor's focus configure. Hiding
            // an overlay here races its first frame and leaves a brief blank
            // window. Explicit dismissal and Escape remain authoritative.
            Some(ShellEvent::FocusChanged { .. }) => {}
            Some(ShellEvent::Text { surface, value }) => {
                if state.insert_launcher_text(&value) {
                    if let Some(role) = shell.surface(surface).map(|entry| entry.role()) {
                        render_role(&mut shell, &mut renderers, &mut state, role)?;
                    }
                }
            }
            Some(ShellEvent::Key {
                surface,
                key,
                modifiers,
                pressed: true,
                ..
            }) => {
                if state.launcher_key(key, modifiers) {
                    if let Some(role) = shell.surface(surface).map(|entry| entry.role()) {
                        render_role(&mut shell, &mut renderers, &mut state, role)?;
                    }
                }
            }
            Some(ShellEvent::PointerMoved { surface, x, y }) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if !matches!(
                    role,
                    Some(SurfaceRole::Launcher | SurfaceRole::ControlCenter)
                ) {
                    continue;
                }
                if state.pointer_moved(x, y) {
                    launcher_hover_deadline = Some(Instant::now() + Duration::from_millis(24));
                }
            }
            Some(ShellEvent::MouseWheel { surface, y, .. }) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if !matches!(
                    role,
                    Some(SurfaceRole::Launcher | SurfaceRole::ControlCenter)
                ) {
                    continue;
                }
                if state.scroll(y) {
                    render_role(
                        &mut shell,
                        &mut renderers,
                        &mut state,
                        role.unwrap_or(SurfaceRole::Desktop),
                    )?;
                }
            }
            Some(ShellEvent::PixelResize {
                surface,
                width,
                height,
                scale,
            }) => {
                let Some(role) = shell.surface(surface).map(|entry| entry.role()) else {
                    continue;
                };
                let renderer = renderers
                    .entry(surface)
                    .or_insert_with(|| SdlComponentRenderer::new(width, height, scale));
                renderer.resize(width, height, scale);
                let (logical_width, logical_height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or((width, height));
                shell.present(
                    surface,
                    renderer,
                    &state.scene(role, logical_width, logical_height),
                )?;
            }
            Some(ShellEvent::Redraw(surface)) => {
                let Some(entry) = shell.surface(surface) else {
                    continue;
                };
                let (width, height) = entry.window().size_in_pixels();
                let (logical_width, logical_height) = entry.window().size();
                let role = entry.role();
                let scale = entry.window().display_scale();
                let renderer = renderers
                    .entry(surface)
                    .or_insert_with(|| SdlComponentRenderer::new(width, height, scale));
                shell.present(
                    surface,
                    renderer,
                    &state.scene(role, logical_width, logical_height),
                )?;
            }
            Some(event) => tracing::debug!(?event, "SDL shell event"),
            None => {}
        }
        if launcher_hover_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            render_role(
                &mut shell,
                &mut renderers,
                &mut state,
                SurfaceRole::Launcher,
            )?;
            launcher_hover_deadline = None;
        }
        if Instant::now() >= refresh_deadline {
            if state.refresh() {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut renderers, &mut state)?;
            }
            refresh_deadline = Instant::now() + Duration::from_millis(100);
        }
    }
    Ok(())
}
