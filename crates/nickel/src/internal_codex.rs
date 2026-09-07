//! Codex applications hosted directly by the Linux compositor.
//!
//! The Windows standalone shell keeps its native `WinitShell` path. This
//! coordinator contains no native window or event-loop identity: the session's
//! [`InternalUiRuntime`] owns each `UiHost` and its frame lifecycle.

use std::{collections::HashSet, path::PathBuf, time::Instant};

use nickel_codex::{BackendChoice, ThreadId};
use nickel_codex_ui::{ChatApplication, ShellRequest, shell_application_with_backend};
use nickel_core::optional_features::CodexSource;
use nickel_ui::{HostBatch, InternalSurfaceId};

use crate::session::{InternalSurfacePlacement, InternalSurfaceRole, InternalUiRuntime};

const MENU_SIZE: (u32, u32) = (520, 680);
const CHAT_SIZE: (u32, u32) = (1120, 760);

#[derive(Clone, Debug)]
pub struct CodexSurfacePlacement {
    pub output: Option<String>,
    pub origin: (i32, i32),
    pub scale: f32,
}

impl Default for CodexSurfacePlacement {
    fn default() -> Self {
        Self {
            output: None,
            origin: (0, 0),
            scale: 1.0,
        }
    }
}

#[derive(Debug)]
struct ChatSurface {
    id: InternalSurfaceId,
    project_id: String,
    thread_id: Option<ThreadId>,
}

/// Owns the identities and domain leases for compositor-hosted Codex UI.
pub struct InternalCodexHost {
    source: CodexSource,
    theme: nickel_ui::SemanticTheme,
    cwd: PathBuf,
    project_menu: Option<InternalSurfaceId>,
    chats: Vec<ChatSurface>,
    writer_leases: HashSet<ThreadId>,
}

impl InternalCodexHost {
    pub fn new(source: CodexSource, theme: nickel_ui::SemanticTheme, cwd: PathBuf) -> Self {
        Self {
            source,
            theme,
            cwd,
            project_menu: None,
            chats: Vec::new(),
            writer_leases: HashSet::new(),
        }
    }

    pub fn project_menu(&self) -> Option<InternalSurfaceId> {
        self.project_menu
    }

    pub fn surface_ids(&self) -> impl Iterator<Item = InternalSurfaceId> + '_ {
        self.project_menu
            .into_iter()
            .chain(self.chats.iter().map(|chat| chat.id))
    }

    pub fn next_deadline(&self, runtime: &InternalUiRuntime) -> Option<Instant> {
        self.surface_ids()
            .filter_map(|id| runtime.surface_deadline(id))
            .min()
    }

    pub fn ensure_project_menu(
        &mut self,
        runtime: &mut InternalUiRuntime,
        placement: CodexSurfacePlacement,
    ) -> Result<InternalSurfaceId, String> {
        if let Some(id) = self.project_menu {
            return Ok(id);
        }
        let mut application = shell_application_with_backend(
            self.cwd.clone(),
            true,
            None,
            None,
            self.backend_choice(),
        )?;
        application.set_theme(self.theme);
        let scale = placement.scale;
        let id = runtime.insert(
            application,
            internal_placement(placement, MENU_SIZE, InternalSurfaceRole::Overlay),
            scale,
        );
        self.project_menu = Some(id);
        Ok(id)
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
            let id = runtime.insert(
                application,
                internal_placement(placement, CHAT_SIZE, InternalSurfaceRole::Application),
                scale,
            );
            runtime.focus_surface(id);
            self.chats.push(ChatSurface {
                id,
                project_id,
                thread_id: initial_thread.clone(),
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
        let menu = self
            .project_menu
            .ok_or_else(|| "Codex project data is still loading".to_owned())?;
        let (project, initial_thread) = {
            let state = &runtime
                .application::<ChatApplication>(menu)
                .ok_or_else(|| "Codex project menu host is unavailable".to_owned())?
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

    /// Drain typed menu requests and create chat hosts in the same runtime.
    pub fn service_requests(
        &mut self,
        runtime: &mut InternalUiRuntime,
        placement: CodexSurfacePlacement,
    ) -> Result<Vec<InternalSurfaceId>, String> {
        let Some(menu) = self.project_menu else {
            return Ok(Vec::new());
        };
        let requests = runtime
            .application_mut::<ChatApplication>(menu)
            .map(ChatApplication::take_shell_requests)
            .unwrap_or_default();
        let mut opened = Vec::new();
        for request in requests {
            if let ShellRequest::OpenProject {
                cwd,
                project_id,
                initial_thread,
                ..
            } = request
            {
                opened.push(self.open_project(
                    runtime,
                    placement.clone(),
                    cwd,
                    project_id,
                    initial_thread,
                )?);
            }
        }
        Ok(opened)
    }

    pub fn close(&mut self, runtime: &mut InternalUiRuntime, id: InternalSurfaceId) -> bool {
        if self.project_menu == Some(id) {
            self.project_menu = None;
            return runtime.remove(id);
        }
        let Some(index) = self.chats.iter().position(|chat| chat.id == id) else {
            return false;
        };
        if let Some(thread) = self.chats.remove(index).thread_id {
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
        self.project_menu = None;
        self.chats.clear();
        self.writer_leases.clear();
    }

    #[allow(dead_code)]
    fn owns(&self, id: InternalSurfaceId) -> bool {
        self.project_menu == Some(id) || self.chats.iter().any(|chat| chat.id == id)
    }

    fn backend_choice(&self) -> Option<BackendChoice> {
        match &self.source {
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
    use nickel_ui::{Application, Text, View, ViewContext};

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
            CodexSource::CompatibleInstalled,
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
    fn project_menu_identity_is_an_internal_surface_id() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(TestApp, placement(InternalSurfaceRole::Overlay), 1.0);
        let mut host = host();
        host.project_menu = Some(id);

        assert_eq!(host.project_menu(), Some(id));
        assert!(host.owns(id));
        assert!(runtime.application::<TestApp>(id).is_some());
        assert!(host.close(&mut runtime, id));
        assert!(runtime.is_empty());
    }

    #[test]
    fn chat_close_releases_writer_and_removes_runtime_host() {
        let mut runtime = InternalUiRuntime::default();
        let id = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let thread = ThreadId("thread-1".into());
        let mut host = host();
        host.writer_leases.insert(thread.clone());
        host.chats.push(ChatSurface {
            id,
            project_id: "project-1".into(),
            thread_id: Some(thread.clone()),
        });

        assert!(host.close(&mut runtime, id));
        assert!(!host.writer_leases.contains(&thread));
        assert!(runtime.is_empty());
    }

    #[test]
    fn shutdown_removes_every_owned_host_but_not_foreign_surfaces() {
        let mut runtime = InternalUiRuntime::default();
        let menu = runtime.insert(TestApp, placement(InternalSurfaceRole::Overlay), 1.0);
        let chat = runtime.insert(TestApp, placement(InternalSurfaceRole::Application), 1.0);
        let foreign = runtime.insert(TestApp, placement(InternalSurfaceRole::Panel), 1.0);
        let mut host = host();
        host.project_menu = Some(menu);
        host.chats.push(ChatSurface {
            id: chat,
            project_id: "project-1".into(),
            thread_id: None,
        });

        host.shutdown(&mut runtime);

        assert_eq!(runtime.len(), 1);
        assert!(runtime.application::<TestApp>(foreign).is_some());
        assert!(host.surface_ids().next().is_none());
    }
}
