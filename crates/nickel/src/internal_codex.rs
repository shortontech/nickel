//! Codex applications hosted directly by the Linux compositor.
//!
//! The Windows standalone shell keeps its native `WinitShell` path. This
//! coordinator contains no native window or event-loop identity: the session's
//! [`InternalUiRuntime`] owns each `UiHost` and its frame lifecycle.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};

use nickel_codex::{BackendChoice, ThreadId};
use nickel_codex_ui::{ChatApplication, ShellRequest, shell_application_with_backend};
use nickel_core::optional_features::{CodexSource, OptionalFeatureSettings};
use twinkle::{Application, HostBatch, HostEvent, InternalSurfaceId};

use crate::session::{InternalSurfacePlacement, InternalSurfaceRole, InternalUiRuntime};

pub(crate) const CHAT_SIZE: (u32, u32) = (1120, 760);

#[derive(Clone, Debug)]
pub struct CodexSurfacePlacement {
    pub output: Option<String>,
    pub origin: (i32, i32),
    pub scale: f32,
    /// Bound a new chat's client area to its decorated output work area.
    pub chat_size: Option<(u32, u32)>,
}

impl Default for CodexSurfacePlacement {
    fn default() -> Self {
        Self {
            output: None,
            origin: (0, 0),
            scale: 1.0,
            chat_size: None,
        }
    }
}

#[derive(Debug)]
struct ChatSurface {
    id: InternalSurfaceId,
    project_id: String,
    thread_id: Option<ThreadId>,
    pending_thread: Option<ThreadId>,
}

#[derive(Debug)]
pub enum NativeProjectMenuAction {
    Closed,
    Opened(InternalSurfaceId),
}

/// Owns the identities and domain leases for compositor-hosted Codex UI.
pub struct InternalCodexHost {
    settings: OptionalFeatureSettings,
    theme: twinkle::SemanticTheme,
    cwd: PathBuf,
    project_controller: Option<InternalSurfaceId>,
    chats: Vec<ChatSurface>,
    writer_leases: HashSet<ThreadId>,
}

impl InternalCodexHost {
    pub fn new(
        settings: OptionalFeatureSettings,
        theme: twinkle::SemanticTheme,
        cwd: PathBuf,
    ) -> Self {
        Self {
            settings,
            theme,
            cwd,
            project_controller: None,
            chats: Vec::new(),
            writer_leases: HashSet::new(),
        }
    }

    pub fn refresh_project_menu(&mut self, runtime: &mut InternalUiRuntime) -> bool {
        let Some(id) = self.project_controller else {
            return false;
        };
        let Some(app) = runtime.application_mut::<ChatApplication>(id) else {
            return false;
        };
        app.update(nickel_codex_ui::ChatMessage::Refresh);
        runtime.step(
            id,
            HostBatch {
                application_changed: true,
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            },
        );
        true
    }

    pub fn sync_shell_projection(
        &self,
        shell: &mut crate::internal_shell::InternalShellCoordinator,
        runtime: &InternalUiRuntime,
    ) -> bool {
        use crate::launcher::{
            DashboardProject, DashboardSection, ProjectActivity, normalize_dashboard_projects,
        };
        use nickel_codex_ui::ConnectionStatus;
        use nickel_core::optional_features::FeatureInstallation;

        let Some(snapshot) = self
            .project_controller
            .as_ref()
            .and_then(|id| runtime.application::<ChatApplication>(*id))
            .map(|app| &app.state)
        else {
            return false;
        };
        let diagnostic = (!snapshot.provenance.is_empty())
            .then(|| snapshot.provenance.clone())
            .or_else(|| snapshot.diagnostics.back().cloned());
        let availability_changed = shell.apply_codex_projection(crate::codex_projection(
            &self.settings,
            FeatureInstallation::Missing,
            snapshot.status.clone(),
            snapshot.account.authenticated,
            diagnostic.clone(),
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
            | ConnectionStatus::Incompatible => DashboardSection::Unavailable(
                diagnostic.unwrap_or_else(|| "Codex backend unavailable".into()),
            ),
        };
        let projects_changed = shell.set_dashboard_projects(projects);
        availability_changed || projects_changed
    }

    pub fn surface_ids(&self) -> impl Iterator<Item = InternalSurfaceId> + '_ {
        self.project_controller
            .into_iter()
            .chain(self.chats.iter().map(|chat| chat.id))
    }

    pub(crate) fn approval_notifications(
        &self,
        runtime: &InternalUiRuntime,
    ) -> Vec<(
        crate::live_shell::CodexApprovalOwner,
        nickel_codex_ui::CodexApprovalNotification,
    )> {
        self.chats
            .iter()
            .filter_map(|chat| {
                runtime
                    .application::<ChatApplication>(chat.id)
                    .map(|app| (chat.id, app.approval_notifications()))
            })
            .flat_map(|(id, approvals)| {
                approvals.into_iter().map(move |approval| {
                    (
                        crate::live_shell::CodexApprovalOwner::Internal(id),
                        approval,
                    )
                })
            })
            .collect()
    }

    pub fn project_application_id(&self, surface: InternalSurfaceId) -> Option<String> {
        self.chats
            .iter()
            .find(|chat| chat.id == surface)
            .map(|chat| {
                crate::codex_project_application_id(
                    Some(&chat.project_id),
                    std::path::Path::new(""),
                )
            })
    }

    pub fn active_chat_count(&self) -> u32 {
        self.chats.len().min(u32::MAX as usize) as u32
    }

    pub fn next_deadline(&self, runtime: &InternalUiRuntime) -> Option<Instant> {
        self.surface_ids()
            .filter_map(|id| runtime.surface_deadline(id))
            .min()
    }

    pub fn set_theme(
        &mut self,
        runtime: &mut InternalUiRuntime,
        theme: twinkle::SemanticTheme,
    ) -> Vec<InternalSurfaceId> {
        if self.theme == theme {
            return Vec::new();
        }
        self.theme = theme;
        let ids = self.surface_ids().collect::<Vec<_>>();
        let mut changed = Vec::new();
        for id in ids {
            if runtime
                .application_mut::<ChatApplication>(id)
                .is_some_and(|application| application.set_theme(theme))
            {
                runtime.step(
                    id,
                    HostBatch {
                        application_changed: true,
                        events: vec![twinkle::HostEvent::Poll],
                        ..HostBatch::default()
                    },
                );
                changed.push(id);
            }
        }
        changed
    }

    pub fn ensure_project_menu(&mut self, runtime: &mut InternalUiRuntime) -> Result<(), String> {
        if self.project_controller.is_some() {
            return Ok(());
        }
        let mut application = shell_application_with_backend(
            self.cwd.clone(),
            true,
            None,
            None,
            self.backend_choice(),
        )?;
        application.set_theme(self.theme);
        let id = runtime.insert(
            application,
            internal_placement(
                CodexSurfacePlacement::default(),
                (360, 420),
                InternalSurfaceRole::Overlay,
            ),
            1.0,
        );
        runtime.set_visible(id, false);
        self.project_controller = Some(id);
        Ok(())
    }

    pub(crate) fn dismiss_unfocused_project_menu(
        &mut self,
        runtime: &mut InternalUiRuntime,
    ) -> bool {
        let Some(id) = self.project_controller else {
            return false;
        };
        if runtime.is_visible(id) && runtime.focused() != Some(id) {
            return runtime.set_visible(id, false);
        }
        false
    }

    pub fn sync_project_menu(
        &mut self,
        runtime: &mut InternalUiRuntime,
        visible: bool,
        placement: CodexSurfacePlacement,
    ) -> bool {
        let Some(id) = self.project_controller else {
            return false;
        };
        let was_visible = runtime.is_visible(id);
        let size = placement.chat_size.map_or((360, 420), |(width, height)| {
            (360.min(width), 420.min(height))
        });
        let moved = runtime.relocate(
            id,
            internal_placement(placement, size, InternalSurfaceRole::Overlay),
        );
        let changed = runtime.set_visible(id, visible);
        if visible && !was_visible {
            runtime.focus_surface(id);
        }
        changed || moved
    }

    pub fn service_project_menu_requests(
        &mut self,
        runtime: &mut InternalUiRuntime,
        placement: CodexSurfacePlacement,
    ) -> Result<Option<NativeProjectMenuAction>, String> {
        let Some(id) = self.project_controller else {
            return Ok(None);
        };
        let requests = runtime
            .application_mut::<ChatApplication>(id)
            .ok_or("project context retired")?
            .take_shell_requests();
        for request in requests {
            if matches!(request, ShellRequest::CloseProjectMenu) {
                runtime.set_visible(id, false);
                return Ok(Some(NativeProjectMenuAction::Closed));
            }
            if let ShellRequest::OpenProject { project_id, .. } = request {
                // Re-resolve the owning backend model; JSX/native input cannot supply paths.
                let surface = self.open_project_by_id(runtime, placement, &project_id)?;
                runtime.set_visible(id, false);
                return Ok(Some(NativeProjectMenuAction::Opened(surface)));
            }
        }
        Ok(None)
    }

    pub fn open_project(
        &mut self,
        runtime: &mut InternalUiRuntime,
        placement: CodexSurfacePlacement,
        cwd: PathBuf,
        project_id: String,
        initial_thread: Option<ThreadId>,
    ) -> Result<InternalSurfaceId, String> {
        if let Some(chat) = self
            .chats
            .iter()
            .find(|chat| chat.project_id == project_id && chat.thread_id == initial_thread)
        {
            runtime.focus_surface(chat.id);
            return Ok(chat.id);
        }
        if let Some(thread) = initial_thread.as_ref()
            && !self.writer_leases.insert(thread.clone())
        {
            return Err(format!("thread {} already has a Nickel writer", thread.0));
        }
        let result = (|| {
            let mut application = shell_application_with_backend(
                cwd,
                false,
                initial_thread.clone(),
                Some(project_id.clone()),
                self.backend_choice(),
            )?;
            application.set_theme(self.theme);
            let scale = placement.scale;
            let size = placement.chat_size.unwrap_or(CHAT_SIZE);
            let id = runtime.insert(
                application,
                internal_placement(placement, size, InternalSurfaceRole::Application),
                scale,
            );
            runtime.focus_surface(id);
            self.chats.push(ChatSurface {
                id,
                project_id,
                thread_id: initial_thread.clone(),
                pending_thread: None,
            });
            Ok(id)
        })();
        if result.is_err()
            && let Some(thread) = initial_thread
        {
            self.writer_leases.remove(&thread);
        }
        result
    }

    /// Resolve a launcher project identity through the already-owned project
    /// menu model and open its most recently used thread directly.
    pub fn open_project_by_id(
        &mut self,
        runtime: &mut InternalUiRuntime,
        placement: CodexSurfacePlacement,
        project_id: &str,
    ) -> Result<InternalSurfaceId, String> {
        let (project, initial_thread) = {
            let id = self
                .project_controller
                .ok_or("Codex project data is still loading")?;
            let state = &runtime
                .application::<ChatApplication>(id)
                .ok_or("Codex project context retired")?
                .state;
            if state.status != nickel_codex_ui::ConnectionStatus::Ready
                || !state.account.authenticated
            {
                return Err("Codex project backend is unavailable".to_owned());
            }
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
                        .and_then(|entry| entry.project_id.as_deref())
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
        self.open_project(runtime, placement, cwd, project.id, initial_thread)
    }

    /// Feed normalized input, focus, and resize data to an owned application.
    #[allow(dead_code)] // Explicit adapter API for non-routed host batches (IME/clipboard).
    pub fn step(
        &mut self,
        runtime: &mut InternalUiRuntime,
        id: InternalSurfaceId,
        batch: HostBatch,
    ) -> bool {
        self.owns(id) && runtime.step(id, batch)
    }

    pub fn poll_due(
        &mut self,
        runtime: &mut InternalUiRuntime,
        now: Instant,
    ) -> Vec<InternalSurfaceId> {
        let ids = self.surface_ids().collect::<Vec<_>>();
        ids.into_iter()
            .filter(|id| runtime.poll_surface(*id, now))
            .collect()
    }

    /// Route conversation selection through the shell's single-writer leases.
    pub fn service_chat_requests(&mut self, runtime: &mut InternalUiRuntime) -> bool {
        let owners = self
            .chats
            .iter()
            .filter_map(|chat| {
                chat.thread_id
                    .as_ref()
                    .map(|thread| (thread.clone(), chat.id))
            })
            .collect::<HashMap<_, _>>();
        let mut changed = false;
        for chat in &mut self.chats {
            let Some(app) = runtime.application_mut::<ChatApplication>(chat.id) else {
                continue;
            };
            for request in app.take_shell_requests() {
                match request {
                    ShellRequest::ResumeThread(thread) => {
                        if chat.pending_thread.is_some() {
                            continue;
                        }
                        changed = true;
                        if let Some(owner) = owners.get(&thread) {
                            runtime.focus_surface(*owner);
                            if let Some(app) = runtime.application_mut::<ChatApplication>(chat.id) {
                                app.report_resume_owner_activation();
                            }
                            continue;
                        }
                        let Some(app) = runtime.application_mut::<ChatApplication>(chat.id) else {
                            continue;
                        };
                        if app.state.thread_runtime.get(&thread).is_some_and(|entry| {
                            entry.status == nickel_codex::ThreadRuntimeStatus::Active
                        }) {
                            app.report_resume_rejection(
                                "Conversation is active outside this Nickel session",
                            );
                            continue;
                        }
                        if !app.prepare_shell_resume(&thread) {
                            continue;
                        }
                        if !self.writer_leases.insert(thread.clone()) {
                            app.report_resume_rejection(format!(
                                "Conversation {} already has a Nickel writer",
                                thread.0
                            ));
                            continue;
                        }
                        if let Err(error) = app.resume_thread(thread.clone()) {
                            self.writer_leases.remove(&thread);
                            app.report_resume_rejection(error);
                            continue;
                        }
                        chat.pending_thread = Some(thread);
                    }
                    ShellRequest::ResumeSucceeded(thread) => {
                        if chat.pending_thread.as_ref() == Some(&thread) {
                            if let Some(previous) = chat.thread_id.replace(thread) {
                                self.writer_leases.remove(&previous);
                            }
                            chat.pending_thread = None;
                            changed = true;
                        }
                    }
                    ShellRequest::ResumeFailed(thread) => {
                        if chat.pending_thread.as_ref() == Some(&thread) {
                            self.writer_leases.remove(&thread);
                            chat.pending_thread = None;
                            changed = true;
                        } else if chat.thread_id.as_ref() == Some(&thread) {
                            self.writer_leases.remove(&thread);
                            chat.thread_id = None;
                            changed = true;
                        }
                    }
                    ShellRequest::OpenProject { .. } | ShellRequest::CloseProjectMenu => {}
                }
            }
        }
        changed
    }

    pub fn close(&mut self, runtime: &mut InternalUiRuntime, id: InternalSurfaceId) -> bool {
        let Some(index) = self.chats.iter().position(|chat| chat.id == id) else {
            return false;
        };
        let chat = self.chats.remove(index);
        if let Some(thread) = chat.thread_id {
            self.writer_leases.remove(&thread);
        }
        if let Some(thread) = chat.pending_thread {
            self.writer_leases.remove(&thread);
        }
        runtime.remove(id)
    }

    #[allow(dead_code)] // Used by session teardown once runtime shutdown becomes explicit.
    pub fn shutdown(&mut self, runtime: &mut InternalUiRuntime) {
        let ids = self.surface_ids().collect::<Vec<_>>();
        for id in ids {
            runtime.remove(id);
        }
        self.project_controller = None;
        self.chats.clear();
        self.writer_leases.clear();
    }

    #[allow(dead_code)]
    fn owns(&self, id: InternalSurfaceId) -> bool {
        self.project_controller == Some(id) || self.chats.iter().any(|chat| chat.id == id)
    }

    fn backend_choice(&self) -> Option<BackendChoice> {
        match &self.settings.codex_source {
            CodexSource::CompatibleInstalled => Some(BackendChoice::Installed),
            CodexSource::Bundled => Some(BackendChoice::Bundled),
            CodexSource::ApprovedRemote => None,
            CodexSource::Executable(path) => Some(BackendChoice::Path(path.clone())),
        }
    }
}

fn internal_placement(
    placement: CodexSurfacePlacement,
    size: (u32, u32),
    role: InternalSurfaceRole,
) -> InternalSurfacePlacement {
    InternalSurfacePlacement {
        role,
        geometry: (placement.origin.0, placement.origin.1, size.0, size.1),
        output: placement.output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twinkle::{Application, Text, View, ViewContext};

    struct TestApp;

    impl Application for TestApp {
        type Message = ();

        fn update(&mut self, (): ()) {}

        fn view(&self, _: ViewContext) -> impl View<Self::Message> {
            Text::new("Codex owned by compositor")
        }

        fn title(&self) -> &str {
            "Codex"
        }
    }

    fn host() -> InternalCodexHost {
        InternalCodexHost::new(
            OptionalFeatureSettings {
                codex_source: nickel_core::optional_features::CodexSource::CompatibleInstalled,
                ..OptionalFeatureSettings::default()
            },
            crate::window_preview::semantic_theme_from_palette(
                nickel_core::theme::ThemePalette::from_appearance(Default::default()),
            ),
            PathBuf::from("/tmp"),
        )
    }

    fn placement(role: InternalSurfaceRole) -> InternalSurfacePlacement {
        InternalSurfacePlacement {
            role,
            geometry: (10, 20, 640, 480),
            output: Some("test".into()),
        }
    }

    #[test]
    fn chat_project_identity_matches_native_codex_windows() {
        let mut runtime = InternalUiRuntime::default();
        let first = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let second = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let mut host = host();
        for id in [first, second] {
            host.chats.push(ChatSurface {
                id,
                project_id: "project-1".into(),
                thread_id: None,
                pending_thread: None,
            });
        }
        let expected =
            crate::codex_project_application_id(Some("project-1"), std::path::Path::new(""));
        assert_eq!(host.project_application_id(first), Some(expected.clone()));
        assert_eq!(host.project_application_id(second), Some(expected));
    }

    #[test]
    fn native_project_switcher_keeps_one_controller_and_only_focuses_when_shown() {
        let mut runtime = InternalUiRuntime::default();
        let application = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        runtime.focus_surface(application);
        let mut host = host();
        let controller = ChatApplication::new(nickel_codex_ui::BackendMode::Replay {
            backend: nickel_codex::ReplayBackend::from_json(
                r#"{"name":"native-projects","projects":[],"events":[]}"#,
            )
            .unwrap(),
            cwd: PathBuf::from("/tmp"),
        })
        .as_shell_project_menu();
        let menu = runtime.insert(controller, placement(InternalSurfaceRole::Overlay), 1.0);
        runtime.set_visible(menu, false);
        host.project_controller = Some(menu);
        assert_eq!(host.surface_ids().count(), 1);
        assert_eq!(runtime.len(), 2);
        assert_eq!(runtime.focused(), Some(application));
        assert!(host.refresh_project_menu(&mut runtime));
        assert_eq!(runtime.focused(), Some(application));
        host.sync_project_menu(&mut runtime, true, CodexSurfacePlacement::default());
        assert_eq!(runtime.focused(), Some(menu));
        assert!(
            runtime
                .semantic_nodes(menu)
                .iter()
                .any(|node| node.role == Some(twinkle::SemanticRole::TextField))
        );
        runtime.focus_surface(application);
        assert!(host.dismiss_unfocused_project_menu(&mut runtime));
        assert_eq!(runtime.focused(), Some(application));
        assert!(!host.dismiss_unfocused_project_menu(&mut runtime));
        assert!(!runtime.is_visible(menu));
    }

    #[test]
    fn native_project_selection_revalidates_backend_identity_before_opening_a_chat() {
        let mut runtime = InternalUiRuntime::default();
        let mut application = ChatApplication::new(nickel_codex_ui::BackendMode::Replay {
            backend: nickel_codex::ReplayBackend::from_json(
                r#"{"name":"native-project-selection","projects":[],"events":[]}"#,
            )
            .unwrap(),
            cwd: PathBuf::from("/tmp"),
        });
        application.state.status = nickel_codex_ui::ConnectionStatus::Ready;
        application.state.account.authenticated = true;
        application.state.projects = vec![nickel_codex::Project {
            id: "backend-project".into(),
            name: "Example".into(),
            roots: vec![PathBuf::from("/tmp/example")],
        }];
        let mut host = host();
        host.project_controller = Some(runtime.insert(
            application.as_shell_project_menu(),
            placement(InternalSurfaceRole::Overlay),
            1.0,
        ));
        let menu = host.project_controller.unwrap();
        runtime.step(
            menu,
            HostBatch {
                application_changed: true,
                ..Default::default()
            },
        );
        let button = runtime
            .semantic_nodes(menu)
            .into_iter()
            .find(|node| {
                node.name.as_deref() == Some("Example")
                    && node.role == Some(twinkle::SemanticRole::Button)
            })
            .expect("native project button");
        let point = twinkle::Point {
            x: button.bounds.origin.x + button.bounds.size.width / 2.0,
            y: button.bounds.origin.y + button.bounds.size.height / 2.0,
        };
        runtime.step(
            menu,
            HostBatch {
                events: vec![
                    HostEvent::Ui(twinkle::UiEvent::PointerPressed(point)),
                    HostEvent::Ui(twinkle::UiEvent::PointerReleased(point)),
                ],
                ..Default::default()
            },
        );
        // A backend refresh retires this identity before the native service consumes it.
        runtime
            .application_mut::<ChatApplication>(menu)
            .unwrap()
            .state
            .projects
            .clear();
        assert!(
            host.service_project_menu_requests(&mut runtime, CodexSurfacePlacement::default())
                .unwrap_err()
                .contains("unavailable")
        );
        assert_eq!(host.active_chat_count(), 0);
    }

    #[test]
    fn chat_close_releases_writer_and_removes_runtime_host() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let thread = ThreadId("thread-1".into());
        let pending = ThreadId("thread-2".into());
        let mut host = host();
        host.writer_leases.insert(thread.clone());
        host.writer_leases.insert(pending.clone());
        host.chats.push(ChatSurface {
            id,
            project_id: "project-1".into(),
            thread_id: Some(thread.clone()),
            pending_thread: Some(pending.clone()),
        });

        assert!(host.close(&mut runtime, id));
        assert!(!host.writer_leases.contains(&thread));
        assert!(!host.writer_leases.contains(&pending));
        assert!(runtime.is_empty());
    }

    #[test]
    fn shutdown_removes_every_owned_host_but_not_foreign_surfaces() {
        let mut runtime = InternalUiRuntime::default();
        let chat = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let foreign = runtime.insert(TestApp, placement(InternalSurfaceRole::Taskbar), 1.0);
        let mut host = host();
        host.chats.push(ChatSurface {
            id: chat,
            project_id: "project-1".into(),
            thread_id: None,
            pending_thread: None,
        });

        host.shutdown(&mut runtime);

        assert_eq!(runtime.len(), 1);
        assert!(runtime.application::<TestApp>(foreign).is_some());
        assert!(host.surface_ids().next().is_none());
    }
}
