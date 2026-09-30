//! Trusted in-process application admission; shell utility surfaces are never registered.
use std::collections::HashMap;

pub(crate) fn is_application_surface(surface: &nickel_core::plugins::PluginSurface) -> bool {
    surface.kind == nickel_core::plugins::PluginSurfaceKind::Window && !surface.reserve_work_area
}

#[derive(Default)]
pub(crate) struct NativeApplicationWindows {
    applications: HashMap<usize, String>,
}
impl NativeApplicationWindows {
    pub(crate) fn register(&mut self, handle: usize, application: &str) {
        if handle != 0 && !application.is_empty() {
            self.applications.insert(handle, application.to_owned());
        }
    }
    pub(crate) fn unregister(&mut self, handle: usize) {
        self.applications.remove(&handle);
    }
    pub(crate) fn application(&self, handle: usize) -> Option<&str> {
        self.applications.get(&handle).map(String::as_str)
    }
    pub(crate) fn admits(&self, handle: usize, process: u32, own_process: u32) -> bool {
        process != 0 && (process != own_process || self.application(handle).is_some())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_declared_ordinary_window_surfaces_are_application_windows() {
        use nickel_core::plugins::{PluginSurface, PluginSurfaceKind};
        let mut surface: PluginSurface =
            serde_json::from_str(r#"{"id":"main","kind":"window","width":400,"height":300}"#)
                .unwrap();
        assert!(is_application_surface(&surface));
        surface.reserve_work_area = true;
        assert!(!is_application_surface(&surface));
        surface.reserve_work_area = false;
        for kind in [
            PluginSurfaceKind::Panel,
            PluginSurfaceKind::Dock,
            PluginSurfaceKind::Dialog,
            PluginSurfaceKind::Overlay,
        ] {
            surface.kind = kind;
            assert!(!is_application_surface(&surface));
        }
    }
    #[test]
    fn only_registered_native_applications_admit_own_process_handles() {
        let mut windows = NativeApplicationWindows::default();
        assert!(!windows.admits(71, 4, 4));
        windows.register(71, "nickel-codex.project.abc");
        assert!(windows.admits(71, 4, 4));
        assert_eq!(windows.application(71), Some("nickel-codex.project.abc"));
        assert!(!windows.admits(72, 4, 4)); // protected shell utility
        assert!(windows.admits(72, 5, 4));
        assert!(!windows.admits(71, 0, 4));
        windows.unregister(71);
        assert!(!windows.admits(71, 4, 4)); // destroyed/reused native handle
        assert_eq!(windows.application(71), None);
    }
}
