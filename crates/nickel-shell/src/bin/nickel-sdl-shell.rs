use nickel_codex::ThreadId;
use nickel_codex_ui::{ChatApplication, ConnectionStatus, ShellRequest, shell_application};
use nickel_input::{
    InputEvent, KeyCode, KeyEdge, LogicalKey, ModifierState, NamedKey, PointerButton, PointerEvent,
    controller::{AxisDirection, ControllerButton, ControllerSignal},
};
use nickel_ui::{ApplicationHost, UiEvent};
use std::{
    collections::HashSet,
    path::{Component, Path},
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
#[cfg(target_os = "linux")]
#[path = "../lock_auth.rs"]
mod lock_auth;
use launcher::{DashboardProject, DashboardSection, ProjectActivity, normalize_dashboard_projects};
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
#[path = "../sdl_gpu.rs"]
mod sdl_gpu;
#[path = "../sdl_launcher_view.rs"]
mod sdl_launcher_view;
#[path = "../sdl_live_shell.rs"]
mod sdl_live_shell;
#[path = "../sdl_screenshot.rs"]
mod sdl_screenshot;
#[path = "../sdl_shell.rs"]
#[allow(dead_code)]
mod sdl_shell;
#[path = "../sdl_window_preview.rs"]
mod sdl_window_preview;

use sdl_live_shell::LiveShell;
use sdl_shell::{SdlShell, ShellEvent, SurfaceId, SurfaceRole};

struct CodexSurfaces {
    project_menu: SurfaceId,
    project_menu_cwd: std::path::PathBuf,
    project_menu_host: Option<ApplicationHost<ChatApplication>>,
    chats: Vec<CodexChatSurface>,
    writer_leases: WriterLeases,
}

struct CodexChatSurface {
    id: SurfaceId,
    project_id: String,
    host: ApplicationHost<ChatApplication>,
    thread_id: Option<ThreadId>,
}

#[derive(Default)]
struct WriterLeases(HashSet<ThreadId>);

impl WriterLeases {
    fn acquire(&mut self, thread: &ThreadId) -> bool {
        self.0.insert(thread.clone())
    }

    fn release(&mut self, thread: &ThreadId) -> bool {
        self.0.remove(thread)
    }

    #[cfg(test)]
    fn contains(&self, thread: &ThreadId) -> bool {
        self.0.contains(thread)
    }
}

fn codex_project_application_id(project_id: Option<&str>, root: &Path) -> String {
    let identity = project_id.map(str::as_bytes).map_or_else(
        || {
            root.components()
                .filter_map(|component| match component {
                    Component::Prefix(prefix) => {
                        Some(prefix.as_os_str().to_string_lossy().into_owned())
                    }
                    Component::RootDir => Some(String::new()),
                    Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                    Component::ParentDir => Some("..".into()),
                    Component::CurDir => None,
                })
                .collect::<Vec<_>>()
                .join("/")
                .into_bytes()
        },
        <[u8]>::to_vec,
    );
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in identity {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("io.nickel.codex.project.{hash:016x}")
}

impl CodexSurfaces {
    fn new(shell: &SdlShell) -> Result<Self, String> {
        let project_menu = shell
            .surfaces()
            .find(|surface| surface.role() == SurfaceRole::CodexProjectMenu)
            .ok_or_else(|| "Codex project_menu surface is missing".to_owned())?;
        Ok(Self {
            project_menu: project_menu.id(),
            project_menu_cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            project_menu_host: None,
            chats: Vec::new(),
            writer_leases: WriterLeases::default(),
        })
    }

    fn ensure_project_menu(&mut self, shell: &SdlShell) -> Result<(), String> {
        if self.project_menu_host.is_some() {
            return Ok(());
        }
        let (width, height) = shell
            .surface(self.project_menu)
            .map(|surface| surface.window().size())
            .ok_or_else(|| "Codex project_menu surface is missing".to_owned())?;
        let application = shell_application(self.project_menu_cwd.clone(), true, None, None)?;
        self.project_menu_host = Some(ApplicationHost::new(application, width, height));
        Ok(())
    }

    fn present(&mut self, shell: &mut SdlShell, surface: SurfaceId) -> Result<(), String> {
        if surface == self.project_menu {
            self.ensure_project_menu(shell)?;
            shell.present(
                surface,
                self.project_menu_host
                    .as_ref()
                    .expect("Codex project_menu initialized")
                    .commands(),
            )?;
        } else if let Some(chat) = self.chats.iter().find(|chat| chat.id == surface) {
            shell.present(surface, chat.host.commands())?;
        }
        Ok(())
    }

    fn host_mut(&mut self, surface: SurfaceId) -> Option<&mut ApplicationHost<ChatApplication>> {
        if surface == self.project_menu {
            self.project_menu_host.as_mut()
        } else {
            self.chats
                .iter_mut()
                .find(|chat| chat.id == surface)
                .map(|chat| &mut chat.host)
        }
    }

    fn remove(&mut self, shell: &mut SdlShell, surface: SurfaceId) {
        if let Some(index) = self.chats.iter().position(|chat| chat.id == surface) {
            if let Some(thread) = self.chats.remove(index).thread_id {
                self.writer_leases.release(&thread);
            }
            shell.destroy_surface(surface);
        }
    }

    fn open_requests(&mut self, shell: &mut SdlShell) -> Result<bool, String> {
        let Some(project_menu_host) = self.project_menu_host.as_mut() else {
            return Ok(false);
        };
        let mut opened = false;
        for request in project_menu_host.application_mut().take_shell_requests() {
            opened = true;
            if let ShellRequest::OpenProject {
                cwd,
                project_id,
                name,
                initial_thread,
            } = request
            {
                self.open_project(shell, cwd, project_id, name, initial_thread)?;
            }
        }
        Ok(opened)
    }

    fn release_failed_resumes(&mut self) {
        for chat in &mut self.chats {
            for request in chat.host.application_mut().take_shell_requests() {
                if let ShellRequest::ResumeFailed(thread) = request
                    && chat.thread_id.as_ref() == Some(&thread)
                {
                    self.writer_leases.release(&thread);
                    chat.thread_id = None;
                }
            }
        }
    }

    fn resume_requests(&mut self) {
        for chat in &mut self.chats {
            let requests = chat.host.application_mut().take_shell_requests();
            for request in requests {
                match request {
                    ShellRequest::ResumeThread(thread) => {
                        if chat.thread_id.as_ref() == Some(&thread) {
                            continue;
                        }
                        if !self.writer_leases.acquire(&thread) {
                            chat.host.application_mut().report_resume_rejection(format!(
                                "Conversation {} already has a Nickel writer",
                                thread.0
                            ));
                            continue;
                        }
                        if let Err(error) =
                            chat.host.application_mut().resume_thread(thread.clone())
                        {
                            self.writer_leases.release(&thread);
                            chat.host.application_mut().report_resume_rejection(error);
                            continue;
                        }
                        if let Some(previous) = chat.thread_id.replace(thread) {
                            self.writer_leases.release(&previous);
                        }
                    }
                    ShellRequest::ResumeFailed(thread) => {
                        if chat.thread_id.as_ref() == Some(&thread) {
                            self.writer_leases.release(&thread);
                            chat.thread_id = None;
                        }
                    }
                    ShellRequest::OpenProject { .. } => {}
                }
            }
        }
    }

    fn open_project(
        &mut self,
        shell: &mut SdlShell,
        cwd: std::path::PathBuf,
        project_id: String,
        name: String,
        initial_thread: Option<ThreadId>,
    ) -> Result<(), String> {
        if let Some(chat) = self.chats.iter().find(|chat| {
            chat.project_id == project_id
                && (initial_thread.is_some() && chat.thread_id == initial_thread
                    || initial_thread.is_none() && chat.thread_id.is_none())
        }) {
            shell.show(chat.id);
            shell.raise(chat.id);
            return Ok(());
        }
        if let Some(thread) = &initial_thread
            && !self.writer_leases.acquire(thread)
        {
            return Err(format!("thread {} already has a Nickel writer", thread.0));
        }
        let result = (|| {
            let title = format!("Codex — {name}");
            let application_id = codex_project_application_id(Some(&project_id), &cwd);
            let id = shell.create_codex_chat_surface(&title, &application_id)?;
            let (width, height) = shell
                .surface(id)
                .map(|surface| surface.window().size())
                .unwrap_or((1120, 760));
            let host = ApplicationHost::new(
                shell_application(cwd, false, initial_thread.clone(), Some(project_id.clone()))?,
                width,
                height,
            );
            self.chats.push(CodexChatSurface {
                id,
                project_id,
                host,
                thread_id: initial_thread.clone(),
            });
            self.present(shell, id)?;
            shell.show(id);
            Ok(())
        })();
        if result.is_err()
            && let Some(thread) = &initial_thread
        {
            self.writer_leases.release(thread);
        }
        result
    }

    fn open_project_by_id(&mut self, shell: &mut SdlShell, project_id: &str) -> Result<(), String> {
        let (project, initial_thread) = {
            let state = &self
                .project_menu_host
                .as_mut()
                .ok_or_else(|| "Codex project data is still loading".to_owned())?
                .application_mut()
                .state;
            let project = state
                .projects
                .iter()
                .find(|project| project.id == project_id)
                .cloned()
                .ok_or_else(|| format!("Codex project {project_id} is unavailable"))?;
            let initial_thread = state
                .threads
                .iter()
                .filter(|thread| {
                    state
                        .thread_runtime
                        .get(&thread.id)
                        .and_then(|runtime| runtime.project_id.as_deref())
                        .map_or_else(
                            || {
                                thread.cwd.as_ref().is_some_and(|cwd| {
                                    project
                                        .roots
                                        .iter()
                                        .any(|root| cwd == root || cwd.starts_with(root))
                                })
                            },
                            |id| id == project.id,
                        )
                })
                .max_by_key(|thread| thread.last_used_at)
                .map(|thread| thread.id.clone());
            (project, initial_thread)
        };
        let cwd = project
            .roots
            .first()
            .cloned()
            .ok_or_else(|| format!("Codex project {} has no root", project.id))?;
        self.open_project(shell, cwd, project.id, project.name, initial_thread)
    }
}

#[cfg(target_os = "macos")]
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(not(target_os = "macos"))]
const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const SYSTEM_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

fn render_all(shell: &mut SdlShell, state: &mut LiveShell) -> Result<(), String> {
    let surfaces = shell
        .surfaces()
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (surface.id(), surface.role(), logical_width, logical_height)
        })
        .collect::<Vec<_>>();
    for (id, role, logical_width, logical_height) in surfaces {
        if matches!(role, SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat) {
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
        #[cfg(target_os = "linux")]
        if role == SurfaceRole::Launcher {
            continue;
        }
        if state.surface_visible(role) {
            shell.show(id);
        } else {
            shell.hide(id);
        }
    }
}

fn focus_visible_overlay(shell: &mut SdlShell, state: &LiveShell) {
    for role in [
        SurfaceRole::Lock,
        SurfaceRole::Screenshot,
        SurfaceRole::Launcher,
        SurfaceRole::ControlCenter,
        SurfaceRole::CodexProjectMenu,
    ] {
        #[cfg(target_os = "linux")]
        if role == SurfaceRole::Launcher {
            continue;
        }
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
        ShellEvent::Input { surface, .. }
        | ShellEvent::FocusChanged { surface, .. }
        | ShellEvent::LogicalResize { surface, .. }
        | ShellEvent::PixelResize { surface, .. }
        | ShellEvent::Redraw(surface)
        | ShellEvent::CloseRequested(surface) => *surface,
        _ => return Ok(false),
    };
    if !shell.surface(surface).is_some_and(|entry| {
        matches!(
            entry.role(),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat
        )
    }) {
        return Ok(false);
    }
    if matches!(
        event,
        ShellEvent::FocusChanged { focused: true, .. }
            | ShellEvent::Input {
                event: InputEvent::Pointer(PointerEvent::Button {
                    button: PointerButton::Primary,
                    edge: KeyEdge::Pressed,
                    ..
                }),
                ..
            }
    ) {
        shell.start_text_input(surface);
    }
    if surface == codex.project_menu {
        codex.ensure_project_menu(shell)?;
    }
    if matches!(event, ShellEvent::CloseRequested(_)) {
        if surface == codex.project_menu {
            state.hide_overlay(SurfaceRole::CodexProjectMenu);
            shell.hide(surface);
        } else {
            codex.remove(shell, surface);
        }
        return Ok(true);
    }
    if surface == codex.project_menu
        && (matches!(event, ShellEvent::FocusChanged { focused: false, .. })
            || matches!(
                event,
                ShellEvent::Input {
                    event: InputEvent::Key(key),
                    ..
                } if key.edge == KeyEdge::Pressed
                    && key.logical == LogicalKey::Named(NamedKey::Escape)
            ))
    {
        state.hide_overlay(SurfaceRole::CodexProjectMenu);
        shell.hide(surface);
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
    let outcome = match event {
        ShellEvent::Input { event, .. } => {
            let clipboard_text = shell.clipboard_text();
            codex
                .host_mut(surface)
                .expect("Codex host exists")
                .handle_input(event, clipboard_text.as_deref())
        }
        ShellEvent::FocusChanged { focused: false, .. } => codex
            .host_mut(surface)
            .expect("Codex host exists")
            .handle_event(UiEvent::FocusLost),
        _ => nickel_ui::HostEventOutcome::default(),
    };
    if let Some(text) = outcome.clipboard_text {
        shell.set_clipboard_text(&text);
    }
    if outcome.changed {
        codex.present(shell, surface)?;
    }
    if codex.open_requests(shell)? {
        state.hide_overlay(SurfaceRole::CodexProjectMenu);
        shell.hide(codex.project_menu);
    }
    codex.resume_requests();
    codex.release_failed_resumes();
    Ok(true)
}

fn handle_shell_input(
    shell: &mut SdlShell,
    state: &mut LiveShell,
    codex: &mut CodexSurfaces,
    surface: SurfaceId,
    event: InputEvent,
    hover_repaint: &mut Option<(SurfaceRole, Instant)>,
) -> Result<(), String> {
    let Some(role) = shell.surface(surface).map(|entry| entry.role()) else {
        return Ok(());
    };
    match event {
        InputEvent::Text(nickel_input::TextEvent::Commit { text, .. }) => {
            if role == SurfaceRole::Lock {
                if state.insert_lock_text(&text) {
                    render_role(shell, state, SurfaceRole::Lock)?;
                }
                return Ok(());
            }
            let started = Instant::now();
            let was_dashboard = state.launcher_is_dashboard();
            if state.insert_launcher_text(&text) {
                render_role(shell, state, role)?;
                if was_dashboard && std::env::var_os("NICKEL_PERF_METRICS").is_some() {
                    eprintln!(
                        "launcher_first_character_ms={:.3}",
                        started.elapsed().as_secs_f64() * 1_000.0
                    );
                }
            }
        }
        InputEvent::Text(nickel_input::TextEvent::Preedit { text, .. }) => {
            if role == SurfaceRole::Launcher && state.set_launcher_preedit(&text) {
                render_role(shell, state, SurfaceRole::Launcher)?;
            }
        }
        InputEvent::Key(key) if key.edge == KeyEdge::Pressed => {
            let keycode = match key.physical {
                nickel_input::PhysicalKey::Code(key) => Some(key),
                nickel_input::PhysicalKey::Native(_) => None,
            };
            let (width, height) = shell
                .surface(surface)
                .map(|entry| entry.window().size())
                .unwrap_or_default();
            let changed = match role {
                SurfaceRole::Lock => state.lock_key(keycode),
                SurfaceRole::ControlCenter => state.control_key(keycode, width, height),
                SurfaceRole::WindowPreview => state.preview_key(keycode),
                SurfaceRole::WindowContextMenu => state.window_menu_key(keycode),
                SurfaceRole::Notification => state.notification_key(keycode),
                SurfaceRole::Panel => state.preview_key(keycode),
                SurfaceRole::Launcher => state.launcher_key(keycode, &key.modifiers),
                SurfaceRole::Screenshot => state.screenshot_key(keycode),
                _ => false,
            };
            if changed {
                sync_visibility(shell, state);
                render_role(shell, state, role)?;
                if matches!(role, SurfaceRole::Panel | SurfaceRole::WindowPreview) {
                    render_role(shell, state, SurfaceRole::WindowPreview)?;
                    render_role(shell, state, SurfaceRole::WindowContextMenu)?;
                }
            }
        }
        InputEvent::Pointer(PointerEvent::Button {
            button,
            edge,
            position: Some(position),
            ..
        }) => {
            let x = position.x as f32;
            let y = position.y as f32;
            if role == SurfaceRole::Screenshot {
                let (width, height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                let changed = match edge {
                    KeyEdge::Pressed => state.screenshot_pointer_pressed(x, y, width, height),
                    KeyEdge::Released => state.screenshot_pointer_released(),
                };
                if changed {
                    sync_visibility(shell, state);
                    render_role(shell, state, SurfaceRole::Screenshot)?;
                }
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::WindowPreview {
                if state.preview_click(x, y, button == PointerButton::Secondary) {
                    sync_visibility(shell, state);
                    state.sync_transient_overlays();
                    render_role(shell, state, SurfaceRole::WindowPreview)?;
                    render_role(shell, state, SurfaceRole::WindowContextMenu)?;
                }
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::WindowContextMenu {
                if state.window_menu_click(x, y) {
                    sync_visibility(shell, state);
                }
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::Panel {
                if let Some(display) = shell.surface_display_geometry(surface) {
                    state.set_panel_origin_x(display.x);
                }
                let width = shell
                    .surface(surface)
                    .map(|entry| entry.window().size().0)
                    .unwrap_or_default();
                if state.panel_click(x, width) {
                    sync_visibility(shell, state);
                    state.sync_transient_overlays();
                    focus_visible_overlay(shell, state);
                    render_role(shell, state, SurfaceRole::ControlCenter)?;
                    render_role(shell, state, SurfaceRole::WindowPreview)?;
                    if state.surface_visible(SurfaceRole::CodexProjectMenu) {
                        codex.present(shell, codex.project_menu)?;
                    }
                }
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::Notification {
                let (width, height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                if state.notification_click(x, y, width, height) {
                    sync_visibility(shell, state);
                }
            } else if edge == KeyEdge::Pressed
                && matches!(role, SurfaceRole::Launcher | SurfaceRole::ControlCenter)
            {
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
                    sync_visibility(shell, state);
                    render_role(shell, state, role)?;
                }
            }
        }
        InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
            let x = position.x as f32;
            let y = position.y as f32;
            if role == SurfaceRole::Screenshot {
                let (width, height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                if state.screenshot_pointer_moved(x, y, width, height) {
                    render_role(shell, state, SurfaceRole::Screenshot)?;
                }
            } else if role == SurfaceRole::Panel {
                if let Some(display) = shell.surface_display_geometry(surface) {
                    state.set_panel_origin_x(display.x);
                }
                let width = shell
                    .surface(surface)
                    .map(|entry| entry.window().size().0)
                    .unwrap_or_default();
                if state.panel_pointer_moved(x, width) {
                    sync_visibility(shell, state);
                    state.sync_transient_overlays();
                    render_role(shell, state, SurfaceRole::WindowPreview)?;
                    *hover_repaint = Some((
                        SurfaceRole::Panel,
                        Instant::now() + Duration::from_millis(24),
                    ));
                }
            } else if role == SurfaceRole::WindowPreview {
                if state.preview_pointer_moved(x, y) {
                    *hover_repaint = Some((
                        SurfaceRole::WindowPreview,
                        Instant::now() + Duration::from_millis(24),
                    ));
                }
            } else if matches!(role, SurfaceRole::Launcher | SurfaceRole::ControlCenter)
                && state.pointer_moved(x, y)
            {
                *hover_repaint = Some((role, Instant::now() + Duration::from_millis(24)));
            }
        }
        InputEvent::Pointer(PointerEvent::Axis { delta, .. }) => {
            if matches!(role, SurfaceRole::Launcher | SurfaceRole::ControlCenter) {
                let started = Instant::now();
                if state.scroll(delta.y as f32) {
                    render_role(shell, state, role)?;
                    if std::env::var_os("NICKEL_PERF_METRICS").is_some() {
                        eprintln!(
                            "launcher_scroll_frame_ms={:.3}",
                            started.elapsed().as_secs_f64() * 1_000.0
                        );
                    }
                }
            }
        }
        InputEvent::FocusGained { .. }
        | InputEvent::FocusLost { .. }
        | InputEvent::DeviceRemoved { .. }
        | InputEvent::Key(_)
        | InputEvent::Pointer(_)
        | InputEvent::Touch(_) => {}
    }
    Ok(())
}

fn controller_navigation_key(signal: &ControllerSignal) -> Option<KeyCode> {
    match signal {
        ControllerSignal::Button {
            button,
            edge: KeyEdge::Pressed,
            ..
        } => Some(match button {
            ControllerButton::DPadUp => KeyCode::ArrowUp,
            ControllerButton::DPadDown => KeyCode::ArrowDown,
            ControllerButton::DPadLeft => KeyCode::ArrowLeft,
            ControllerButton::DPadRight => KeyCode::ArrowRight,
            ControllerButton::South => KeyCode::Enter,
            ControllerButton::East | ControllerButton::Select => KeyCode::Escape,
            _ => return None,
        }),
        ControllerSignal::Direction {
            direction,
            edge: KeyEdge::Pressed,
            ..
        } => Some(match direction {
            AxisDirection::Up => KeyCode::ArrowUp,
            AxisDirection::Down => KeyCode::ArrowDown,
            AxisDirection::Left => KeyCode::ArrowLeft,
            AxisDirection::Right => KeyCode::ArrowRight,
        }),
        _ => None,
    }
}

fn main() -> Result<(), String> {
    nickel_logging::init("nickel-sdl-shell").map_err(|error| error.to_string())?;
    if !platform::register_session_shell() {
        tracing::warn!("Nickel shell could not authenticate with the session protocol");
    }
    let started = Instant::now();
    let mut shell = SdlShell::new(started)?;
    shell.create_shell_surfaces()?;
    let mut state = LiveShell::new()?;
    let mut codex = CodexSurfaces::new(&shell)?;
    let hotkey_feed = platform::launcher_hotkey_receiver();
    tracing::info!(
        ownership = ?hotkey_feed.ownership,
        capability = ?hotkey_feed.capability,
        "global shortcut adapter initialized"
    );
    let hotkey_rx = hotkey_feed.receiver;
    let event_sender = shell.event_sender();
    std::thread::Builder::new()
        .name("nickel-shortcut-events".into())
        .spawn(move || {
            while let Ok(shortcut) = hotkey_rx.recv() {
                if event_sender.push_custom_event(shortcut).is_err() {
                    break;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    {
        let semantic_rx = platform::semantic_target_receiver();
        let event_sender = shell.event_sender();
        std::thread::Builder::new()
            .name("nickel-semantic-target-events".into())
            .spawn(move || {
                while let Ok(request) = semantic_rx.recv() {
                    if event_sender.push_custom_event(request).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
    }
    sync_visibility(&mut shell, &state);
    render_all(&mut shell, &mut state)?;
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
    if let Err(error) = codex.ensure_project_menu(&shell) {
        tracing::warn!(%error, "Codex integration is unavailable");
    }
    let mut refresh_deadline = Instant::now() + REFRESH_INTERVAL;
    let mut system_refresh_deadline = Instant::now() + SYSTEM_REFRESH_INTERVAL;
    let mut hover_repaint: Option<(SurfaceRole, Instant)> = None;
    let mut initial_exposures = HashSet::new();
    #[cfg(not(target_os = "linux"))]
    let mut focused_overlays = HashSet::new();
    #[cfg(not(target_os = "linux"))]
    let mut overlay_focus_loss: Option<(SurfaceId, SurfaceRole, Instant)> = None;
    loop {
        let next_deadline = refresh_deadline.min(system_refresh_deadline);
        let next_deadline = hover_repaint
            .map(|(_, deadline)| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        #[cfg(not(target_os = "linux"))]
        let next_deadline = overlay_focus_loss
            .map(|(_, _, deadline)| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        let timeout = next_deadline.saturating_duration_since(Instant::now());
        let event = shell.wait_event_timeout(timeout);
        if let Some(ref event) = event
            && handle_codex_event(&mut codex, &mut shell, &mut state, event)?
        {
            continue;
        }
        match event {
            #[cfg(target_os = "linux")]
            Some(ShellEvent::SemanticTarget(request)) => {
                let target = state.resolve_semantic_target(&request.target);
                platform::respond_semantic_target(request, target);
            }
            Some(ShellEvent::GlobalShortcut(shortcut)) => {
                if state.global_shortcut(shortcut) {
                    sync_visibility(&mut shell, &state);
                    focus_visible_overlay(&mut shell, &state);
                    render_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
                    render_role(&mut shell, &mut state, SurfaceRole::ControlCenter)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Lock)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Screenshot)?;
                }
            }
            Some(ShellEvent::Controller(signal)) => {
                if matches!(
                    signal,
                    ControllerSignal::Button {
                        button: ControllerButton::Guide,
                        edge: KeyEdge::Pressed,
                        repeat: false,
                        ..
                    }
                ) {
                    if state.global_shortcut(platform::GlobalShortcut::ShowLauncher) {
                        sync_visibility(&mut shell, &state);
                        focus_visible_overlay(&mut shell, &state);
                        render_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
                    }
                    continue;
                }
                let Some(key) = controller_navigation_key(&signal) else {
                    continue;
                };
                let Some(surface) = shell
                    .surfaces()
                    .find(|surface| surface.window().has_input_focus())
                    .map(|surface| surface.id())
                else {
                    continue;
                };
                let Some(entry) = shell.surface(surface) else {
                    continue;
                };
                let role = entry.role();
                let (width, height) = entry.window().size();
                let changed = match role {
                    SurfaceRole::Lock => state.lock_key(Some(key)),
                    SurfaceRole::ControlCenter => state.control_key(Some(key), width, height),
                    SurfaceRole::WindowPreview => state.preview_key(Some(key)),
                    SurfaceRole::WindowContextMenu => state.window_menu_key(Some(key)),
                    SurfaceRole::Notification => state.notification_key(Some(key)),
                    SurfaceRole::Panel => state.preview_key(Some(key)),
                    SurfaceRole::Launcher => {
                        state.launcher_key(Some(key), &ModifierState::default())
                    }
                    SurfaceRole::Screenshot => state.screenshot_key(Some(key)),
                    _ => false,
                };
                if changed {
                    sync_visibility(&mut shell, &state);
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            Some(ShellEvent::Input { surface, event }) => {
                handle_shell_input(
                    &mut shell,
                    &mut state,
                    &mut codex,
                    surface,
                    event,
                    &mut hover_repaint,
                )?;
            }
            Some(ShellEvent::CloseRequested(surface))
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::Screenshot) =>
            {
                state.hide_overlay(SurfaceRole::Screenshot);
                sync_visibility(&mut shell, &state);
            }
            Some(ShellEvent::Quit) | Some(ShellEvent::CloseRequested(_)) => break,
            // SDL reports an initial focus loss while a newly shown Wayland
            // surface is waiting for the compositor's focus configure. Hiding
            // an overlay here races its first frame and leaves a brief blank
            // window. Explicit dismissal and Escape remain authoritative.
            Some(ShellEvent::FocusChanged {
                surface: _surface,
                focused: false,
            }) => {
                #[cfg(not(target_os = "linux"))]
                if let Some(role @ (SurfaceRole::Launcher | SurfaceRole::ControlCenter)) =
                    shell.surface(_surface).map(|entry| entry.role())
                    && focused_overlays.remove(&_surface)
                {
                    overlay_focus_loss =
                        Some((_surface, role, Instant::now() + Duration::from_millis(100)));
                }
            }
            Some(ShellEvent::FocusChanged {
                surface,
                focused: true,
            }) => {
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::Launcher)
                {
                    shell.start_text_input(surface);
                }
                #[cfg(not(target_os = "linux"))]
                if shell.surface(surface).is_some_and(|entry| {
                    matches!(
                        entry.role(),
                        SurfaceRole::Launcher | SurfaceRole::ControlCenter
                    )
                }) {
                    focused_overlays.insert(surface);
                }
                #[cfg(not(target_os = "linux"))]
                if overlay_focus_loss.is_some_and(|(pending, _, _)| pending == surface) {
                    overlay_focus_loss = None;
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
            Some(ShellEvent::PointerEntered {
                surface,
                entered: true,
            }) if shell
                .surface(surface)
                .is_some_and(|entry| entry.role() == SurfaceRole::Panel) =>
            {
                state.panel_pointer_entered();
            }
            Some(ShellEvent::PointerEntered { surface, entered })
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::WindowPreview) =>
            {
                if state.preview_pointer_entered(entered) {
                    render_role(&mut shell, &mut state, SurfaceRole::WindowPreview)?;
                }
            }
            Some(ShellEvent::PointerEntered { .. }) => {}
            Some(ShellEvent::DisplayTopologyChanged) => {
                shell.sync_display_geometry()?;
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
            }
            Some(
                ShellEvent::LogicalResize { surface, .. } | ShellEvent::PixelResize { surface, .. },
            ) => {
                let Some(role) = shell.surface(surface).map(|entry| entry.role()) else {
                    continue;
                };
                let (logical_width, logical_height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                shell.present(surface, &state.scene(role, logical_width, logical_height))?;
            }
            Some(ShellEvent::Shown(surface)) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if let Some(
                    role @ (SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
                    | SurfaceRole::Lock),
                ) = role
                {
                    state.sync_transient_overlays();
                    // Wayland recreates the wl_surface when SDL shows one of
                    // these transient windows again. The earlier presenter
                    // contents do not belong to that new surface, and the
                    // one-shot initial Exposed handler has already run.
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            Some(ShellEvent::Redraw(surface)) if initial_exposures.insert(surface) => {
                let Some(entry) = shell.surface(surface) else {
                    continue;
                };
                let (logical_width, logical_height) = entry.window().size();
                let role = entry.role();
                shell.present(surface, &state.scene(role, logical_width, logical_height))?;
            }
            Some(ShellEvent::Redraw(_)) => {}
            Some(event) => tracing::debug!(?event, "SDL shell event"),
            None => {}
        }
        if let Some(project_id) = state.take_requested_codex_project() {
            codex.open_project_by_id(&mut shell, &project_id)?;
            sync_visibility(&mut shell, &state);
        }
        if state.screenshot_capture_ready() && state.capture_screenshot() {
            sync_visibility(&mut shell, &state);
            focus_visible_overlay(&mut shell, &state);
            render_role(&mut shell, &mut state, SurfaceRole::Screenshot)?;
        }
        if hover_repaint.is_some_and(|(_, deadline)| Instant::now() >= deadline)
            && let Some((role, _)) = hover_repaint.take()
        {
            render_role(&mut shell, &mut state, role)?;
        }
        #[cfg(not(target_os = "linux"))]
        if overlay_focus_loss.is_some_and(|(_, _, deadline)| Instant::now() >= deadline)
            && let Some((_, role, _)) = overlay_focus_loss.take()
            && state.hide_overlay(role)
        {
            sync_visibility(&mut shell, &state);
        }
        if Instant::now() >= refresh_deadline {
            let mut codex_redraw = Vec::new();
            let project_menu_changed = codex
                .project_menu_host
                .as_mut()
                .is_some_and(|host| host.poll());
            if project_menu_changed {
                codex_redraw.push(codex.project_menu);
                if let Some(host) = codex.project_menu_host.as_mut() {
                    let snapshot = &host.application_mut().state;
                    tracing::info!(
                        status = ?snapshot.status,
                        authenticated = snapshot.account.authenticated,
                        project_count = snapshot.projects.len(),
                        "Codex project discovery changed"
                    );
                }
            }
            if let Some(host) = codex.project_menu_host.as_mut() {
                let snapshot = &host.application_mut().state;
                let codex_available =
                    snapshot.status == ConnectionStatus::Ready && snapshot.account.authenticated;
                let availability_changed = state.set_codex_available(codex_available);
                let projects = match snapshot.status {
                    ConnectionStatus::Loading => DashboardSection::Loading,
                    ConnectionStatus::Ready if snapshot.thread_snapshot_available => {
                        let projects = normalize_dashboard_projects(
                            &snapshot.projects,
                            &snapshot.threads,
                            &snapshot.thread_runtime,
                        );
                        if projects.is_empty() {
                            DashboardSection::Empty
                        } else {
                            DashboardSection::Ready(projects)
                        }
                    }
                    ConnectionStatus::Ready => {
                        let projects = snapshot
                            .projects
                            .iter()
                            .map(|project| DashboardProject {
                                id: project.id.clone(),
                                name: project.name.clone(),
                                roots: project.roots.clone(),
                                chat_count: None,
                                activity: ProjectActivity::Unknown,
                                last_used_at: None,
                            })
                            .collect::<Vec<_>>();
                        if projects.is_empty() {
                            DashboardSection::Empty
                        } else {
                            DashboardSection::Ready(projects)
                        }
                    }
                    ConnectionStatus::Disconnected | ConnectionStatus::Incompatible => {
                        DashboardSection::Unavailable(snapshot.provenance.clone())
                    }
                };
                if state.set_dashboard_projects(projects)
                    && state.surface_visible(SurfaceRole::Launcher)
                {
                    render_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
                }
                if availability_changed {
                    sync_visibility(&mut shell, &state);
                    render_all(&mut shell, &mut state)?;
                }
            }
            for chat in &mut codex.chats {
                if chat.host.poll() {
                    codex_redraw.push(chat.id);
                }
            }
            codex.release_failed_resumes();
            for surface in codex_redraw {
                codex.present(&mut shell, surface)?;
            }
            if codex.open_requests(&mut shell)? {
                state.hide_overlay(SurfaceRole::CodexProjectMenu);
                shell.hide(codex.project_menu);
            }
            if state.refresh_fast() {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
            }
            refresh_deadline = Instant::now() + REFRESH_INTERVAL;
        }
        if Instant::now() >= system_refresh_deadline {
            if state.refresh_system() {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
            }
            system_refresh_deadline = Instant::now() + SYSTEM_REFRESH_INTERVAL;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use nickel_codex::ThreadId;

    use super::{WriterLeases, codex_project_application_id};

    #[test]
    fn codex_project_identity_is_canonical_and_path_opaque() {
        let first = codex_project_application_id(Some("project-one"), Path::new("/private/a"));
        let same = codex_project_application_id(Some("project-one"), Path::new("/private/b"));
        let other = codex_project_application_id(Some("project-two"), Path::new("/private/a"));
        assert_eq!(first, same);
        assert_ne!(first, other);
        assert!(!first.contains("private"));
        assert!(first.starts_with("io.nickel.codex.project."));
    }

    #[test]
    fn codex_project_identity_has_a_normalized_root_fallback() {
        let first = codex_project_application_id(None, Path::new("/work/./sample"));
        let same = codex_project_application_id(None, Path::new("/work/sample"));
        let other = codex_project_application_id(None, Path::new("/other/sample"));
        assert_eq!(first, same);
        assert_ne!(first, other);
    }

    #[test]
    fn writer_lease_allows_exactly_one_owner_and_releases_for_retry() {
        let thread = ThreadId("thread-1".into());
        let mut leases = WriterLeases::default();
        assert!(leases.acquire(&thread));
        assert!(!leases.acquire(&thread));
        assert!(leases.contains(&thread));
        assert!(leases.release(&thread));
        assert!(!leases.contains(&thread));
        assert!(leases.acquire(&thread));
    }
}
