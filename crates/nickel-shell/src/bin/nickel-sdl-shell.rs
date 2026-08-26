use nickel_codex_ui::{ChatApplication, ShellRequest, shell_application};
use nickel_ui::{ApplicationHost, Point, Shortcut, UiEvent};
use sdl3::{
    keyboard::{Keycode, Mod},
    mouse::MouseButton,
};
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
#[path = "../notification.rs"]
mod notification;
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

use sdl_live_shell::LiveShell;
use sdl_shell::{SdlShell, ShellEvent, SurfaceId, SurfaceRole};

struct CodexSurfaces {
    hub: SurfaceId,
    hub_host: ApplicationHost<ChatApplication>,
    chats: Vec<(SurfaceId, ApplicationHost<ChatApplication>)>,
    cursors: HashMap<SurfaceId, Point>,
}

impl CodexSurfaces {
    fn new(shell: &SdlShell) -> Result<Self, String> {
        let hub = shell
            .surfaces()
            .find(|surface| surface.role() == SurfaceRole::CodexHub)
            .ok_or_else(|| "Codex hub surface is missing".to_owned())?;
        let (width, height) = hub.window().size();
        let application = shell_application(
            std::env::current_dir().map_err(|error| error.to_string())?,
            true,
            None,
            None,
        )?;
        Ok(Self {
            hub: hub.id(),
            hub_host: ApplicationHost::new(application, width, height),
            chats: Vec::new(),
            cursors: HashMap::new(),
        })
    }

    fn present(&self, shell: &mut SdlShell, surface: SurfaceId) -> Result<(), String> {
        if surface == self.hub {
            shell.present(surface, self.hub_host.commands())?;
        } else if let Some((_, host)) = self.chats.iter().find(|(id, _)| *id == surface) {
            shell.present(surface, host.commands())?;
        }
        Ok(())
    }

    fn host_mut(&mut self, surface: SurfaceId) -> Option<&mut ApplicationHost<ChatApplication>> {
        if surface == self.hub {
            Some(&mut self.hub_host)
        } else {
            self.chats
                .iter_mut()
                .find(|(id, _)| *id == surface)
                .map(|(_, host)| host)
        }
    }

    fn remove(&mut self, shell: &mut SdlShell, surface: SurfaceId) {
        if let Some(index) = self.chats.iter().position(|(id, _)| *id == surface) {
            self.chats.remove(index);
            self.cursors.remove(&surface);
            shell.destroy_surface(surface);
        }
    }

    fn open_requests(&mut self, shell: &mut SdlShell) -> Result<(), String> {
        for request in self.hub_host.application_mut().take_shell_requests() {
            let (cwd, project_id, thread) = match request {
                ShellRequest::OpenProject { cwd, project_id } => (cwd, project_id, None),
                ShellRequest::OpenThread {
                    cwd,
                    project_id,
                    thread,
                } => (cwd, project_id, Some(thread)),
            };
            let title = format!("Codex — {}", cwd.display());
            let id = shell.create_codex_chat_surface(&title)?;
            let (width, height) = shell
                .surface(id)
                .map(|surface| surface.window().size())
                .unwrap_or((1120, 760));
            let host = ApplicationHost::new(
                shell_application(cwd, false, thread, Some(project_id))?,
                width,
                height,
            );
            self.chats.push((id, host));
            self.present(shell, id)?;
            shell.show(id);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(not(target_os = "macos"))]
const REFRESH_INTERVAL: Duration = Duration::from_millis(100);

fn render_all(shell: &mut SdlShell, state: &mut LiveShell) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (surface.id(), surface.role(), logical_width, logical_height)
        })
        .collect::<Vec<_>>();
    for (id, role, logical_width, logical_height) in surfaces {
        if matches!(role, SurfaceRole::CodexHub | SurfaceRole::CodexChat) {
            continue;
        }
        if !state.surface_visible(role) {
            continue;
        }
        shell.present(id, &state.scene(role, logical_width, logical_height))?;
    }
    Ok(())
}

fn render_role(
    shell: &mut SdlShell,
    state: &mut LiveShell,
    wanted: SurfaceRole,
) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .filter(|surface| surface.role() == wanted)
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (surface.id(), surface.role(), logical_width, logical_height)
        })
        .collect::<Vec<_>>();
    for (id, role, logical_width, logical_height) in surfaces {
        if !state.surface_visible(role) {
            continue;
        }
        shell.present(id, &state.scene(role, logical_width, logical_height))?;
    }
    Ok(())
}

fn prewarm_role(
    shell: &mut SdlShell,
    state: &mut LiveShell,
    wanted: SurfaceRole,
) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .filter(|surface| surface.role() == wanted)
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (surface.id(), logical_width, logical_height)
        })
        .collect::<Vec<_>>();
    for (id, logical_width, logical_height) in surfaces {
        shell.present(id, &state.scene(wanted, logical_width, logical_height))?;
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

fn focus_visible_overlay(shell: &mut SdlShell, state: &LiveShell) {
    for role in [
        SurfaceRole::Launcher,
        SurfaceRole::ControlCenter,
        SurfaceRole::CodexHub,
    ] {
        if state.surface_visible(role) {
            shell.raise_role(role);
        }
    }
}

fn handle_codex_event(
    codex: &mut CodexSurfaces,
    shell: &mut SdlShell,
    state: &mut LiveShell,
    event: &ShellEvent,
) -> Result<bool, String> {
    let surface = match event {
        ShellEvent::PointerMoved { surface, .. }
        | ShellEvent::PointerButton { surface, .. }
        | ShellEvent::MouseWheel { surface, .. }
        | ShellEvent::Key { surface, .. }
        | ShellEvent::Text { surface, .. }
        | ShellEvent::Ime { surface, .. }
        | ShellEvent::FocusChanged { surface, .. }
        | ShellEvent::LogicalResize { surface, .. }
        | ShellEvent::PixelResize { surface, .. }
        | ShellEvent::Redraw(surface)
        | ShellEvent::CloseRequested(surface) => *surface,
        _ => return Ok(false),
    };
    if !shell
        .surface(surface)
        .is_some_and(|entry| matches!(entry.role(), SurfaceRole::CodexHub | SurfaceRole::CodexChat))
    {
        return Ok(false);
    }
    if matches!(event, ShellEvent::CloseRequested(_)) {
        if surface == codex.hub {
            state.hide_overlay(SurfaceRole::CodexHub);
            shell.hide(surface);
        } else {
            codex.remove(shell, surface);
        }
        return Ok(true);
    }
    if matches!(
        event,
        ShellEvent::LogicalResize { .. } | ShellEvent::PixelResize { .. }
    ) {
        let (width, height) = shell
            .surface(surface)
            .map(|entry| entry.window().size())
            .unwrap_or((1, 1));
        if let Some(host) = codex.host_mut(surface) {
            host.resize(width, height);
            codex.present(shell, surface)?;
        }
        return Ok(true);
    }
    let shortcut = match event {
        ShellEvent::Key {
            key: Some(Keycode::Return),
            modifiers,
            pressed: true,
            ..
        } if !modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => Some(Shortcut::Submit),
        ShellEvent::Key {
            key: Some(Keycode::Escape),
            pressed: true,
            ..
        } => Some(Shortcut::Escape),
        _ => None,
    };
    if let Some(shortcut) = shortcut
        && codex
            .host_mut(surface)
            .is_some_and(|host| host.shortcut(shortcut))
    {
        codex.present(shell, surface)?;
        return Ok(true);
    }
    let command = |modifiers: &Mod| {
        modifiers.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD | Mod::LGUIMOD | Mod::RGUIMOD)
    };
    let ui_event = match event {
        ShellEvent::PointerMoved { x, y, .. } => {
            let point = Point { x: *x, y: *y };
            codex.cursors.insert(surface, point);
            Some(UiEvent::PointerMoved(point))
        }
        ShellEvent::PointerButton {
            button: MouseButton::Left,
            pressed: true,
            x,
            y,
            ..
        } => Some(UiEvent::PointerPressed(Point { x: *x, y: *y })),
        ShellEvent::PointerButton {
            button: MouseButton::Left,
            pressed: false,
            x,
            y,
            ..
        } => Some(UiEvent::PointerReleased(Point { x: *x, y: *y })),
        ShellEvent::MouseWheel { x, y, .. } if x.abs() > y.abs() => {
            Some(UiEvent::ScrollHorizontal {
                point: codex.cursors.get(&surface).copied().unwrap_or_default(),
                delta_x: -*x * 42.0,
            })
        }
        ShellEvent::MouseWheel { y, .. } => Some(UiEvent::Scroll {
            point: codex.cursors.get(&surface).copied().unwrap_or_default(),
            delta_y: -*y * 42.0,
        }),
        ShellEvent::Text { value, .. } => Some(UiEvent::TextInput(value.clone())),
        ShellEvent::Ime { value, .. } => Some(UiEvent::ImePreedit(value.clone())),
        ShellEvent::FocusChanged { focused: false, .. } => Some(UiEvent::FocusLost),
        ShellEvent::Key {
            key: Some(Keycode::Return),
            modifiers,
            pressed: true,
            ..
        } if modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => {
            Some(UiEvent::TextInput("\n".into()))
        }
        ShellEvent::Key {
            key: Some(Keycode::Return),
            pressed: true,
            ..
        } => Some(UiEvent::KeyboardActivate),
        ShellEvent::Key {
            key: Some(Keycode::Escape),
            pressed: true,
            ..
        } => Some(UiEvent::Dismiss),
        ShellEvent::Key {
            key: Some(Keycode::Backspace),
            pressed: true,
            ..
        } => Some(UiEvent::TextBackspace),
        ShellEvent::Key {
            key: Some(Keycode::Delete),
            pressed: true,
            ..
        } => Some(UiEvent::TextDelete),
        ShellEvent::Key {
            key: Some(Keycode::Tab),
            modifiers,
            pressed: true,
            ..
        } if modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => Some(UiEvent::FocusPrevious),
        ShellEvent::Key {
            key: Some(Keycode::Tab),
            pressed: true,
            ..
        } => Some(UiEvent::FocusNext),
        ShellEvent::Key {
            key: Some(Keycode::A),
            modifiers,
            pressed: true,
            ..
        } if command(modifiers) => Some(UiEvent::TextSelectAll),
        ShellEvent::Key {
            key: Some(Keycode::C),
            modifiers,
            pressed: true,
            ..
        } if command(modifiers) => Some(UiEvent::TextCopy),
        ShellEvent::Key {
            key: Some(Keycode::V),
            modifiers,
            pressed: true,
            ..
        } if command(modifiers) => shell.clipboard_text().map(UiEvent::TextPaste),
        ShellEvent::Key {
            key: Some(Keycode::Left),
            modifiers,
            pressed: true,
            ..
        } => Some(UiEvent::TextMoveLeft {
            extend_selection: modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        }),
        ShellEvent::Key {
            key: Some(Keycode::Right),
            modifiers,
            pressed: true,
            ..
        } => Some(UiEvent::TextMoveRight {
            extend_selection: modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        }),
        _ => None,
    };
    if let Some(ui_event) = ui_event {
        let outcome = codex
            .host_mut(surface)
            .expect("Codex host exists")
            .handle_event(ui_event);
        if let Some(text) = outcome.clipboard_text {
            shell.set_clipboard_text(&text);
        }
        if outcome.changed {
            codex.present(shell, surface)?;
        }
    }
    codex.open_requests(shell)?;
    Ok(true)
}

fn main() -> Result<(), String> {
    nickel_logging::init("nickel-sdl-shell").map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut shell = SdlShell::new(started)?;
    shell.create_shell_surfaces()?;
    let mut state = LiveShell::new()?;
    let mut codex = CodexSurfaces::new(&shell)?;
    let hotkey_rx = platform::launcher_hotkey_receiver();
    sync_visibility(&mut shell, &state);
    render_all(&mut shell, &mut state)?;
    codex.present(&mut shell, codex.hub)?;
    println!(
        "time_to_first_shell_ms={:.3}",
        started.elapsed().as_secs_f64() * 1_000.0
    );

    tracing::info!(
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "SDL Nickel shell presented"
    );
    let launcher_warm_started = Instant::now();
    prewarm_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
    tracing::info!(
        elapsed_ms = launcher_warm_started.elapsed().as_secs_f64() * 1_000.0,
        "SDL launcher presenter and frame prewarmed"
    );
    let mut refresh_deadline = Instant::now() + REFRESH_INTERVAL;
    let mut hover_repaint: Option<(SurfaceRole, Instant)> = None;
    loop {
        while let Ok(shortcut) = hotkey_rx.try_recv() {
            if state.global_shortcut(shortcut) {
                sync_visibility(&mut shell, &state);
                focus_visible_overlay(&mut shell, &state);
                render_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
                render_role(&mut shell, &mut state, SurfaceRole::ControlCenter)?;
            }
        }
        let next_deadline = hover_repaint
            .map(|(_, deadline)| deadline.min(refresh_deadline))
            .unwrap_or(refresh_deadline);
        let timeout = next_deadline.saturating_duration_since(Instant::now());
        let event = shell.wait_event_timeout(timeout);
        if let Some(ref event) = event
            && handle_codex_event(&mut codex, &mut shell, &mut state, event)?
        {
            continue;
        }
        match event {
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
                    focus_visible_overlay(&mut shell, &state);
                    // The launcher owns a persistent accelerated presenter and a
                    // pre-rendered buffer. Showing it must not synchronously
                    // rebuild and submit the whole scene on the click path.
                    render_role(&mut shell, &mut state, SurfaceRole::ControlCenter)?;
                }
            }
            Some(ShellEvent::PointerButton {
                surface,
                pressed: true,
                ..
            }) if shell
                .surface(surface)
                .is_some_and(|entry| entry.role() == SurfaceRole::Notification) =>
            {
                if state.dismiss_notification() {
                    sync_visibility(&mut shell, &state);
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
                    SurfaceRole::Notification => state.dismiss_notification(),
                    _ => false,
                };
                if changed {
                    sync_visibility(&mut shell, &state);
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            // SDL reports an initial focus loss while a newly shown Wayland
            // surface is waiting for the compositor's focus configure. Hiding
            // an overlay here races its first frame and leaves a brief blank
            // window. Explicit dismissal and Escape remain authoritative.
            Some(ShellEvent::FocusChanged {
                surface: _surface,
                focused: false,
            }) => {
                #[cfg(target_os = "macos")]
                if let Some(role @ (SurfaceRole::Launcher | SurfaceRole::ControlCenter)) =
                    shell.surface(_surface).map(|entry| entry.role())
                    && state.hide_overlay(role)
                {
                    sync_visibility(&mut shell, &state);
                }
            }
            Some(ShellEvent::FocusChanged { .. }) => {}
            Some(ShellEvent::Text { surface, value }) => {
                if state.insert_launcher_text(&value)
                    && let Some(role) = shell.surface(surface).map(|entry| entry.role())
                {
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            Some(ShellEvent::Key {
                surface,
                key,
                modifiers,
                pressed: true,
                ..
            }) => {
                if state.launcher_key(key, modifiers)
                    && let Some(role) = shell.surface(surface).map(|entry| entry.role())
                {
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            Some(ShellEvent::PointerMoved { surface, x, y }) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if role == Some(SurfaceRole::Panel) {
                    let width = shell
                        .surface(surface)
                        .map(|entry| entry.window().size().0)
                        .unwrap_or_default();
                    if state.panel_pointer_moved(x, width) {
                        hover_repaint = Some((
                            SurfaceRole::Panel,
                            Instant::now() + Duration::from_millis(24),
                        ));
                    }
                    continue;
                }
                if !matches!(
                    role,
                    Some(SurfaceRole::Launcher | SurfaceRole::ControlCenter)
                ) {
                    continue;
                }
                if state.pointer_moved(x, y) {
                    hover_repaint =
                        Some((role.unwrap(), Instant::now() + Duration::from_millis(24)));
                }
            }
            Some(ShellEvent::PointerEntered {
                surface,
                entered: false,
            }) if shell
                .surface(surface)
                .is_some_and(|entry| entry.role() == SurfaceRole::Panel) =>
            {
                if state.panel_pointer_left() {
                    render_role(&mut shell, &mut state, SurfaceRole::Panel)?;
                }
            }
            Some(ShellEvent::PointerEntered { .. }) => {}
            Some(ShellEvent::MouseWheel { surface, y, .. }) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if !matches!(
                    role,
                    Some(SurfaceRole::Launcher | SurfaceRole::ControlCenter)
                ) {
                    continue;
                }
                if state.scroll(y) {
                    render_role(&mut shell, &mut state, role.unwrap_or(SurfaceRole::Desktop))?;
                }
            }
            Some(ShellEvent::PixelResize { surface, .. }) => {
                let Some(role) = shell.surface(surface).map(|entry| entry.role()) else {
                    continue;
                };
                let (logical_width, logical_height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                shell.present(surface, &state.scene(role, logical_width, logical_height))?;
            }
            Some(ShellEvent::Redraw(surface)) => {
                let Some(entry) = shell.surface(surface) else {
                    continue;
                };
                let (logical_width, logical_height) = entry.window().size();
                let role = entry.role();
                shell.present(surface, &state.scene(role, logical_width, logical_height))?;
            }
            Some(event) => tracing::debug!(?event, "SDL shell event"),
            None => {}
        }
        if hover_repaint.is_some_and(|(_, deadline)| Instant::now() >= deadline)
            && let Some((role, _)) = hover_repaint.take()
        {
            render_role(&mut shell, &mut state, role)?;
        }
        if Instant::now() >= refresh_deadline {
            let mut codex_redraw = Vec::new();
            if codex.hub_host.poll() {
                codex_redraw.push(codex.hub);
            }
            for (surface, host) in &mut codex.chats {
                if host.poll() {
                    codex_redraw.push(*surface);
                }
            }
            for surface in codex_redraw {
                codex.present(&mut shell, surface)?;
            }
            codex.open_requests(&mut shell)?;
            if state.refresh() {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
            }
            refresh_deadline = Instant::now() + REFRESH_INTERVAL;
        }
    }
    Ok(())
}
