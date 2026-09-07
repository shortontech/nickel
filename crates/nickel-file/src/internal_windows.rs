//! Compositor-owned Nickel File windows.
//!
//! This coordinator is the product-side replacement for launching a sibling
//! `nickel-file` process. It owns ordinary [`FileApp`] values through Nickel
//! UI's event-loop-free surface host and returns typed focus/open/close actions
//! for the compositor to apply.

use std::collections::BTreeMap;

use nickel_ui::{Application, HostedApplication, InternalSurfaceId, InternalSurfaceSet};

use crate::{FileApp, FileLaunch};

/// A request from shell policy or an existing file window to the internal
/// window owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileWindowRequest {
    /// Open a new window even if another window has the same launch target.
    Open(FileLaunch),
    /// Focus a matching window, or open one when no match exists.
    OpenOrFocus(FileLaunch),
    Focus(InternalSurfaceId),
    Close(InternalSurfaceId),
}

/// A compositor operation produced after applying a [`FileWindowRequest`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileWindowAction {
    Opened(InternalSurfaceId),
    Focused(InternalSurfaceId),
    Closed(InternalSurfaceId),
    /// The requested window no longer exists.
    NotFound(InternalSurfaceId),
}

/// Multi-window Nickel File state owned by the compositor process.
#[derive(Default)]
pub struct FileWindowCoordinator {
    surfaces: InternalSurfaceSet,
    launches: BTreeMap<InternalSurfaceId, FileLaunch>,
}

impl FileWindowCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn surfaces(&self) -> &InternalSurfaceSet {
        &self.surfaces
    }

    pub fn surfaces_mut(&mut self) -> &mut InternalSurfaceSet {
        &mut self.surfaces
    }

    pub fn launch(&self, id: InternalSurfaceId) -> Option<&FileLaunch> {
        self.launches.get(&id)
    }

    pub fn application(&self, id: InternalSurfaceId) -> Option<&FileApp> {
        self.surfaces
            .get(id)?
            .application()
            .downcast_ref::<FileApp>()
    }

    pub fn application_mut(&mut self, id: InternalSurfaceId) -> Option<&mut FileApp> {
        self.surfaces
            .get_mut(id)?
            .application_mut()
            .downcast_mut::<FileApp>()
    }

    pub fn handle(&mut self, request: FileWindowRequest) -> FileWindowAction {
        match request {
            FileWindowRequest::Open(launch) => self.open(launch),
            FileWindowRequest::OpenOrFocus(launch) => {
                if let Some(id) = self.matching_window(&launch) {
                    FileWindowAction::Focused(id)
                } else {
                    self.open(launch)
                }
            }
            FileWindowRequest::Focus(id) => {
                if self.launches.contains_key(&id) {
                    FileWindowAction::Focused(id)
                } else {
                    FileWindowAction::NotFound(id)
                }
            }
            FileWindowRequest::Close(id) => self.close(id),
        }
    }

    /// Removes applications that asked their host to close and returns their
    /// surface ids so the compositor can discard placement/focus state too.
    pub fn drain_close_requests(&mut self) -> Vec<InternalSurfaceId> {
        let closing = self
            .launches
            .keys()
            .copied()
            .filter(|id| self.application(*id).is_some_and(FileApp::close_requested))
            .collect::<Vec<_>>();
        for id in &closing {
            self.surfaces.remove(*id);
            self.launches.remove(id);
        }
        closing
    }

    fn matching_window(&self, launch: &FileLaunch) -> Option<InternalSurfaceId> {
        self.launches
            .iter()
            .find_map(|(id, existing)| (existing == launch).then_some(*id))
    }

    fn open(&mut self, launch: FileLaunch) -> FileWindowAction {
        let application = launch.clone().into_app();
        let (width, height) = application.initial_size();
        let id = self
            .surfaces
            .insert_hosted(HostedApplication::new(application, width, height));
        self.launches.insert(id, launch);
        FileWindowAction::Opened(id)
    }

    fn close(&mut self, id: InternalSurfaceId) -> FileWindowAction {
        if self.surfaces.remove(id).is_some() {
            self.launches.remove(&id);
            FileWindowAction::Closed(id)
        } else {
            FileWindowAction::NotFound(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{FileWindowAction, FileWindowCoordinator, FileWindowRequest};
    use crate::FileLaunch;

    #[test]
    fn open_or_focus_reuses_a_matching_internal_window() {
        let directory = tempfile::tempdir().unwrap();
        let launch = FileLaunch::Browse(directory.path().to_path_buf());
        let mut windows = FileWindowCoordinator::new();

        let FileWindowAction::Opened(id) =
            windows.handle(FileWindowRequest::OpenOrFocus(launch.clone()))
        else {
            panic!("first request should open a window");
        };
        assert_eq!(
            windows.handle(FileWindowRequest::OpenOrFocus(launch)),
            FileWindowAction::Focused(id)
        );
        assert_eq!(windows.surfaces().len(), 1);
    }

    #[test]
    fn explicit_open_allows_two_windows_for_the_same_location() {
        let directory = tempfile::tempdir().unwrap();
        let launch = FileLaunch::Browse(directory.path().to_path_buf());
        let mut windows = FileWindowCoordinator::new();

        let first = windows.handle(FileWindowRequest::Open(launch.clone()));
        let second = windows.handle(FileWindowRequest::Open(launch));

        assert_ne!(first, second);
        assert_eq!(windows.surfaces().len(), 2);
    }

    #[test]
    fn close_requests_remove_application_and_surface_together() {
        let directory = tempfile::tempdir().unwrap();
        let mut windows = FileWindowCoordinator::new();
        let FileWindowAction::Opened(id) = windows.handle(FileWindowRequest::Open(
            FileLaunch::Browse(directory.path().to_path_buf()),
        )) else {
            unreachable!()
        };

        windows.application_mut(id).unwrap().request_close();

        assert_eq!(windows.drain_close_requests(), vec![id]);
        assert!(windows.application(id).is_none());
        assert!(windows.surfaces().is_empty());
    }

    #[test]
    fn distinct_launch_modes_do_not_alias() {
        let mut windows = FileWindowCoordinator::new();
        let target = PathBuf::from("/tmp/example.txt");
        let FileWindowAction::Opened(properties) = windows.handle(FileWindowRequest::OpenOrFocus(
            FileLaunch::Properties(target.clone()),
        )) else {
            unreachable!()
        };
        let FileWindowAction::Opened(rename) =
            windows.handle(FileWindowRequest::OpenOrFocus(FileLaunch::Rename(target)))
        else {
            unreachable!()
        };

        assert_ne!(properties, rename);
    }
}
