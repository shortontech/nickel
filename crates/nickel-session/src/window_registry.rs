use std::collections::BTreeMap;

/// Maximum UTF-8 bytes retained for one client-controlled window title.
pub const MAX_WINDOW_TITLE_BYTES: usize = 4_096;
/// Maximum UTF-8 bytes retained for one client-controlled application identity.
pub const MAX_WINDOW_APP_ID_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WindowMetadataDiagnostics {
    pub entries: usize,
    pub title_bytes: usize,
    pub peak_title_bytes: usize,
    pub app_id_bytes: usize,
    pub peak_app_id_bytes: usize,
    pub truncations: u64,
    pub updates: u64,
    pub snapshot_bytes: usize,
    pub peak_snapshot_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub active: bool,
}

#[derive(Debug, Default)]
pub struct WindowRegistry {
    windows: BTreeMap<WindowId, WindowInfo>,
    stacking_order: Vec<WindowId>,
    next_id: u64,
    metadata_diagnostics: WindowMetadataDiagnostics,
}

impl WindowRegistry {
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn insert(&mut self) -> WindowId {
        let id = self.insert_inactive();
        self.set_active(id);
        id
    }

    pub fn insert_inactive(&mut self) -> WindowId {
        self.next_id += 1;
        let id = WindowId(self.next_id);
        self.windows.insert(
            id,
            WindowInfo {
                id,
                title: String::new(),
                app_id: String::new(),
                active: false,
            },
        );
        self.stacking_order.push(id);
        eprintln!("nickel-session: mapped window {}", id.0);
        id
    }

    pub fn update_metadata(&mut self, id: WindowId, title: Option<String>, app_id: Option<String>) {
        if let Some(window) = self.windows.get_mut(&id) {
            if let Some(title) = title {
                self.metadata_diagnostics.title_bytes = self
                    .metadata_diagnostics
                    .title_bytes
                    .saturating_sub(window.title.len());
                let (title, truncated) = bounded_utf8(title, MAX_WINDOW_TITLE_BYTES);
                self.metadata_diagnostics.title_bytes += title.len();
                self.metadata_diagnostics.truncations += u64::from(truncated);
                window.title = title;
            }
            if let Some(app_id) = app_id {
                self.metadata_diagnostics.app_id_bytes = self
                    .metadata_diagnostics
                    .app_id_bytes
                    .saturating_sub(window.app_id.len());
                let (app_id, truncated) = bounded_utf8(app_id, MAX_WINDOW_APP_ID_BYTES);
                self.metadata_diagnostics.app_id_bytes += app_id.len();
                self.metadata_diagnostics.truncations += u64::from(truncated);
                window.app_id = app_id;
            }
            self.metadata_diagnostics.updates += 1;
            self.metadata_diagnostics.peak_title_bytes = self
                .metadata_diagnostics
                .peak_title_bytes
                .max(self.metadata_diagnostics.title_bytes);
            self.metadata_diagnostics.peak_app_id_bytes = self
                .metadata_diagnostics
                .peak_app_id_bytes
                .max(self.metadata_diagnostics.app_id_bytes);
            eprintln!(
                "nickel-session: window {} [{}] {}",
                id.0, window.app_id, window.title
            );
        }
    }

    pub fn raise(&mut self, id: WindowId) {
        self.stacking_order.retain(|candidate| *candidate != id);
        self.stacking_order.push(id);
        for window in self.windows.values_mut() {
            window.active = window.id == id;
        }
    }

    pub fn deactivate_all(&mut self) {
        for window in self.windows.values_mut() {
            window.active = false;
        }
    }

    pub fn set_active(&mut self, id: WindowId) {
        for window in self.windows.values_mut() {
            window.active = window.id == id;
        }
    }

    pub fn remove(&mut self, id: WindowId) {
        if let Some(window) = self.windows.remove(&id) {
            self.metadata_diagnostics.title_bytes = self
                .metadata_diagnostics
                .title_bytes
                .saturating_sub(window.title.len());
            self.metadata_diagnostics.app_id_bytes = self
                .metadata_diagnostics
                .app_id_bytes
                .saturating_sub(window.app_id.len());
        }
        self.stacking_order.retain(|candidate| *candidate != id);
        eprintln!("nickel-session: unmapped window {}", id.0);
    }

    pub fn snapshot(&self) -> Vec<&WindowInfo> {
        self.stacking_order
            .iter()
            .filter_map(|id| self.windows.get(id))
            .collect()
    }

    pub fn is_active(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.active)
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.contains_key(&id)
    }

    pub fn title(&self, id: WindowId) -> Option<&str> {
        self.windows.get(&id).map(|window| window.title.as_str())
    }

    pub fn app_id(&self, id: WindowId) -> Option<&str> {
        self.windows.get(&id).map(|window| window.app_id.as_str())
    }

    pub fn metadata_diagnostics(&self) -> WindowMetadataDiagnostics {
        WindowMetadataDiagnostics {
            entries: self.windows.len(),
            ..self.metadata_diagnostics
        }
    }

    /// Record the owned UTF-8 payload of one request-scoped protocol projection.
    /// The projection itself remains owned by the synchronous response and is
    /// not retained by the registry.
    pub fn record_snapshot_bytes(&mut self, bytes: usize) {
        self.metadata_diagnostics.snapshot_bytes = bytes;
        self.metadata_diagnostics.peak_snapshot_bytes =
            self.metadata_diagnostics.peak_snapshot_bytes.max(bytes);
    }

    #[cfg(test)]
    fn test_snapshot(&self) -> Vec<&WindowInfo> {
        self.snapshot()
    }
}

fn bounded_utf8(mut value: String, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        value.shrink_to_fit();
        return (value, false);
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.shrink_to_fit();
    (value, true)
}

#[cfg(test)]
mod tests {
    use super::{MAX_WINDOW_APP_ID_BYTES, MAX_WINDOW_TITLE_BYTES, WindowRegistry, bounded_utf8};

    #[test]
    fn lifecycle_tracks_metadata_focus_and_stacking() {
        let mut registry = WindowRegistry::default();
        let terminal = registry.insert();
        registry.update_metadata(terminal, Some("Terminal".into()), Some("terminal".into()));
        let browser = registry.insert();

        let windows = registry.test_snapshot();
        assert_eq!(
            windows.iter().map(|window| window.id.0).collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(!windows[0].active);
        assert!(windows[1].active);
        assert_eq!(windows[0].title, "Terminal");

        registry.set_active(terminal);
        let windows = registry.test_snapshot();
        assert_eq!(
            windows.iter().map(|window| window.id.0).collect::<Vec<_>>(),
            [1, 2],
            "active-state repair must not rewrite stacking order"
        );
        assert!(windows[0].active);
        assert!(!windows[1].active);

        registry.raise(terminal);
        registry.remove(browser);
        let windows = registry.test_snapshot();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, terminal);
        assert!(windows[0].active);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn inactive_insertion_preserves_existing_activation() {
        let mut registry = WindowRegistry::default();
        let focused = registry.insert();
        let background = registry.insert_inactive();
        assert!(registry.is_active(focused));
        assert!(!registry.is_active(background));
        assert_eq!(
            registry
                .test_snapshot()
                .into_iter()
                .map(|window| window.id)
                .collect::<Vec<_>>(),
            [focused, background]
        );
    }

    #[test]
    fn metadata_is_utf8_bounded_and_replacement_does_not_follow_history() {
        let mut registry = WindowRegistry::default();
        let window = registry.insert();
        let oversized_title = "🙂".repeat(MAX_WINDOW_TITLE_BYTES);
        let oversized_app_id = "é".repeat(MAX_WINDOW_APP_ID_BYTES);

        for _ in 0..32 {
            registry.update_metadata(
                window,
                Some(oversized_title.clone()),
                Some(oversized_app_id.clone()),
            );
        }

        let title = registry.title(window).unwrap();
        let app_id = registry.app_id(window).unwrap();
        assert!(title.len() <= MAX_WINDOW_TITLE_BYTES);
        assert!(app_id.len() <= MAX_WINDOW_APP_ID_BYTES);
        assert!(std::str::from_utf8(title.as_bytes()).is_ok());
        assert!(std::str::from_utf8(app_id.as_bytes()).is_ok());
        let diagnostics = registry.metadata_diagnostics();
        assert_eq!(diagnostics.title_bytes, title.len());
        assert_eq!(diagnostics.app_id_bytes, app_id.len());
        assert_eq!(diagnostics.peak_title_bytes, title.len());
        assert_eq!(diagnostics.peak_app_id_bytes, app_id.len());
        assert_eq!(diagnostics.truncations, 64);
        assert_eq!(diagnostics.updates, 32);
        assert_eq!(registry.windows[&window].title.capacity(), title.len());
        assert_eq!(registry.windows[&window].app_id.capacity(), app_id.len());

        registry.record_snapshot_bytes(title.len() + app_id.len());
        let diagnostics = registry.metadata_diagnostics();
        assert_eq!(
            diagnostics.snapshot_bytes,
            diagnostics.title_bytes + diagnostics.app_id_bytes
        );
        assert_eq!(diagnostics.peak_snapshot_bytes, diagnostics.snapshot_bytes);

        registry.remove(window);
        let diagnostics = registry.metadata_diagnostics();
        assert_eq!(diagnostics.entries, 0);
        assert_eq!(diagnostics.title_bytes, 0);
        assert_eq!(diagnostics.app_id_bytes, 0);
    }

    #[test]
    fn metadata_boundaries_preserve_unicode_and_empty_values() {
        for value in ["", "ordinary", "e\u{301}", "🙂"] {
            assert_eq!(
                bounded_utf8(value.to_owned(), value.len()),
                (value.to_owned(), false)
            );
        }
        assert_eq!(bounded_utf8("🙂🙂".to_owned(), 7), ("🙂".to_owned(), true));
        assert_eq!(
            bounded_utf8("abcdef".to_owned(), 6),
            ("abcdef".to_owned(), false)
        );
    }

    #[test]
    fn maximum_window_population_has_a_declared_metadata_ceiling() {
        let mut registry = WindowRegistry::default();
        for _ in 0..nickel_session_protocol::MAX_WINDOWS {
            let window = registry.insert_inactive();
            registry.update_metadata(
                window,
                Some("x".repeat(MAX_WINDOW_TITLE_BYTES * 2)),
                Some("y".repeat(MAX_WINDOW_APP_ID_BYTES * 2)),
            );
        }
        let diagnostics = registry.metadata_diagnostics();
        assert_eq!(diagnostics.entries, nickel_session_protocol::MAX_WINDOWS);
        assert_eq!(
            diagnostics.title_bytes,
            nickel_session_protocol::MAX_WINDOWS * MAX_WINDOW_TITLE_BYTES
        );
        assert_eq!(
            diagnostics.app_id_bytes,
            nickel_session_protocol::MAX_WINDOWS * MAX_WINDOW_APP_ID_BYTES
        );
    }
}
