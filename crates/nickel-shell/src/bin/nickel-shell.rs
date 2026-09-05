use nickel_codex::ThreadId;
use nickel_codex_ui::{
    ChatApplication, ConnectionStatus, ShellRequest, shell_application_with_backend,
};
use nickel_core::optional_features::{
    CodexAvailabilityProjection, CodexSource, FeatureEffectiveState, FeatureHealth,
    FeatureInstallation, FeatureSupport, OptionalFeatureRuntime, OptionalFeatureSettings,
};
use nickel_input::{
    AggregateModifier, InputEvent, KeyEdge, LogicalKey, NamedKey, PointerButton, PointerEvent,
};
use nickel_ui::{
    Application, ControllerAction, ControllerInput, HostBatch, HostChangeToken, HostEvent,
    HostEventOutcome, HostFailure, HostFailureStage, UiHost,
};
use std::{
    collections::HashSet,
    path::{Component, Path},
    time::{Duration, Instant},
};

#[path = "../allocation_counter.rs"]
mod allocation_counter;

#[global_allocator]
static GLOBAL_ALLOCATOR: allocation_counter::CountingSystemAllocator =
    allocation_counter::CountingSystemAllocator;

fn is_clipboard_paste(event: &InputEvent) -> bool {
    matches!(event, InputEvent::Key(key)
        if key.edge == KeyEdge::Pressed
        && matches!(&key.logical, LogicalKey::Character(value) if value.eq_ignore_ascii_case("v"))
        && (key.modifiers.aggregate(AggregateModifier::Control)
            || key.modifiers.aggregate(AggregateModifier::Super)))
}

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
#[path = "../control_view.rs"]
mod control_view;
#[path = "../launcher_view.rs"]
mod launcher_view;
#[path = "../live_shell.rs"]
mod live_shell;
#[path = "../model.rs"]
#[allow(dead_code)]
mod model;
#[path = "../notification.rs"]
mod notification;
#[path = "../notification_view.rs"]
mod notification_view;
#[path = "../places.rs"]
#[allow(dead_code)]
mod places;
#[path = "../platform/mod.rs"]
#[allow(dead_code, unused_imports)]
mod platform;
#[path = "../screenshot.rs"]
mod screenshot;
#[path = "../softbuffer_presenter.rs"]
mod softbuffer_presenter;
#[path = "../window_preview.rs"]
mod window_preview;
#[path = "../winit_shell.rs"]
#[allow(dead_code)]
mod winit_shell;

use live_shell::LiveShell;
use winit_shell::{
    PanelEdge, ShellEvent, ShellOptions, ShellUserEvent, SurfaceId, SurfaceRole, WinitShell,
    WinitWindowCompat,
};

const NO_DESKTOP_WINDOWS_FLAG: &str = "--no-desktop-windows";
const PANEL_TOP_FLAG: &str = "--panel-top";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CommandLineOptions {
    no_desktop_windows: bool,
    panel_top: bool,
}

impl CommandLineOptions {
    fn parse(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        for argument in arguments {
            let argument = argument
                .into_string()
                .map_err(|_| "Nickel shell arguments must be valid UTF-8".to_string())?;
            match argument.as_str() {
                NO_DESKTOP_WINDOWS_FLAG => options.no_desktop_windows = true,
                PANEL_TOP_FLAG => options.panel_top = true,
                _ => {
                    return Err(format!(
                        "unknown Nickel shell argument {argument:?}; supported acceptance flags: \
                         {NO_DESKTOP_WINDOWS_FLAG}, {PANEL_TOP_FLAG}"
                    ));
                }
            }
        }
        Ok(options)
    }

    fn shell_options(self) -> ShellOptions {
        ShellOptions {
            create_desktop_surfaces: !self.no_desktop_windows,
            panel_edge: if self.panel_top {
                PanelEdge::Top
            } else {
                PanelEdge::Bottom
            },
            bar_on_all_displays: true,
        }
    }
}

#[cfg(test)]
mod command_line_tests {
    use super::{CommandLineOptions, NO_DESKTOP_WINDOWS_FLAG, PANEL_TOP_FLAG};
    use crate::winit_shell::{PanelEdge, ShellOptions};
    use std::ffi::OsString;

    fn parse(arguments: &[&str]) -> Result<CommandLineOptions, String> {
        CommandLineOptions::parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn production_defaults_create_desktops_and_place_panel_at_bottom() {
        assert_eq!(parse(&[]).unwrap().shell_options(), ShellOptions::default());
    }

    #[test]
    fn acceptance_flags_are_independent_and_composable() {
        assert_eq!(
            parse(&[NO_DESKTOP_WINDOWS_FLAG]).unwrap().shell_options(),
            ShellOptions {
                create_desktop_surfaces: false,
                panel_edge: PanelEdge::Bottom,
                bar_on_all_displays: true,
            }
        );
        assert_eq!(
            parse(&[PANEL_TOP_FLAG]).unwrap().shell_options(),
            ShellOptions {
                create_desktop_surfaces: true,
                panel_edge: PanelEdge::Top,
                bar_on_all_displays: true,
            }
        );
        assert_eq!(
            parse(&[NO_DESKTOP_WINDOWS_FLAG, PANEL_TOP_FLAG])
                .unwrap()
                .shell_options(),
            ShellOptions {
                create_desktop_surfaces: false,
                panel_edge: PanelEdge::Top,
                bar_on_all_displays: true,
            }
        );
    }

    #[test]
    fn unknown_arguments_fail_closed() {
        let error = parse(&["--desktop-ish"]).unwrap_err();
        assert!(error.contains("unknown Nickel shell argument"));
        assert!(error.contains(NO_DESKTOP_WINDOWS_FLAG));
        assert!(error.contains(PANEL_TOP_FLAG));
    }
}

#[cfg(any(target_os = "linux", test))]
fn p95_u64(samples: &[u64]) -> Option<u64> {
    let mut samples = samples.to_vec();
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let index = ((samples.len() * 95).div_ceil(100)).saturating_sub(1);
    samples.get(index).copied()
}

fn codex_availability(
    status: ConnectionStatus,
    authenticated: bool,
) -> Option<(FeatureInstallation, FeatureHealth)> {
    match status {
        ConnectionStatus::Loading => None,
        ConnectionStatus::Unavailable => {
            Some((FeatureInstallation::Missing, FeatureHealth::Failed))
        }
        ConnectionStatus::Incompatible => {
            Some((FeatureInstallation::Incompatible, FeatureHealth::Failed))
        }
        ConnectionStatus::Disconnected => {
            Some((FeatureInstallation::Installed, FeatureHealth::Failed))
        }
        ConnectionStatus::Ready if authenticated => {
            Some((FeatureInstallation::Installed, FeatureHealth::Ready))
        }
        ConnectionStatus::Ready => Some((FeatureInstallation::Installed, FeatureHealth::SignedOut)),
    }
}

fn codex_projection(
    settings: &OptionalFeatureSettings,
    known_installation: FeatureInstallation,
    status: ConnectionStatus,
    authenticated: bool,
    reason: Option<String>,
) -> CodexAvailabilityProjection {
    let (installation, health) = codex_availability(status, authenticated)
        .unwrap_or((known_installation, FeatureHealth::Loading));
    CodexAvailabilityProjection::new(
        FeatureSupport::Supported,
        installation,
        settings.codex_enabled,
        health,
        settings.codex_generation,
        reason,
    )
}

#[cfg(test)]
mod allocation_summary_tests {
    use nickel_codex_ui::ConnectionStatus;

    use nickel_core::optional_features::{FeatureHealth, FeatureInstallation};

    #[test]
    fn allocation_p95_requires_samples_and_uses_nearest_rank() {
        assert_eq!(super::p95_u64(&[]), None);
        assert_eq!(super::p95_u64(&[9, 0, 1, 2, 3]), Some(9));
        assert_eq!(super::p95_u64(&[0; 64]), Some(0));
    }

    #[test]
    fn codex_backend_state_projects_truthful_shell_availability() {
        assert_eq!(
            super::codex_availability(ConnectionStatus::Loading, false),
            None
        );
        assert_eq!(
            super::codex_availability(ConnectionStatus::Unavailable, false),
            Some((FeatureInstallation::Missing, FeatureHealth::Failed))
        );
        assert_eq!(
            super::codex_availability(ConnectionStatus::Incompatible, false),
            Some((FeatureInstallation::Incompatible, FeatureHealth::Failed))
        );
        assert_eq!(
            super::codex_availability(ConnectionStatus::Ready, false),
            Some((FeatureInstallation::Installed, FeatureHealth::SignedOut))
        );
        assert_eq!(
            super::codex_availability(ConnectionStatus::Ready, true),
            Some((FeatureInstallation::Installed, FeatureHealth::Ready))
        );
        assert_eq!(
            super::codex_availability(ConnectionStatus::Disconnected, true),
            Some((FeatureInstallation::Installed, FeatureHealth::Failed))
        );
    }

    #[test]
    fn preference_and_backend_state_remain_separate_in_shell_projection() {
        let mut settings = nickel_core::optional_features::OptionalFeatureSettings {
            codex_enabled: false,
            codex_generation: 44,
            ..Default::default()
        };
        let disabled = super::codex_projection(
            &settings,
            FeatureInstallation::Missing,
            ConnectionStatus::Ready,
            true,
            None,
        );
        assert_eq!(disabled.installation, FeatureInstallation::Installed);
        assert_eq!(
            disabled.presentation(),
            nickel_core::optional_features::CodexPresentation::Hidden
        );
        settings.codex_enabled = true;
        let signed_out = super::codex_projection(
            &settings,
            FeatureInstallation::Installed,
            ConnectionStatus::Ready,
            false,
            Some("Sign in".into()),
        );
        assert_eq!(signed_out.generation, 44);
        assert_eq!(signed_out.health, FeatureHealth::SignedOut);
        assert_eq!(
            signed_out.presentation(),
            nickel_core::optional_features::CodexPresentation::Recoverable
        );
        let loading = super::codex_projection(
            &settings,
            FeatureInstallation::Installed,
            ConnectionStatus::Loading,
            false,
            None,
        );
        assert_eq!(loading.installation, FeatureInstallation::Installed);
        assert_eq!(loading.health, FeatureHealth::Loading);
        assert_eq!(
            loading.presentation(),
            nickel_core::optional_features::CodexPresentation::Recoverable
        );
    }
}

#[cfg(target_os = "linux")]
const SHELL_STARTUP_BARRIER_ENV: &str = "NICKEL_SHELL_STARTUP_BARRIER";
#[cfg(target_os = "linux")]
const SHELL_STARTUP_BARRIER_MAGIC: &[u8; 8] = b"NIKREADY";

struct CodexSurfaces {
    enabled: bool,
    source: CodexSource,
    project_menu: SurfaceId,
    project_menu_cwd: std::path::PathBuf,
    project_menu_host: Option<EmbeddedUiSurface<ChatApplication>>,
    chats: Vec<CodexChatSurface>,
    writer_leases: WriterLeases,
    installation: FeatureInstallation,
    theme: nickel_ui::SemanticTheme,
}

struct CodexChatSurface {
    id: SurfaceId,
    project_id: String,
    host: EmbeddedUiSurface<ChatApplication>,
    thread_id: Option<ThreadId>,
}

struct CodexRuntimeInput {
    enabled: bool,
    generation: u64,
    status: Option<ConnectionStatus>,
    authenticated: bool,
    active_windows: u32,
    cache_entries: u32,
    source_label: String,
    diagnostic: Option<String>,
    installation: FeatureInstallation,
}

fn codex_runtime_from(input: CodexRuntimeInput) -> OptionalFeatureRuntime {
    if !input.enabled {
        return OptionalFeatureRuntime {
            codex_generation: input.generation,
            codex_support: FeatureSupport::Supported,
            codex_installation: input.installation,
            source_label: input.source_label,
            ..Default::default()
        };
    }
    let (effective, health) = match input.status {
        None | Some(ConnectionStatus::Loading) => {
            (FeatureEffectiveState::Enabling, FeatureHealth::Loading)
        }
        Some(ConnectionStatus::Ready) if !input.authenticated => {
            (FeatureEffectiveState::Enabled, FeatureHealth::SignedOut)
        }
        Some(ConnectionStatus::Ready) => (FeatureEffectiveState::Enabled, FeatureHealth::Ready),
        Some(
            ConnectionStatus::Unavailable
            | ConnectionStatus::Disconnected
            | ConnectionStatus::Incompatible,
        ) => (FeatureEffectiveState::Rejected, FeatureHealth::Failed),
    };
    let owned = u32::from(input.status.is_some());
    OptionalFeatureRuntime {
        codex_generation: input.generation,
        codex_effective: effective,
        codex_health: health,
        codex_support: FeatureSupport::Supported,
        codex_installation: input.installation,
        active_windows: input.active_windows,
        background_workers: owned,
        subscriptions: owned + input.active_windows,
        warm_surfaces: owned + input.active_windows,
        cache_entries: input.cache_entries,
        source_label: input.source_label,
        diagnostic: input.diagnostic,
        ..Default::default()
    }
}

struct EmbeddedUiSurface<A: Application> {
    host: UiHost<A>,
    change_token: HostChangeToken,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct EmbeddedControllerTransition {
    changed: bool,
    dismiss_surface: bool,
}

fn poll_due_codex_hosts(
    project_menu: Option<&mut EmbeddedUiSurface<ChatApplication>>,
    chats: &mut [CodexChatSurface],
    now: Instant,
) -> (bool, Vec<SurfaceId>) {
    let project_menu_changed = project_menu
        .and_then(|host| host.poll_due(now))
        .is_some_and(|outcome| outcome.changed);
    let changed_chats = chats
        .iter_mut()
        .filter_map(|chat| {
            chat.host
                .poll_due(now)
                .is_some_and(|outcome| outcome.changed)
                .then_some(chat.id)
        })
        .collect();
    (project_menu_changed, changed_chats)
}

impl<A: Application> EmbeddedUiSurface<A> {
    fn new(application: A, width: u32, height: u32, now: Instant) -> Self {
        Self {
            host: UiHost::new_at(application, width, height, now),
            change_token: HostChangeToken::default(),
        }
    }

    fn application_mut(&mut self) -> &mut A {
        self.host.application_mut()
    }

    fn commands(&self) -> &[nickel_ui::backend::PaintCommand] {
        self.host.commands()
    }

    #[cfg(test)]
    fn accessibility_nodes(&self) -> &[nickel_ui::AccessibilityNode] {
        self.host.accessibility_nodes()
    }

    #[cfg(test)]
    fn inspection(&self) -> nickel_ui::HostInspection {
        self.host.inspect()
    }

    fn normalized_input(
        &mut self,
        input: InputEvent,
        clipboard_text: Option<String>,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text,
            }],
            ..HostBatch::default()
        })
    }

    fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        self.host.paste_clipboard_image(width, height, rgba)
    }

    fn window_focus(&mut self, focused: bool) -> HostEventOutcome {
        self.step(HostBatch {
            window_focused: Some(focused),
            ..HostBatch::default()
        })
    }

    fn suspend(&mut self) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Ui(nickel_ui::UiEvent::Suspended)],
            ..HostBatch::default()
        })
    }

    fn step(&mut self, batch: HostBatch) -> HostEventOutcome {
        let outcome = self.host.step(batch);
        self.change_token = outcome.change_token;
        outcome
    }

    fn deadline(&self) -> Option<Instant> {
        self.host.next_deadline()
    }

    fn poll_due(&mut self, now: Instant) -> Option<HostEventOutcome> {
        let deadline = self.host.next_deadline()?;
        if now < deadline {
            return None;
        }
        let outcome = self.step(HostBatch {
            now: Some(now),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        Some(outcome)
    }
}

fn step_embedded_codex_controller(
    host: &mut EmbeddedUiSurface<ChatApplication>,
    project_menu: bool,
    action: ControllerAction,
) -> EmbeddedControllerTransition {
    if project_menu && action == ControllerAction::Cancel {
        return EmbeddedControllerTransition {
            dismiss_surface: true,
            ..EmbeddedControllerTransition::default()
        };
    }
    let outcome = host.step(HostBatch {
        events: vec![HostEvent::Controller(action)],
        ..HostBatch::default()
    });
    EmbeddedControllerTransition {
        changed: outcome.changed,
        dismiss_surface: false,
    }
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
    fn set_theme(&mut self, theme: nickel_ui::SemanticTheme) -> bool {
        if self.theme == theme {
            return false;
        }
        self.theme = theme;
        if let Some(host) = self.project_menu_host.as_mut()
            && host.application_mut().set_theme(theme)
        {
            host.step(HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
        }
        for chat in &mut self.chats {
            if chat.host.application_mut().set_theme(theme) {
                chat.host.step(HostBatch {
                    events: vec![HostEvent::Poll],
                    ..HostBatch::default()
                });
            }
        }
        true
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.project_menu_host
            .as_ref()
            .and_then(EmbeddedUiSurface::deadline)
            .into_iter()
            .chain(self.chats.iter().filter_map(|chat| chat.host.deadline()))
            .min()
    }

    fn poll_due(&mut self, now: Instant) -> (bool, Vec<SurfaceId>) {
        if !self.enabled {
            return (false, Vec::new());
        }
        let (project_menu_changed, mut redraw) =
            poll_due_codex_hosts(self.project_menu_host.as_mut(), &mut self.chats, now);
        if project_menu_changed {
            redraw.insert(0, self.project_menu);
        }
        (project_menu_changed, redraw)
    }

    fn new(
        shell: &WinitShell,
        settings: &OptionalFeatureSettings,
        theme: nickel_ui::SemanticTheme,
    ) -> Result<Self, String> {
        let project_menu = shell
            .surfaces()
            .find(|surface| surface.role() == SurfaceRole::CodexProjectMenu)
            .ok_or_else(|| "Codex project_menu surface is missing".to_owned())?;
        Ok(Self {
            enabled: settings.codex_enabled,
            source: settings.codex_source.clone(),
            project_menu: project_menu.id(),
            project_menu_cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            project_menu_host: None,
            chats: Vec::new(),
            writer_leases: WriterLeases::default(),
            // UI-host construction and a previous successful connection are not
            // installation probes. The canonical selector will classify this
            // source through its first connection snapshot.
            installation: FeatureInstallation::Missing,
            theme,
        })
    }

    fn ensure_project_menu(&mut self, shell: &WinitShell) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.project_menu_host.is_some() {
            return Ok(());
        }
        let (width, height) = shell
            .surface(self.project_menu)
            .map(|surface| surface.window().size())
            .ok_or_else(|| "Codex project_menu surface is missing".to_owned())?;
        let mut application = shell_application_with_backend(
            self.project_menu_cwd.clone(),
            true,
            None,
            None,
            self.backend_choice(),
        )?;
        application.set_theme(self.theme);
        self.project_menu_host = Some(EmbeddedUiSurface::new(
            application,
            width,
            height,
            Instant::now(),
        ));
        Ok(())
    }

    fn set_enabled(&mut self, shell: &mut WinitShell, enabled: bool) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        if !enabled {
            self.project_menu_host = None;
            self.writer_leases.0.clear();
            for chat in self.chats.drain(..) {
                shell.destroy_surface(chat.id);
            }
            shell.hide(self.project_menu);
        }
        true
    }

    fn apply_settings(
        &mut self,
        shell: &mut WinitShell,
        settings: &OptionalFeatureSettings,
    ) -> bool {
        let source_changed = self.source != settings.codex_source;
        let enabled_changed = self.enabled != settings.codex_enabled;
        if !source_changed && !enabled_changed {
            return false;
        }
        if self.enabled {
            self.set_enabled(shell, false);
        }
        self.source = settings.codex_source.clone();
        if source_changed {
            self.installation = FeatureInstallation::Missing;
        }
        if settings.codex_enabled {
            self.set_enabled(shell, true);
        }
        true
    }

    fn backend_choice(&self) -> Option<nickel_codex::BackendChoice> {
        match &self.source {
            CodexSource::CompatibleInstalled => Some(nickel_codex::BackendChoice::Installed),
            CodexSource::Bundled => Some(nickel_codex::BackendChoice::Bundled),
            CodexSource::ApprovedRemote => None,
            CodexSource::Executable(path) => Some(nickel_codex::BackendChoice::Path(path.clone())),
        }
    }

    fn runtime_snapshot(&mut self, generation: u64) -> OptionalFeatureRuntime {
        let (status, authenticated, cache_entries, diagnostic) = self
            .project_menu_host
            .as_mut()
            .map_or((None, false, 0, None), |host| {
                let state = &host.application_mut().state;
                (
                    Some(state.status.clone()),
                    state.account.authenticated,
                    state.projects.len().saturating_add(state.threads.len()) as u32,
                    matches!(
                        state.status,
                        ConnectionStatus::Unavailable
                            | ConnectionStatus::Disconnected
                            | ConnectionStatus::Incompatible
                    )
                    .then(|| state.provenance.clone()),
                )
            });
        if let Some(status) = status.as_ref()
            && let Some((installation, _)) = codex_availability(status.clone(), authenticated)
        {
            self.installation = installation;
        }
        codex_runtime_from(CodexRuntimeInput {
            enabled: self.enabled,
            generation,
            status,
            authenticated,
            active_windows: self.chats.len() as u32,
            cache_entries,
            source_label: format!("{:?}", self.source),
            diagnostic,
            installation: self.installation,
        })
    }

    fn present(&mut self, shell: &mut WinitShell, surface: SurfaceId) -> Result<(), HostFailure> {
        if !self.enabled {
            return Ok(());
        }
        if surface == self.project_menu {
            self.ensure_project_menu(shell)
                .map_err(|detail| HostFailure {
                    surface: format!("{surface:?}"),
                    stage: HostFailureStage::DomainService,
                    optional: true,
                    detail,
                })?;
            shell
                .present(
                    surface,
                    self.project_menu_host
                        .as_ref()
                        .expect("Codex project_menu initialized")
                        .commands(),
                )
                .map_err(|detail| HostFailure {
                    surface: format!("{surface:?}"),
                    stage: HostFailureStage::Presenter,
                    optional: false,
                    detail,
                })?;
        } else if let Some(chat) = self.chats.iter().find(|chat| chat.id == surface) {
            shell
                .present(surface, chat.host.commands())
                .map_err(|detail| HostFailure {
                    surface: format!("{surface:?}"),
                    stage: HostFailureStage::Presenter,
                    optional: false,
                    detail,
                })?;
        }
        Ok(())
    }

    fn host_mut(&mut self, surface: SurfaceId) -> Option<&mut EmbeddedUiSurface<ChatApplication>> {
        if surface == self.project_menu {
            self.project_menu_host.as_mut()
        } else {
            self.chats
                .iter_mut()
                .find(|chat| chat.id == surface)
                .map(|chat| &mut chat.host)
        }
    }

    fn remove(&mut self, shell: &mut WinitShell, surface: SurfaceId) {
        if let Some(index) = self.chats.iter().position(|chat| chat.id == surface) {
            if let Some(thread) = self.chats.remove(index).thread_id {
                self.writer_leases.release(&thread);
            }
            shell.destroy_surface(surface);
        }
    }

    fn open_requests(&mut self, shell: &mut WinitShell) -> Result<bool, String> {
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
        shell: &mut WinitShell,
        cwd: std::path::PathBuf,
        project_id: String,
        name: String,
        initial_thread: Option<ThreadId>,
    ) -> Result<(), String> {
        if !self.enabled {
            return Err("Codex integration is disabled".into());
        }
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
            let mut application = shell_application_with_backend(
                cwd,
                false,
                initial_thread.clone(),
                Some(project_id.clone()),
                self.backend_choice(),
            )?;
            application.set_theme(self.theme);
            let host = EmbeddedUiSurface::new(application, width, height, Instant::now());
            self.chats.push(CodexChatSurface {
                id,
                project_id,
                host,
                thread_id: initial_thread.clone(),
            });
            self.present(shell, id)
                .map_err(|error| format!("{error:?}"))?;
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

    fn open_project_by_id(
        &mut self,
        shell: &mut WinitShell,
        project_id: &str,
    ) -> Result<(), String> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DomainSubscriptionSchedule {
    minimum: Duration,
    maximum: Duration,
    interval: Duration,
    deadline: Instant,
    change_token: u64,
}

impl DomainSubscriptionSchedule {
    fn new(now: Instant, minimum: Duration, maximum: Duration) -> Self {
        Self {
            minimum,
            maximum: maximum.max(minimum),
            interval: minimum,
            deadline: now + minimum,
            change_token: 0,
        }
    }

    fn deadline(self) -> Instant {
        self.deadline
    }

    fn is_due(self, now: Instant) -> bool {
        now >= self.deadline
    }

    fn observed(&mut self, now: Instant, changed: bool) {
        if changed {
            self.change_token = self.change_token.saturating_add(1);
            self.interval = self.minimum;
        } else {
            self.interval = self.interval.saturating_mul(2).min(self.maximum);
        }
        self.deadline = now + self.interval;
    }
}

fn render_all(shell: &mut WinitShell, state: &mut LiveShell) -> Result<(), String> {
    sync_desktop_outputs(shell, state);
    let surfaces = shell
        .surfaces()
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (
                surface.id(),
                surface.role(),
                surface.output_name().to_owned(),
                logical_width,
                logical_height,
            )
        })
        .collect::<Vec<_>>();
    for (id, role, output, logical_width, logical_height) in surfaces {
        if matches!(role, SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat) {
            continue;
        }
        if !state.surface_visible(role) {
            continue;
        }
        if role == SurfaceRole::Panel {
            state.set_panel_output(output);
        } else if role == SurfaceRole::Desktop
            && let Some(display) = shell.surface_display_geometry(id)
        {
            state.set_desktop_output(
                output,
                display.x as f32 / display.scale,
                display.y as f32 / display.scale,
                display.scale,
            );
        }
        let commands = state.scene(role, logical_width, logical_height);
        if let Some(token) = state.scene_change_token(role) {
            shell.present_host_frame(id, token, &commands)?;
        } else {
            shell.present(id, &commands)?;
        }
    }
    Ok(())
}

fn render_role(
    shell: &mut WinitShell,
    state: &mut LiveShell,
    wanted: SurfaceRole,
) -> Result<(), String> {
    if wanted == SurfaceRole::Desktop {
        sync_desktop_outputs(shell, state);
    }
    let surfaces = shell
        .surfaces()
        .filter(|surface| surface.role() == wanted)
        .map(|surface| {
            let (logical_width, logical_height) = surface.window().size();
            (
                surface.id(),
                surface.role(),
                surface.output_name().to_owned(),
                logical_width,
                logical_height,
            )
        })
        .collect::<Vec<_>>();
    for (id, role, output, logical_width, logical_height) in surfaces {
        if !state.surface_visible(role) {
            continue;
        }
        if role == SurfaceRole::Panel {
            state.set_panel_output(output);
        } else if role == SurfaceRole::Desktop
            && let Some(display) = shell.surface_display_geometry(id)
        {
            state.set_desktop_output(
                output,
                display.x as f32 / display.scale,
                display.y as f32 / display.scale,
                display.scale,
            );
        }
        let commands = state.scene(role, logical_width, logical_height);
        if let Some(token) = state.scene_change_token(role) {
            shell.present_host_frame(id, token, &commands)?;
        } else {
            shell.present(id, &commands)?;
        }
    }
    Ok(())
}

fn sync_desktop_outputs(shell: &WinitShell, state: &mut LiveShell) {
    let outputs = shell
        .surfaces()
        .filter(|surface| surface.role() == SurfaceRole::Desktop)
        .filter_map(|surface| {
            let geometry = shell.surface_display_geometry(surface.id())?;
            Some(nickel_file::desktop::DesktopOutput {
                id: surface.output_name().to_owned(),
                work_area: nickel_file::desktop::Rect {
                    x: geometry.x as f32 / geometry.scale,
                    y: geometry.y as f32 / geometry.scale,
                    width: geometry.width as f32 / geometry.scale,
                    height: (geometry.height as f32 / geometry.scale - 56.0).max(1.0),
                },
                scale: geometry.scale,
            })
        })
        .collect();
    state.set_desktop_outputs(outputs);
}

fn prewarm_role(
    shell: &mut WinitShell,
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
        let commands = state.scene(wanted, logical_width, logical_height);
        if let Some(token) = state.scene_change_token(wanted) {
            shell.present_host_frame(id, token, &commands)?;
        } else {
            shell.present(id, &commands)?;
        }
    }
    Ok(())
}

fn sync_visibility(shell: &mut WinitShell, state: &LiveShell) {
    let surfaces = shell
        .surfaces()
        .map(|surface| (surface.id(), surface.role()))
        .collect::<Vec<_>>();
    for (id, role) in surfaces {
        #[cfg(target_os = "linux")]
        if role == SurfaceRole::Launcher {
            continue;
        }
        set_surface_visibility(shell, id, role, state.surface_visible(role));
    }
}

#[cfg(target_os = "linux")]
fn sync_panel_popover_anchor(shell: &WinitShell, state: &LiveShell) {
    let preferred = match shell.panel_edge() {
        PanelEdge::Top => nickel_session_protocol::AnchorSide::Below,
        PanelEdge::Bottom => nickel_session_protocol::AnchorSide::Above,
    };
    let Some((role, anchor)) = state.popover_anchor(preferred) else {
        return;
    };
    if let Err(error) =
        platform::send_shell_command(platform::ShellCommand::ShowAnchoredShellRole { role, anchor })
    {
        tracing::warn!(?role, %error, "failed to place anchored shell popover");
    }
}

#[cfg(not(target_os = "linux"))]
fn sync_panel_popover_anchor(_shell: &WinitShell, _state: &LiveShell) {}

fn set_surface_visibility(shell: &mut WinitShell, id: SurfaceId, role: SurfaceRole, visible: bool) {
    let changed = if visible {
        shell.show(id)
    } else {
        shell.hide(id)
    };
    #[cfg(target_os = "linux")]
    if changed
        && let Some(role) = session_visibility_role(role)
        && let Err(error) =
            platform::send_shell_command(platform::ShellCommand::SetShellRoleVisible {
                role,
                visible,
            })
    {
        tracing::warn!(?role, visible, %error, "failed to reconcile compositor shell visibility");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (changed, role);
}

#[cfg(target_os = "linux")]
fn session_visibility_role(role: SurfaceRole) -> Option<nickel_session_protocol::ShellRole> {
    use nickel_session_protocol::ShellRole;
    match role {
        SurfaceRole::ControlCenter => Some(ShellRole::ControlCenter),
        SurfaceRole::Notification => Some(ShellRole::Notification),
        SurfaceRole::VolumeOsd => Some(ShellRole::VolumeOsd),
        SurfaceRole::WindowPreview => Some(ShellRole::Preview),
        SurfaceRole::WindowContextMenu => Some(ShellRole::ContextMenu),
        SurfaceRole::CodexProjectMenu => Some(ShellRole::ProjectMenu),
        SurfaceRole::Screenshot => Some(ShellRole::Screenshot),
        SurfaceRole::Desktop
        | SurfaceRole::Panel
        | SurfaceRole::Launcher
        | SurfaceRole::Lock
        | SurfaceRole::CodexChat => None,
    }
}

fn focus_visible_overlay(shell: &mut WinitShell, state: &LiveShell) {
    for role in [
        SurfaceRole::Lock,
        SurfaceRole::Screenshot,
        SurfaceRole::Launcher,
        SurfaceRole::ControlCenter,
        SurfaceRole::CodexProjectMenu,
        SurfaceRole::WindowPreview,
    ] {
        #[cfg(target_os = "linux")]
        if role == SurfaceRole::Launcher {
            continue;
        }
        #[cfg(target_os = "windows")]
        if role == SurfaceRole::WindowPreview {
            continue;
        }
        if state.surface_visible(role) {
            shell.raise_role(role);
        }
    }
}

fn handle_codex_event(
    codex: &mut CodexSurfaces,
    shell: &mut WinitShell,
    state: &mut LiveShell,
    event: &ShellEvent,
) -> Result<bool, String> {
    let surface = match event {
        ShellEvent::Input { surface, .. }
        | ShellEvent::FocusChanged { surface, .. }
        | ShellEvent::LogicalResize { surface, .. }
        | ShellEvent::PixelResize { surface, .. }
        | ShellEvent::Shown(surface)
        | ShellEvent::Hidden(surface)
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
    if surface == codex.project_menu {
        codex.ensure_project_menu(shell)?;
    }
    if matches!(event, ShellEvent::FocusChanged { focused: false, .. }) {
        shell.stop_text_input(surface);
    }
    if matches!(event, ShellEvent::CloseRequested(_)) {
        shell.stop_text_input(surface);
        if surface == codex.project_menu {
            state.hide_overlay(SurfaceRole::CodexProjectMenu);
            set_surface_visibility(shell, surface, SurfaceRole::CodexProjectMenu, false);
        } else {
            codex.remove(shell, surface);
        }
        return Ok(true);
    }
    if matches!(event, ShellEvent::Hidden(_)) {
        shell.stop_text_input(surface);
        if let Some(host) = codex.host_mut(surface) {
            host.suspend();
        }
        return Ok(true);
    }
    if matches!(event, ShellEvent::Shown(_)) {
        if surface == codex.project_menu && !state.surface_visible(SurfaceRole::CodexProjectMenu) {
            set_surface_visibility(shell, surface, SurfaceRole::CodexProjectMenu, false);
            return Ok(true);
        }
        codex
            .present(shell, surface)
            .map_err(|error| format!("{error:?}"))?;
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
        set_surface_visibility(shell, surface, SurfaceRole::CodexProjectMenu, false);
        return Ok(true);
    }
    if matches!(
        event,
        ShellEvent::LogicalResize { .. } | ShellEvent::PixelResize { .. }
    ) {
        if surface == codex.project_menu && !state.surface_visible(SurfaceRole::CodexProjectMenu) {
            return Ok(true);
        }
        let (width, height) = shell
            .surface(surface)
            .map(|entry| entry.window().size())
            .unwrap_or((1, 1));
        if let Some(host) = codex.host_mut(surface) {
            host.step(HostBatch {
                surface_size: Some((width, height)),
                ..HostBatch::default()
            });
            codex
                .present(shell, surface)
                .map_err(|error| format!("{error:?}"))?;
        }
        return Ok(true);
    }
    let outcome = match event {
        ShellEvent::Input { event, .. } => {
            let image_pasted = if is_clipboard_paste(event) {
                shell
                    .clipboard_image()
                    .is_some_and(|(width, height, bytes)| {
                        codex
                            .host_mut(surface)
                            .expect("Codex host exists")
                            .paste_clipboard_image(width, height, &bytes)
                    })
            } else {
                false
            };
            if image_pasted {
                HostEventOutcome {
                    changed: true,
                    ..HostEventOutcome::default()
                }
            } else {
                codex
                    .host_mut(surface)
                    .expect("Codex host exists")
                    .normalized_input(event.clone(), shell.clipboard_text())
            }
        }
        ShellEvent::FocusChanged { focused, .. } => codex
            .host_mut(surface)
            .expect("Codex host exists")
            .window_focus(*focused),
        _ => HostEventOutcome::default(),
    };
    if matches!(
        event,
        ShellEvent::Input { .. } | ShellEvent::FocusChanged { .. }
    ) {
        if outcome.text_input_active {
            shell.start_text_input(surface);
        } else {
            shell.stop_text_input(surface);
        }
    }
    for failure in &outcome.failures {
        tracing::warn!(
            ?failure,
            "embedded UI transition reported recoverable failure"
        );
    }
    if let Some(text) = outcome.clipboard_text {
        shell.set_clipboard_text(&text);
    }
    if outcome.changed {
        codex
            .present(shell, surface)
            .map_err(|error| format!("{error:?}"))?;
    }
    if codex.open_requests(shell)? {
        state.hide_overlay(SurfaceRole::CodexProjectMenu);
        set_surface_visibility(
            shell,
            codex.project_menu,
            SurfaceRole::CodexProjectMenu,
            false,
        );
    }
    codex.resume_requests();
    codex.release_failed_resumes();
    Ok(true)
}

fn handle_shell_input(
    shell: &mut WinitShell,
    state: &mut LiveShell,
    codex: &mut CodexSurfaces,
    surface: SurfaceId,
    event: InputEvent,
    hover_repaint: &mut Option<(SurfaceRole, Instant)>,
) -> Result<(), String> {
    let Some(role) = shell.surface(surface).map(|entry| entry.role()) else {
        return Ok(());
    };
    if role == SurfaceRole::Panel
        && let Some(output) = shell
            .surface(surface)
            .map(|entry| entry.output_name().to_owned())
    {
        state.set_panel_output(output);
    }
    if role == SurfaceRole::Desktop {
        let coalesce_motion = matches!(&event, InputEvent::Pointer(PointerEvent::Motion { .. }));
        if let Some(entry) = shell.surface(surface) {
            let output = entry.output_name().to_owned();
            if let Some(display) = shell.surface_display_geometry(surface) {
                state.set_desktop_output(
                    output,
                    display.x as f32 / display.scale,
                    display.y as f32 / display.scale,
                    display.scale,
                );
            }
        }
        if state.desktop_input(event) {
            if coalesce_motion {
                *hover_repaint = Some((
                    SurfaceRole::Desktop,
                    Instant::now() + Duration::from_millis(16),
                ));
            } else {
                *hover_repaint = None;
                render_role(shell, state, SurfaceRole::Desktop)?;
            }
        }
        return Ok(());
    }
    if role == SurfaceRole::Lock {
        let (width, height) = shell
            .surface(surface)
            .map(|entry| entry.window().size())
            .unwrap_or_default();
        if state.lock_host_input(event, width, height) {
            render_role(shell, state, SurfaceRole::Lock)?;
        }
        return Ok(());
    }
    if role == SurfaceRole::Launcher {
        let (width, height) = shell
            .surface(surface)
            .map(|entry| entry.window().size())
            .unwrap_or_default();
        let outcome = state.launcher_host_input(event, shell.clipboard_text(), width, height);
        if let Some(text) = outcome.clipboard_text {
            shell.set_clipboard_text(&text);
        }
        if outcome.changed {
            sync_visibility(shell, state);
            render_role(shell, state, role)?;
        }
        return Ok(());
    }
    if role == SurfaceRole::WindowContextMenu {
        let (width, height) = shell
            .surface(surface)
            .map(|entry| entry.window().size())
            .unwrap_or_default();
        if state.window_menu_host_input(event, width, height) {
            sync_visibility(shell, state);
            render_role(shell, state, role)?;
        }
        return Ok(());
    }
    if role == SurfaceRole::WindowPreview {
        let outcome = state.preview_host_input(event);
        for failure in &outcome.failures {
            eprintln!("window preview host failure: {failure:?}");
        }
        if outcome.changed {
            sync_visibility(shell, state);
            state.sync_transient_overlays();
            render_role(shell, state, SurfaceRole::WindowPreview)?;
            render_role(shell, state, SurfaceRole::WindowContextMenu)?;
        }
        return Ok(());
    }
    match event {
        InputEvent::Text(nickel_input::TextEvent::Commit { .. }) => {
            log_unroutable_launcher_input(role, "text-commit", "non-launcher-surface");
        }
        InputEvent::Text(nickel_input::TextEvent::Preedit { .. }) => {
            log_unroutable_launcher_input(role, "text-preedit", "non-launcher-surface");
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
                SurfaceRole::Lock => false,
                SurfaceRole::ControlCenter => state.control_key(keycode, width, height),
                SurfaceRole::WindowPreview => state.preview_key(keycode),
                SurfaceRole::WindowContextMenu => false,
                SurfaceRole::Notification => state.notification_key(keycode),
                SurfaceRole::Panel => state.preview_key(keycode),
                SurfaceRole::Launcher => false,
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
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::Panel {
                shell.set_active_output_from_surface(surface);
                if let Some(output) = shell
                    .surface(surface)
                    .map(|entry| entry.output_name().to_owned())
                {
                    state.set_panel_output(output);
                }
                if let Some(display) = shell.surface_display_geometry(surface) {
                    state.set_panel_origin_x(display.x);
                }
                let width = shell
                    .surface(surface)
                    .map(|entry| entry.window().size().0)
                    .unwrap_or_default();
                if state.panel_click(x, width, button == PointerButton::Secondary) {
                    sync_panel_popover_anchor(shell, state);
                    sync_visibility(shell, state);
                    state.sync_transient_overlays();
                    focus_visible_overlay(shell, state);
                    render_role(shell, state, SurfaceRole::ControlCenter)?;
                    render_role(shell, state, SurfaceRole::WindowPreview)?;
                    if state.surface_visible(SurfaceRole::CodexProjectMenu) {
                        codex
                            .present(shell, codex.project_menu)
                            .map_err(|error| format!("{error:?}"))?;
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
            } else if edge == KeyEdge::Pressed && role == SurfaceRole::ControlCenter {
                let (width, height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                let changed = match role {
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
                if let Some(output) = shell
                    .surface(surface)
                    .map(|entry| entry.output_name().to_owned())
                {
                    state.set_panel_output(output);
                }
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
            } else if role == SurfaceRole::WindowPreview && state.preview_pointer_moved(x, y) {
                *hover_repaint = Some((
                    SurfaceRole::WindowPreview,
                    Instant::now() + Duration::from_millis(24),
                ));
            }
        }
        InputEvent::Pointer(PointerEvent::Axis { delta, .. }) => {
            if role == SurfaceRole::ControlCenter {
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
        InputEvent::Touch(nickel_input::TouchEvent::Ended { position, .. })
            if role == SurfaceRole::Panel =>
        {
            shell.set_active_output_from_surface(surface);
            let width = shell
                .surface(surface)
                .map(|entry| entry.window().size().0)
                .unwrap_or_default();
            if state.panel_click(position.x as f32, width, false) {
                sync_panel_popover_anchor(shell, state);
                sync_visibility(shell, state);
                focus_visible_overlay(shell, state);
                render_role(shell, state, SurfaceRole::ControlCenter)?;
                if state.surface_visible(SurfaceRole::CodexProjectMenu) {
                    codex
                        .present(shell, codex.project_menu)
                        .map_err(|error| format!("{error:?}"))?;
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

fn log_unroutable_launcher_input(
    role: SurfaceRole,
    event_class: &'static str,
    rejection_reason: &'static str,
) {
    static LAST_DIAGNOSTIC: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let Ok(mut last) = LAST_DIAGNOSTIC.lock() else {
        return;
    };
    let now = Instant::now();
    if last.is_some_and(|previous| now.duration_since(previous) < Duration::from_secs(30)) {
        return;
    }
    *last = Some(now);
    tracing::warn!(
        surface_role = ?role,
        focus_ownership = "event-delivered-to-surface",
        event_class,
        rejection_reason,
        "launcher input could not be routed"
    );
}

fn controller_launcher_shortcut(action: ControllerAction) -> Option<platform::GlobalShortcut> {
    (action == ControllerAction::Launcher).then_some(platform::GlobalShortcut::ToggleLauncher)
}

fn controller_target_role(
    launcher_visible: bool,
    focused_role: Option<SurfaceRole>,
) -> Option<SurfaceRole> {
    launcher_visible
        .then_some(SurfaceRole::Launcher)
        .or(focused_role)
}

fn handle_controller_action(
    shell: &mut WinitShell,
    state: &mut LiveShell,
    codex: &mut CodexSurfaces,
    action: ControllerAction,
    family: nickel_ui::ControllerFamily,
) -> Result<(), String> {
    if controller_launcher_shortcut(action).is_some() {
        state.set_launcher_controller_family(family);
        let changed = state.request_launcher_toggle();
        if changed {
            sync_visibility(shell, state);
            focus_visible_overlay(shell, state);
            render_role(shell, state, SurfaceRole::Launcher)?;
        }
        return Ok(());
    }
    let focused_surface = shell
        .surfaces()
        .find(|surface| surface.window().has_input_focus())
        .map(|surface| surface.id())
        .or_else(|| {
            shell
                .surfaces()
                .find(|surface| surface.role() == SurfaceRole::Desktop)
                .map(|surface| surface.id())
        });
    let focused_role =
        focused_surface.and_then(|surface| shell.surface(surface).map(|entry| entry.role()));
    if controller_target_role(state.surface_visible(SurfaceRole::Launcher), focused_role)
        == Some(SurfaceRole::Launcher)
    {
        if state.launcher_host_controller(action, family) {
            sync_visibility(shell, state);
            render_role(shell, state, SurfaceRole::Launcher)?;
        }
        return Ok(());
    }
    let Some(surface) = focused_surface else {
        return Ok(());
    };
    let Some(entry) = shell.surface(surface) else {
        return Ok(());
    };
    let role = entry.role();
    if role == SurfaceRole::Launcher {
        if state.launcher_host_controller(action, family) {
            sync_visibility(shell, state);
            render_role(shell, state, role)?;
        }
        return Ok(());
    }
    if matches!(role, SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat) {
        let transition = codex
            .host_mut(surface)
            .map(|host| {
                step_embedded_codex_controller(host, role == SurfaceRole::CodexProjectMenu, action)
            })
            .unwrap_or_default();
        if transition.changed {
            codex
                .present(shell, surface)
                .map_err(|error| format!("{error:?}"))?;
        }
        if transition.dismiss_surface {
            state.hide_overlay(SurfaceRole::CodexProjectMenu);
            set_surface_visibility(shell, surface, SurfaceRole::CodexProjectMenu, false);
        }
        return Ok(());
    }
    let (width, height) = entry.window().size();
    let changed = match role {
        SurfaceRole::Lock => state.lock_host_controller(action),
        SurfaceRole::ControlCenter => state.control_controller(action, width, height),
        SurfaceRole::WindowPreview => state.preview_controller(action),
        SurfaceRole::WindowContextMenu => state.window_menu_host_controller(action),
        SurfaceRole::Notification => state.notification_controller(action),
        SurfaceRole::Panel => state.panel_controller(action, width),
        SurfaceRole::Desktop => state.desktop_controller(action),
        SurfaceRole::Launcher => unreachable!("launcher controller input is handled semantically"),
        SurfaceRole::Screenshot => state.screenshot_controller(action),
        _ => false,
    };
    if changed {
        if role == SurfaceRole::Panel {
            sync_panel_popover_anchor(shell, state);
        }
        sync_visibility(shell, state);
        render_role(shell, state, role)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_supervisor_token(token: &[u8], pid: u32) -> Result<(), String> {
    if token.len() != SHELL_STARTUP_BARRIER_MAGIC.len() + 4
        || &token[..SHELL_STARTUP_BARRIER_MAGIC.len()] != SHELL_STARTUP_BARRIER_MAGIC
    {
        return Err("Nickel shell startup barrier token is invalid".to_owned());
    }
    let expected_pid = u32::from_ne_bytes(
        token[SHELL_STARTUP_BARRIER_MAGIC.len()..]
            .try_into()
            .expect("startup barrier token has a fixed PID field"),
    );
    if expected_pid != pid {
        return Err(format!(
            "Nickel shell startup barrier belongs to PID {expected_pid}, not this shell"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn wait_for_supervisor_readiness() -> Result<(), String> {
    use std::{io::Read, os::unix::net::UnixStream};

    let path = std::env::var_os(SHELL_STARTUP_BARRIER_ENV)
        .ok_or_else(|| "Nickel shell startup barrier is missing".to_owned())?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("could not connect to Nickel shell startup barrier: {error}"))?;
    let mut token = [0_u8; SHELL_STARTUP_BARRIER_MAGIC.len() + 4];
    stream
        .read_exact(&mut token)
        .map_err(|error| format!("Nickel shell startup barrier was not released: {error}"))?;
    validate_supervisor_token(&token, std::process::id())
}

#[cfg(target_os = "linux")]
fn validate_shell_readiness(
    readiness: &nickel_session_protocol::ShellReadinessSnapshot,
) -> Result<(), String> {
    let counts = format!(
        "outputs={} desktops={} panels={} locks={} launchers={} singletons_ready={} output_roles_ready={} reserved_ordinary_windows={}",
        readiness.outputs,
        readiness.desktops,
        readiness.panels,
        readiness.locks,
        readiness.launchers,
        readiness.required_singletons_ready,
        readiness.output_roles_ready,
        readiness.reserved_ordinary_windows,
    );
    let expected_panels =
        if nickel_core::shell_settings::ShellSettings::load_default().bar_on_all_displays {
            readiness.outputs
        } else {
            1
        };
    let roles_are_complete = readiness.outputs > 0
        && readiness.desktops == readiness.outputs
        && readiness.panels == expected_panels
        && readiness.locks == readiness.outputs
        && readiness.launchers == 1
        && readiness.required_singletons_ready
        && readiness.output_roles_ready
        && readiness.reserved_ordinary_windows == 0;
    let pids_are_authenticated = readiness.expected_shell_pid.is_some()
        && readiness.expected_shell_pid == readiness.authenticated_shell_pid;
    if readiness.ready && roles_are_complete && pids_are_authenticated {
        return Ok(());
    }
    Err(format!(
        "Nickel shell session is not ready ({counts}; expected_shell_pid={:?}; authenticated_shell_pid={:?})",
        readiness.expected_shell_pid, readiness.authenticated_shell_pid,
    ))
}

#[cfg(target_os = "linux")]
fn wait_for_shell_readiness_with<F>(
    mut query: F,
    timeout: Duration,
    retry_interval: Duration,
) -> Result<nickel_session_protocol::ShellReadinessSnapshot, String>
where
    F: FnMut() -> Result<nickel_session_protocol::ShellReadinessSnapshot, String>,
{
    let started = Instant::now();
    loop {
        let last_failure = match query() {
            Ok(readiness) => match validate_shell_readiness(&readiness) {
                Ok(()) => return Ok(readiness),
                Err(error) => error,
            },
            Err(error) => error,
        };
        if started.elapsed() >= timeout {
            return Err(format!(
                "Nickel shell readiness did not converge within {} ms: {last_failure}",
                timeout.as_millis()
            ));
        }
        std::thread::sleep(retry_interval.min(timeout.saturating_sub(started.elapsed())));
    }
}

#[cfg(target_os = "linux")]
fn wait_for_shell_readiness() -> Result<(), String> {
    wait_for_shell_readiness_with(
        || {
            platform::shell_readiness().map_err(|error| {
                format!("Nickel shell could not verify session readiness: {error}")
            })
        },
        Duration::from_secs(2),
        Duration::from_millis(25),
    )
    .map(|_| ())
}

fn wait_for_initial_display_with(
    mut wait_step: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    while !wait_step()? {}
    Ok(())
}

fn wait_for_initial_display(shell: &mut WinitShell) -> Result<(), String> {
    let mut logged = false;
    wait_for_initial_display_with(|| {
        if !shell.display_geometries()?.is_empty() {
            return Ok(true);
        }
        if !logged {
            tracing::info!("winit shell is waiting for a display instead of restarting");
            logged = true;
        }
        let _ = shell.wait_event_timeout(Duration::from_secs(1));
        Ok(false)
    })
}

fn shell_event_ends_process(event: &ShellEvent) -> bool {
    matches!(event, ShellEvent::Quit | ShellEvent::CloseRequested(_)) && !cfg!(target_os = "linux")
}

fn main() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    platform::prepare_audio_environment();
    let command_line = CommandLineOptions::parse(std::env::args_os().skip(1))?;
    nickel_logging::init("nickel-shell").map_err(|error| error.to_string())?;
    // The supervisor publishes its expected child PID and releases this
    // barrier before the shell attempts registration. Creating any Wayland
    // surfaces before authentication is unsafe: Linux shell surfaces are
    // initially visible so winit can obtain their first configure, and an
    // unauthenticated surface is classified as an ordinary movable window.
    #[cfg(target_os = "linux")]
    wait_for_supervisor_readiness()?;
    #[cfg(target_os = "linux")]
    platform::register_session_shell().map_err(|error| {
        format!("Nickel shell could not authenticate with the session protocol: {error}")
    })?;
    #[cfg(not(target_os = "linux"))]
    platform::register_session_shell().map_err(|error| {
        format!("Nickel shell could not authenticate with the session protocol: {error}")
    })?;
    let started = Instant::now();
    let mut shell_options = command_line.shell_options();
    shell_options.bar_on_all_displays =
        nickel_core::shell_settings::ShellSettings::load_default().bar_on_all_displays;
    let mut shell = WinitShell::new_with_options(started, shell_options)?;
    wait_for_initial_display(&mut shell)?;
    shell.set_primary_output_name(platform::configured_primary_output())?;
    shell.create_shell_surfaces()?;
    #[cfg(target_os = "linux")]
    wait_for_shell_readiness()?;
    let mut state = LiveShell::new()?;
    let mut feature_settings = OptionalFeatureSettings::load_default();
    feature_settings.codex_enabled = feature_settings.effective_codex_enabled();
    let mut codex = CodexSurfaces::new(&shell, &feature_settings, state.semantic_theme())?;
    state.apply_codex_projection(CodexAvailabilityProjection::new(
        FeatureSupport::Supported,
        codex.installation,
        feature_settings.codex_enabled,
        FeatureHealth::Loading,
        feature_settings.codex_generation,
        Some("Checking the selected Codex backend…".into()),
    ));
    codex.ensure_project_menu(&shell)?;
    let _ = codex
        .runtime_snapshot(feature_settings.codex_generation)
        .save_default();
    let hotkey_feed = platform::launcher_hotkey_receiver();
    state.set_global_shortcut_capability(&hotkey_feed.capability);
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
                tracing::debug!(?shortcut, "forwarding global shortcut to winit shell");
                if event_sender
                    .send_event(ShellUserEvent::GlobalShortcut(shortcut))
                    .is_err()
                {
                    tracing::warn!("winit shell stopped accepting global shortcuts");
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
                    if event_sender
                        .send_event(ShellUserEvent::TestControl(request))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
    }
    sync_visibility(&mut shell, &state);
    render_all(&mut shell, &mut state)?;
    let memory = shell.memory_diagnostics();
    let presenter_roles = shell.presenter_roles();
    let images = state.image_cache_diagnostics();
    tracing::info!(
        ?presenter_roles,
        presenters = memory.presenter_caches.presenters,
        cache_live_entries = memory.presenter_caches.live_entries,
        cache_live_bytes = memory.presenter_caches.live_bytes,
        cache_peak_bytes = memory.presenter_caches.peak_cache_bytes,
        process_rss_bytes = memory.process_rss_bytes,
        launcher_icon_entries = images.launcher_icon_entries,
        launcher_icon_bytes = images.launcher_icon_bytes,
        wallpaper_entries = images.wallpaper_entries,
        wallpaper_bytes = images.wallpaper_bytes,
        tray_entries = images.tray_entries,
        tray_bytes = images.tray_bytes,
        preview_entries = images.preview_entries,
        preview_bytes = images.preview_bytes,
        "shell presenter cache and allocator-visible memory accounting"
    );
    println!(
        "time_to_first_shell_ms={:.3}",
        started.elapsed().as_secs_f64() * 1_000.0
    );

    tracing::info!(
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "winit Nickel shell presented"
    );
    let launcher_warm_started = Instant::now();
    prewarm_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
    tracing::info!(
        elapsed_ms = launcher_warm_started.elapsed().as_secs_f64() * 1_000.0,
        "winit launcher presenter and frame prewarmed"
    );
    if let Err(error) = codex.ensure_project_menu(&shell) {
        tracing::warn!(%error, "Codex integration is unavailable");
    }
    let schedule_now = Instant::now();
    let mut fast_subscription = DomainSubscriptionSchedule::new(
        schedule_now,
        Duration::from_millis(100),
        Duration::from_secs(2),
    );
    let mut system_subscription = DomainSubscriptionSchedule::new(
        schedule_now,
        Duration::from_secs(1),
        Duration::from_secs(10),
    );
    let mut hover_repaint: Option<(SurfaceRole, Instant)> = None;
    let mut controller = ControllerInput::new();
    let mut controller_schedule = nickel_ui::ControllerPollSchedule::new(Instant::now());
    let mut diagnostic_loop_started = Instant::now();
    let mut diagnostic_loop_iterations = 0_u64;
    let mut diagnostic_overdue_after_poll = Vec::new();
    let mut project_menu_changed_since_refresh = false;
    loop {
        diagnostic_loop_iterations = diagnostic_loop_iterations.saturating_add(1);
        let now = Instant::now();
        if controller_schedule.is_due(now) {
            for action in controller.poll_global(now) {
                shell.begin_input_observation(Instant::now());
                let result = handle_controller_action(
                    &mut shell,
                    &mut state,
                    &mut codex,
                    action,
                    controller.active_family().unwrap_or_default(),
                );
                shell.finish_input_observation();
                result?;
            }
            controller_schedule.mark_polled(now, controller.connected());
        }
        let (project_menu_changed, due_codex_redraw) = codex.poll_due(Instant::now());
        project_menu_changed_since_refresh |= project_menu_changed;
        for surface in due_codex_redraw {
            if surface == codex.project_menu
                && !state.surface_visible(SurfaceRole::CodexProjectMenu)
            {
                continue;
            }
            codex
                .present(&mut shell, surface)
                .map_err(|error| format!("{error:?}"))?;
        }
        let next_deadline = fast_subscription
            .deadline()
            .min(system_subscription.deadline())
            .min(controller_schedule.deadline());
        let next_deadline = codex
            .next_deadline()
            .map(|deadline| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        let next_deadline = state
            .next_host_deadline()
            .map(|deadline| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        let next_deadline = hover_repaint
            .map(|(_, deadline)| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        let next_deadline = shell
            .next_output_retirement_deadline()
            .map(|deadline| deadline.min(next_deadline))
            .unwrap_or(next_deadline);
        let timeout = next_deadline.saturating_duration_since(Instant::now());
        let event = shell.wait_event_timeout(timeout);
        if diagnostic_loop_started.elapsed() >= Duration::from_secs(1) {
            if diagnostic_loop_iterations >= 1 {
                tracing::info!(
                    iterations = diagnostic_loop_iterations,
                    ?timeout,
                    ?event,
                    fast_due = fast_subscription.is_due(Instant::now()),
                    system_due = system_subscription.is_due(Instant::now()),
                    host_deadline = ?state.next_host_deadline(),
                    host_deadline_sources = ?state.host_deadline_sources(),
                    overdue_after_previous_poll = ?diagnostic_overdue_after_poll,
                    "diagnostic: winit shell event loop is busy"
                );
            }
            diagnostic_loop_started = Instant::now();
            diagnostic_loop_iterations = 0;
        }
        if let Some(ref event) = event
            && handle_codex_event(&mut codex, &mut shell, &mut state, event)?
        {
            continue;
        }
        match event {
            #[cfg(target_os = "linux")]
            Some(ShellEvent::TestControl(request)) => match request {
                platform::ShellTestRequest::SemanticTarget {
                    request_id,
                    target,
                    reply_path,
                } => {
                    if let nickel_session_protocol::ShellSemanticTarget::Screenshot { action } =
                        target
                    {
                        let performed = state.perform_screenshot_semantic_action(action);
                        platform::respond_semantic_action(request_id, &reply_path, performed);
                        continue;
                    }
                    let target = state.resolve_semantic_target(&target);
                    platform::respond_semantic_target(request_id, &reply_path, target);
                }
                platform::ShellTestRequest::RuntimeDiagnostics {
                    request_id,
                    reply_path,
                } => {
                    let runtime = shell.runtime_diagnostics();
                    let memory = shell.memory_diagnostics();
                    let (
                        input_to_message_us,
                        input_to_frame_us,
                        layout_us,
                        paint_list_us,
                        scheduled_wakeups,
                    ) = state.host_runtime_samples();
                    let host_phase_samples_available = !input_to_frame_us.is_empty();
                    platform::respond_runtime_diagnostics(
                        request_id,
                        &reply_path,
                        nickel_session_protocol::ShellRuntimeDiagnostics {
                            input_to_message_us,
                            input_to_frame_us,
                            layout_us,
                            paint_list_us,
                            warm_present_us: runtime.warm_present_us,
                            input_to_visible_us: runtime.input_to_present_us,
                            scheduled_wakeups,
                            host_phase_samples_available,
                            retained_presenter_bytes: memory.presenter_caches.live_bytes as u64,
                            frame_allocations: if runtime.warm_present_allocations.is_empty() {
                                nickel_session_protocol::AllocationMeasurement {
                                    count: None,
                                    sample_count: 0,
                                    scope: nickel_session_protocol::AllocationScope::Process,
                                    unavailable_reason: Some(
                                        "no completed warm native presenter frames".into(),
                                    ),
                                }
                            } else {
                                nickel_session_protocol::AllocationMeasurement {
                                    count: p95_u64(&runtime.warm_present_allocations),
                                    sample_count: runtime.warm_present_allocations.len(),
                                    scope: nickel_session_protocol::AllocationScope::Process,
                                    unavailable_reason: None,
                                }
                            },
                        },
                    );
                }
            },
            Some(ShellEvent::GlobalShortcut(shortcut)) => {
                tracing::debug!(?shortcut, "handling global shortcut");
                shell.begin_input_observation(Instant::now());
                #[cfg(not(target_os = "linux"))]
                if matches!(
                    shortcut,
                    platform::GlobalShortcut::ToggleLauncher
                        | platform::GlobalShortcut::ShowLauncher
                ) && let Some(point) = platform::active_display_point()
                {
                    shell.set_active_output_at(point);
                }
                if shortcut == platform::GlobalShortcut::ReloadShellSettings {
                    let settings = nickel_core::shell_settings::ShellSettings::load_default();
                    if shell.set_bar_on_all_displays(settings.bar_on_all_displays)? {
                        sync_visibility(&mut shell, &state);
                    }
                }
                if state.global_shortcut(shortcut) {
                    sync_visibility(&mut shell, &state);
                    state.sync_transient_overlays();
                    focus_visible_overlay(&mut shell, &state);
                    render_role(&mut shell, &mut state, SurfaceRole::Desktop)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Panel)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Launcher)?;
                    render_role(&mut shell, &mut state, SurfaceRole::ControlCenter)?;
                    render_role(&mut shell, &mut state, SurfaceRole::VolumeOsd)?;
                    render_role(&mut shell, &mut state, SurfaceRole::WindowPreview)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Lock)?;
                    render_role(&mut shell, &mut state, SurfaceRole::Screenshot)?;
                }
                shell.finish_input_observation();
            }
            Some(ShellEvent::Input { surface, event }) => {
                shell.begin_input_observation(Instant::now());
                let result = handle_shell_input(
                    &mut shell,
                    &mut state,
                    &mut codex,
                    surface,
                    event,
                    &mut hover_repaint,
                );
                shell.finish_input_observation();
                result?;
            }
            Some(ShellEvent::FileDrop { surface, path }) => {
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::Desktop)
                    && state.desktop_file_drop(&path)
                {
                    render_role(&mut shell, &mut state, SurfaceRole::Desktop)?;
                }
            }
            Some(ShellEvent::CloseRequested(surface))
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::Screenshot) =>
            {
                state.hide_overlay(SurfaceRole::Screenshot);
                sync_visibility(&mut shell, &state);
            }
            Some(ShellEvent::RuntimeTerminated { code }) => {
                tracing::info!(code, "native shell event source terminated; exiting");
                break;
            }
            Some(event @ (ShellEvent::Quit | ShellEvent::CloseRequested(_)))
                if shell_event_ends_process(&event) =>
            {
                break;
            }
            Some(ShellEvent::Quit | ShellEvent::CloseRequested(_)) => {
                shell.sync_display_geometry()?;
                sync_visibility(&mut shell, &state);
            }
            // Winit reports an initial focus loss while a newly shown Wayland
            // surface is waiting for the compositor's focus configure. Hiding
            // an overlay here races its first frame and leaves a brief blank
            // window. Explicit dismissal and Escape remain authoritative.
            Some(ShellEvent::FocusChanged { focused: false, .. }) => {}
            Some(ShellEvent::FocusChanged {
                surface,
                focused: true,
            }) => {
                shell.set_active_output_from_surface(surface);
                if shell
                    .surface(surface)
                    .is_some_and(|entry| entry.role() == SurfaceRole::Launcher)
                {
                    state.focus_launcher_search();
                    shell.start_text_input(surface);
                }
            }
            Some(ShellEvent::PointerEntered {
                surface,
                entered: false,
            }) if shell
                .surface(surface)
                .is_some_and(|entry| entry.role() == SurfaceRole::Panel) =>
            {
                if let Some(output) = shell
                    .surface(surface)
                    .map(|entry| entry.output_name().to_owned())
                {
                    state.set_panel_output(output);
                }
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
                if !state.surface_visible(role) {
                    continue;
                }
                let (logical_width, logical_height) = shell
                    .surface(surface)
                    .map(|entry| entry.window().size())
                    .unwrap_or_default();
                shell.present(surface, &state.scene(role, logical_width, logical_height))?;
            }
            Some(ShellEvent::Shown(surface)) => {
                let role = shell.surface(surface).map(|entry| entry.role());
                if let Some(
                    role @ (SurfaceRole::Desktop
                    | SurfaceRole::Panel
                    | SurfaceRole::WindowPreview
                    | SurfaceRole::WindowContextMenu
                    | SurfaceRole::Lock),
                ) = role
                {
                    state.sync_transient_overlays();
                    // Wayland recreates the wl_surface when winit shows one of
                    // these transient windows again. The earlier presenter
                    // contents do not belong to that new surface, and the
                    // one-shot initial Exposed handler has already run.
                    render_role(&mut shell, &mut state, role)?;
                }
            }
            Some(ShellEvent::Redraw(surface)) if shell.mark_initial_exposed(surface) => {
                let Some(entry) = shell.surface(surface) else {
                    continue;
                };
                let (logical_width, logical_height) = entry.window().size();
                let role = entry.role();
                if state.surface_visible(role) {
                    shell.present(surface, &state.scene(role, logical_width, logical_height))?;
                }
            }
            Some(ShellEvent::Redraw(_)) => {}
            Some(event) => tracing::debug!(?event, "winit shell event"),
            None => {}
        }
        if shell
            .next_output_retirement_deadline()
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            shell.sync_display_geometry()?;
            sync_visibility(&mut shell, &state);
            render_all(&mut shell, &mut state)?;
        }
        if let Some(project_id) = state.take_requested_codex_project() {
            codex.open_project_by_id(&mut shell, &project_id)?;
            sync_visibility(&mut shell, &state);
        }
        let deadline_outcome = state.poll_deadlines(Instant::now());
        diagnostic_overdue_after_poll = state
            .host_deadline_sources()
            .into_iter()
            .filter(|(_, deadline)| *deadline <= Instant::now())
            .collect::<Vec<_>>();
        if deadline_outcome.visibility_changed {
            sync_visibility(&mut shell, &state);
        }
        if deadline_outcome.capture_screenshot && state.capture_screenshot() {
            sync_visibility(&mut shell, &state);
            focus_visible_overlay(&mut shell, &state);
            render_role(&mut shell, &mut state, SurfaceRole::Screenshot)?;
        }
        for role in deadline_outcome.redraw {
            if state.surface_visible(role) {
                render_role(&mut shell, &mut state, role)?;
            }
        }
        if hover_repaint.is_some_and(|(_, deadline)| Instant::now() >= deadline)
            && let Some((role, _)) = hover_repaint.take()
        {
            render_role(&mut shell, &mut state, role)?;
        }
        if fast_subscription.is_due(Instant::now()) {
            let refresh_now = Instant::now();
            let mut codex_redraw = Vec::new();
            let project_menu_changed = std::mem::take(&mut project_menu_changed_since_refresh);
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
                // The panel entry represents the installed integration, not only a healthy,
                // authenticated connection. Keep it reachable so its UI can explain and recover
                // from disconnected, incompatible, or signed-out states.
                let availability_changed = state.apply_codex_projection(codex_projection(
                    &feature_settings,
                    codex.installation,
                    snapshot.status.clone(),
                    snapshot.account.authenticated,
                    (!snapshot.provenance.is_empty()).then(|| snapshot.provenance.clone()),
                ));
                let projects = match snapshot.status {
                    ConnectionStatus::Loading => DashboardSection::Loading,
                    ConnectionStatus::Ready if !snapshot.account.authenticated => {
                        DashboardSection::Failed {
                            message: "Sign in to Codex to load projects".into(),
                            recoverable: true,
                        }
                    }
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
                    ConnectionStatus::Unavailable
                    | ConnectionStatus::Disconnected
                    | ConnectionStatus::Incompatible => {
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
            codex.release_failed_resumes();
            let codex_changed = !codex_redraw.is_empty();
            for surface in codex_redraw {
                if surface == codex.project_menu
                    && !state.surface_visible(SurfaceRole::CodexProjectMenu)
                {
                    continue;
                }
                codex
                    .present(&mut shell, surface)
                    .map_err(|error| format!("{error:?}"))?;
            }
            let opened_codex = codex.open_requests(&mut shell)?;
            if opened_codex {
                state.hide_overlay(SurfaceRole::CodexProjectMenu);
                set_surface_visibility(
                    &mut shell,
                    codex.project_menu,
                    SurfaceRole::CodexProjectMenu,
                    false,
                );
            }
            let fast_changed = state.refresh_fast();
            if fast_changed {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
            }
            let _ = codex
                .runtime_snapshot(feature_settings.codex_generation)
                .save_default();
            fast_subscription.observed(refresh_now, fast_changed || codex_changed || opened_codex);
        }
        if system_subscription.is_due(Instant::now()) {
            let refresh_now = Instant::now();
            let mut requested = OptionalFeatureSettings::load_default();
            requested.codex_enabled = requested.effective_codex_enabled();
            if requested != feature_settings {
                feature_settings = requested;
                if codex.apply_settings(&mut shell, &feature_settings) {
                    if feature_settings.codex_enabled {
                        state.apply_codex_projection(CodexAvailabilityProjection::new(
                            FeatureSupport::Supported,
                            codex.installation,
                            true,
                            FeatureHealth::Loading,
                            feature_settings.codex_generation,
                            Some("Checking the selected Codex backend…".into()),
                        ));
                        if let Err(error) = codex.ensure_project_menu(&shell) {
                            tracing::warn!(%error, "Codex integration could not be enabled");
                        }
                    } else {
                        state.apply_codex_projection(CodexAvailabilityProjection::new(
                            FeatureSupport::Supported,
                            codex.installation,
                            false,
                            FeatureHealth::Unknown,
                            feature_settings.codex_generation,
                            Some("Codex integration is disabled".into()),
                        ));
                    }
                    sync_visibility(&mut shell, &state);
                    render_all(&mut shell, &mut state)?;
                }
            }
            let _ = codex
                .runtime_snapshot(feature_settings.codex_generation)
                .save_default();
            let system_changed = state.refresh_system();
            let codex_theme_changed = codex.set_theme(state.semantic_theme());
            let primary_output_changed =
                shell.set_primary_output_name(state.primary_output_name())?;
            if system_changed || primary_output_changed || codex_theme_changed {
                sync_visibility(&mut shell, &state);
                render_all(&mut shell, &mut state)?;
                if codex_theme_changed {
                    let mut surfaces = vec![codex.project_menu];
                    surfaces.extend(codex.chats.iter().map(|chat| chat.id));
                    for surface in surfaces {
                        codex.present(&mut shell, surface).map_err(|error| {
                            format!("could not redraw Codex surface after theme change: {error:?}")
                        })?;
                    }
                }
            }
            system_subscription.observed(refresh_now, system_changed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        CodexRuntimeInput, FeatureEffectiveState, FeatureHealth, FeatureInstallation,
        codex_runtime_from,
    };
    use nickel_ui::ControllerAction;
    use std::time::{Duration, Instant};

    fn embedded_chat() -> EmbeddedUiSurface<ChatApplication> {
        let backend = ReplayBackend::from_json(r#"{"name":"embedded-shell-host","events":[]}"#)
            .expect("static replay backend");
        let mut application = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_chat(std::path::Path::new("/projects/nickel"));
        application.state.status = ConnectionStatus::Ready;
        EmbeddedUiSurface::new(application, 900, 640, Instant::now())
    }

    fn pointer_button(
        order: u64,
        button: nickel_input::PointerButton,
        edge: nickel_input::KeyEdge,
        point: InputPoint,
    ) -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(1),
            order: EventOrder(order),
            button,
            edge,
            position: Some(point),
        })
    }

    fn center(node: &nickel_ui::AccessibilityNode) -> InputPoint {
        InputPoint {
            x: f64::from(node.rect.origin.x + node.rect.size.width / 2.0),
            y: f64::from(node.rect.origin.y + node.rect.size.height / 2.0),
        }
    }

    #[test]
    fn production_embedded_chat_host_exposes_accessibility_and_restores_text_focus() {
        let mut surface = embedded_chat();
        for role in [
            SemanticRole::Button,
            SemanticRole::TextField,
            SemanticRole::Menu,
        ] {
            assert!(
                surface
                    .accessibility_nodes()
                    .iter()
                    .any(|node| node.semantic_role == Some(role)),
                "missing embedded accessibility role {role:?}"
            );
        }
        assert!(
            surface
                .accessibility_nodes()
                .iter()
                .any(|node| node.component == "Text"),
            "embedded accessibility tree omitted visible text"
        );

        let draft = surface
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::TextField))
            .expect("chat composer text field")
            .clone();
        let point = center(&draft);
        surface.normalized_input(
            pointer_button(
                1,
                nickel_input::PointerButton::Primary,
                nickel_input::KeyEdge::Pressed,
                point,
            ),
            None,
        );
        surface.normalized_input(
            pointer_button(
                2,
                nickel_input::PointerButton::Primary,
                nickel_input::KeyEdge::Released,
                point,
            ),
            None,
        );
        assert_eq!(surface.inspection().keyboard_focus, Some(draft.id.clone()));

        surface.window_focus(false);
        assert!(!surface.inspection().window_focused);
        assert_eq!(surface.inspection().keyboard_focus, Some(draft.id.clone()));
        surface.window_focus(true);
        assert!(surface.inspection().window_focused);
        assert_eq!(surface.inspection().keyboard_focus, Some(draft.id));

        let preedit = surface.normalized_input(
            InputEvent::Text(TextEvent::Preedit {
                device: DeviceId(1),
                order: EventOrder(3),
                text: "世".into(),
                selection: Some((0, 3)),
            }),
            None,
        );
        assert!(preedit.changed);
        assert!(surface.application_mut().state.draft.is_empty());
        let commit = surface.normalized_input(
            InputEvent::Text(TextEvent::Commit {
                device: DeviceId(1),
                order: EventOrder(4),
                text: "世界".into(),
            }),
            None,
        );
        assert!(commit.changed);
        assert_eq!(surface.application_mut().state.draft, "世界");
    }

    #[test]
    fn production_embedded_chat_host_routes_scroll_context_menu_and_suspend() {
        let mut surface = embedded_chat();
        for index in 0..200 {
            surface.application_mut().state.items.push_back(ChatItem {
                id: format!("item-{index}"),
                kind: ChatItemKind::Agent,
                text: format!("history item {index}"),
                complete: true,
            });
        }
        surface.step(HostBatch {
            surface_size: Some((901, 640)),
            ..HostBatch::default()
        });

        let conversation = surface
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Conversation"))
            .expect("accessible conversation scroll surface")
            .clone();
        let scroll = surface.normalized_input(
            InputEvent::Pointer(PointerEvent::Axis {
                device: DeviceId(1),
                order: EventOrder(1),
                delta: Vector { x: 0.0, y: 1.0 },
                discrete: Some((0, 1)),
                position: Some(center(&conversation)),
            }),
            None,
        );
        assert!(scroll.changed);
        assert!(!surface.application_mut().state.conversation_pinned);

        let file_menu = surface
            .accessibility_nodes()
            .iter()
            .find(|node| {
                node.semantic_role == Some(SemanticRole::Menu)
                    && node.label.as_deref() == Some("File")
            })
            .expect("File menu")
            .clone();
        assert!(file_menu.actions.contains(&ActionKind::ContextMenu));
        let menu = surface.normalized_input(
            pointer_button(
                2,
                nickel_input::PointerButton::Secondary,
                nickel_input::KeyEdge::Pressed,
                center(&file_menu),
            ),
            None,
        );
        assert!(menu.changed);
        assert!(surface.accessibility_nodes().iter().any(|node| {
            node.semantic_role == Some(SemanticRole::MenuItem)
                && node.label.as_deref() == Some("New conversation")
        }));

        let primary = pointer_button(
            3,
            nickel_input::PointerButton::Primary,
            nickel_input::KeyEdge::Pressed,
            center(&file_menu),
        );
        surface.normalized_input(primary, None);
        surface.suspend();
        assert!(surface.inspection().pointer_capture.is_none());
        assert_eq!(surface.application_mut().state.items.len(), 200);
        assert!(!surface.accessibility_nodes().is_empty());

        surface.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::FocusGained)],
            ..HostBatch::default()
        });
        assert!(surface.inspection().window_focused);
    }

    #[test]
    fn unchanged_domain_subscriptions_back_off_and_changes_reset_the_deadline() {
        let started = Instant::now();
        let mut schedule = super::DomainSubscriptionSchedule::new(
            started,
            Duration::from_millis(10),
            Duration::from_millis(40),
        );
        schedule.observed(started, false);
        assert_eq!(schedule.deadline(), started + Duration::from_millis(20));
        schedule.observed(started, false);
        assert_eq!(schedule.deadline(), started + Duration::from_millis(40));
        schedule.observed(started, false);
        assert_eq!(schedule.deadline(), started + Duration::from_millis(40));
        assert_eq!(schedule.change_token, 0);
        schedule.observed(started, true);
        assert_eq!(schedule.deadline(), started + Duration::from_millis(10));
        assert_eq!(schedule.change_token, 1);
    }

    #[test]
    fn independent_domain_schedules_isolate_unchanged_services() {
        let started = Instant::now();
        let mut fast = super::DomainSubscriptionSchedule::new(
            started,
            Duration::from_millis(10),
            Duration::from_millis(80),
        );
        let mut system = super::DomainSubscriptionSchedule::new(
            started,
            Duration::from_millis(50),
            Duration::from_millis(200),
        );
        fast.observed(started, true);
        system.observed(started, false);
        assert_eq!(fast.change_token, 1);
        assert_eq!(system.change_token, 0);
        assert_eq!(fast.deadline(), started + Duration::from_millis(10));
        assert_eq!(system.deadline(), started + Duration::from_millis(100));
    }

    #[test]
    fn project_menu_deadline_is_polled_independently_of_fast_subscription() {
        let mut project_menu = embedded_chat();
        let due = project_menu.deadline().expect("chat host poll deadline");
        let mut no_chats = Vec::new();

        let _ = super::poll_due_codex_hosts(Some(&mut project_menu), &mut no_chats, due);

        assert!(
            project_menu
                .deadline()
                .is_some_and(|deadline| deadline > due)
        );
    }

    #[test]
    fn controller_launcher_action_toggles_launcher() {
        assert_eq!(
            super::controller_launcher_shortcut(ControllerAction::Launcher),
            Some(super::platform::GlobalShortcut::ToggleLauncher)
        );
    }

    #[test]
    fn controller_navigation_does_not_toggle_launcher() {
        assert_eq!(
            super::controller_launcher_shortcut(ControllerAction::Confirm),
            None
        );
    }

    #[test]
    fn visible_launcher_owns_controller_input_without_window_focus() {
        assert_eq!(
            super::controller_target_role(true, Some(super::SurfaceRole::Panel)),
            Some(super::SurfaceRole::Launcher)
        );
        assert_eq!(
            super::controller_target_role(false, Some(super::SurfaceRole::Panel)),
            Some(super::SurfaceRole::Panel)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn every_nonpersistent_wayland_shell_role_has_compositor_visibility_authority() {
        use nickel_session_protocol::ShellRole;

        for (surface, session) in [
            (super::SurfaceRole::ControlCenter, ShellRole::ControlCenter),
            (super::SurfaceRole::Notification, ShellRole::Notification),
            (super::SurfaceRole::VolumeOsd, ShellRole::VolumeOsd),
            (super::SurfaceRole::WindowPreview, ShellRole::Preview),
            (
                super::SurfaceRole::WindowContextMenu,
                ShellRole::ContextMenu,
            ),
            (super::SurfaceRole::CodexProjectMenu, ShellRole::ProjectMenu),
            (super::SurfaceRole::Screenshot, ShellRole::Screenshot),
        ] {
            assert_eq!(super::session_visibility_role(surface), Some(session));
        }
        for persistent in [
            super::SurfaceRole::Desktop,
            super::SurfaceRole::Panel,
            super::SurfaceRole::Launcher,
            super::SurfaceRole::Lock,
            super::SurfaceRole::CodexChat,
        ] {
            assert_eq!(super::session_visibility_role(persistent), None);
        }
    }

    #[test]
    fn ordinary_controller_surfaces_do_not_translate_actions_to_keys() {
        let source = include_str!("nickel-shell.rs");
        let handler = source
            .split("fn handle_controller_action(")
            .nth(1)
            .and_then(|source| source.split("fn handle_input_event(").next())
            .expect("controller handler remains inspectable");

        assert!(!handler.contains("KeyCode"));
        assert!(!handler.contains("_key("));
        for direct_dispatch in [
            "control_controller(action, width, height)",
            "preview_controller(action)",
            "window_menu_host_controller(action)",
            "notification_controller(action)",
            "panel_controller(action, width)",
            "screenshot_controller(action)",
        ] {
            assert!(
                handler.contains(direct_dispatch),
                "missing direct controller dispatch: {direct_dispatch}"
            );
        }
    }

    use std::path::Path;

    use nickel_codex::{Project, ReplayBackend, ThreadId};
    use nickel_codex_ui::{
        BackendMode, ChatApplication, ChatItem, ChatItemKind, ConnectionStatus, ShellRequest,
    };
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, Point as InputPoint, PointerEvent, TextEvent, Vector,
    };
    use nickel_ui::{
        ActionKind, HostBatch, HostEvent, SemanticAction, SemanticRole, SemanticValueInput, UiEvent,
    };

    use super::{
        EmbeddedUiSurface, WriterLeases, codex_project_application_id,
        step_embedded_codex_controller,
    };

    fn embedded_project_menu() -> EmbeddedUiSurface<ChatApplication> {
        let backend = ReplayBackend::from_json(r#"{"name":"embedded-menu","events":[]}"#)
            .expect("static replay is valid");
        let mut application = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_project_menu();
        application.state.projects = vec![
            Project {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: vec!["/projects/nickel".into()],
            },
            Project {
                id: "vesalius".into(),
                name: "Vesalius".into(),
                roots: vec!["/projects/vesalius".into()],
            },
        ];
        application.state.status = nickel_codex_ui::ConnectionStatus::Ready;
        let mut embedded = EmbeddedUiSurface::new(application, 920, 680, std::time::Instant::now());
        embedded.step(HostBatch {
            window_focused: Some(true),
            ..HostBatch::default()
        });
        embedded
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stale_startup_barrier_token_cannot_authorize_a_new_shell() {
        let mut token = Vec::from(*super::SHELL_STARTUP_BARRIER_MAGIC);
        token.extend_from_slice(&42_u32.to_ne_bytes());
        assert!(super::validate_supervisor_token(&token, 42).is_ok());
        let error = super::validate_supervisor_token(&token, 43).unwrap_err();
        assert!(error.contains("PID 42"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_barrier_token_requires_the_supervisor_magic() {
        let token = [0_u8; super::SHELL_STARTUP_BARRIER_MAGIC.len() + 4];
        assert_eq!(
            super::validate_supervisor_token(&token, 42).unwrap_err(),
            "Nickel shell startup barrier token is invalid"
        );
    }

    #[cfg(target_os = "linux")]
    fn shell_readiness(
        outputs: u16,
        desktops: u16,
        panels: u16,
        launchers: u16,
        reserved_ordinary_windows: u16,
        ready: bool,
    ) -> nickel_session_protocol::ShellReadinessSnapshot {
        nickel_session_protocol::ShellReadinessSnapshot {
            expected_shell_pid: Some(42),
            authenticated_shell_pid: Some(42),
            outputs,
            desktops,
            panels,
            locks: outputs,
            launchers,
            required_singletons_ready: true,
            output_roles_ready: true,
            reserved_ordinary_windows,
            ready,
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn healthy_shell_readiness_snapshot_is_accepted() {
        assert!(super::validate_shell_readiness(&shell_readiness(2, 2, 2, 1, 0, true)).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_shell_role_fails_closed_with_safe_counts() {
        let error =
            super::validate_shell_readiness(&shell_readiness(2, 1, 2, 1, 0, false)).unwrap_err();
        assert!(error.contains("outputs=2 desktops=1 panels=2 locks=2 launchers=1"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn duplicate_shell_role_fails_closed_with_safe_counts() {
        let error =
            super::validate_shell_readiness(&shell_readiness(2, 2, 2, 2, 0, false)).unwrap_err();
        assert!(error.contains("launchers=2"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn misclassified_shell_role_fails_closed_with_safe_counts() {
        let error =
            super::validate_shell_readiness(&shell_readiness(2, 2, 2, 1, 1, false)).unwrap_err();
        assert!(error.contains("reserved_ordinary_windows=1"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn readiness_barrier_accepts_late_roles_without_exposing_interaction() {
        let mut attempts = 0;
        let readiness = super::wait_for_shell_readiness_with(
            || {
                attempts += 1;
                Ok(if attempts < 3 {
                    shell_readiness(2, 1, 1, 1, 0, false)
                } else {
                    shell_readiness(2, 2, 2, 1, 0, true)
                })
            },
            std::time::Duration::from_secs(1),
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert_eq!(attempts, 3);
        assert!(readiness.ready);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn readiness_barrier_fails_closed_with_the_last_safe_snapshot() {
        let error = super::wait_for_shell_readiness_with(
            || Ok(shell_readiness(2, 1, 2, 1, 0, false)),
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.contains("did not converge within 0 ms"));
        assert!(error.contains("outputs=2 desktops=1 panels=2"));
    }

    #[test]
    fn headless_start_waits_in_one_process_until_a_display_returns() {
        let mut steps = 0;
        super::wait_for_initial_display_with(|| {
            steps += 1;
            Ok(steps == 3)
        })
        .unwrap();
        assert_eq!(steps, 3);
    }

    #[test]
    fn linux_output_loss_does_not_end_the_shell_process() {
        assert_eq!(
            super::shell_event_ends_process(&super::ShellEvent::Quit),
            !cfg!(target_os = "linux")
        );
    }

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
    fn embedded_project_menu_accessibility_set_value_uses_the_production_host() {
        let mut embedded = embedded_project_menu();
        let search = embedded
            .host
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::TextField))
            .expect("project search is exposed as a textbox")
            .clone();
        assert!(search.actions.contains(&ActionKind::SetValue));

        let outcome = embedded.step(HostBatch {
            events: vec![HostEvent::Accessibility {
                target: search.id,
                action: SemanticAction::SetValue(SemanticValueInput::Text("nick".into())),
            }],
            ..HostBatch::default()
        });

        assert!(outcome.changed);
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(embedded.host.application().state.draft, "nick");
        let labels = embedded
            .host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Nickel"));
        assert!(!labels.contains(&"Vesalius"));
    }

    #[test]
    fn embedded_project_menu_controller_opens_the_selected_project() {
        let mut embedded = embedded_project_menu();
        for action in [
            ControllerAction::Down,
            ControllerAction::Down,
            ControllerAction::Down,
            ControllerAction::Confirm,
        ] {
            step_embedded_codex_controller(&mut embedded, true, action);
        }

        assert_eq!(
            embedded.host.application_mut().take_shell_requests(),
            vec![ShellRequest::OpenProject {
                cwd: "/projects/nickel".into(),
                project_id: "nickel".into(),
                name: "Nickel".into(),
                initial_thread: None,
            }]
        );
    }

    #[test]
    fn embedded_project_menu_cancel_requests_dismiss_and_retains_selection_for_reopen() {
        let mut embedded = embedded_project_menu();
        step_embedded_codex_controller(&mut embedded, true, ControllerAction::Down);
        step_embedded_codex_controller(&mut embedded, true, ControllerAction::Down);
        let selected = embedded.host.inspect().controller_target;
        assert!(selected.is_some(), "controller acquired a semantic target");

        let transition =
            step_embedded_codex_controller(&mut embedded, true, ControllerAction::Cancel);
        assert!(transition.dismiss_surface);
        assert!(!transition.changed);
        assert_eq!(embedded.host.inspect().controller_target, selected);

        // Production dismissal hides and suspends this reusable overlay; it
        // does not destroy the host or route an ordinary-window focus loss.
        embedded.suspend();
        assert_eq!(embedded.host.inspect().controller_target, selected);
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

    #[test]
    fn disabled_codex_runtime_has_no_owned_resources() {
        let runtime = codex_runtime_from(CodexRuntimeInput {
            enabled: false,
            generation: 4,
            status: Some(ConnectionStatus::Ready),
            authenticated: true,
            active_windows: 7,
            cache_entries: 99,
            source_label: "ignored".into(),
            diagnostic: None,
            installation: FeatureInstallation::Installed,
        });
        assert_eq!(runtime.codex_generation, 4);
        assert_eq!(runtime.codex_installation, FeatureInstallation::Installed);
        assert_eq!(runtime.source_label, "ignored");
        assert!(runtime.disabled_is_quiescent());
    }

    #[test]
    fn codex_runtime_reports_loading_signed_out_failed_and_ready() {
        let snapshot = |status, authenticated| {
            codex_runtime_from(CodexRuntimeInput {
                enabled: true,
                generation: 8,
                status,
                authenticated,
                active_windows: 1,
                cache_entries: 3,
                source_label: "fixture".into(),
                diagnostic: None,
                installation: FeatureInstallation::Installed,
            })
        };
        assert_eq!(
            snapshot(Some(ConnectionStatus::Loading), false).codex_effective,
            FeatureEffectiveState::Enabling
        );
        assert_eq!(
            snapshot(Some(ConnectionStatus::Ready), false).codex_health,
            FeatureHealth::SignedOut
        );
        assert_eq!(
            snapshot(Some(ConnectionStatus::Unavailable), false).codex_effective,
            FeatureEffectiveState::Rejected
        );
        let ready = snapshot(Some(ConnectionStatus::Ready), true);
        assert_eq!(ready.codex_health, FeatureHealth::Ready);
        assert_eq!(ready.background_workers, 1);
        assert_eq!(ready.subscriptions, 2);
        assert_eq!(ready.warm_surfaces, 2);
        assert_eq!(ready.cache_entries, 3);
    }
}
