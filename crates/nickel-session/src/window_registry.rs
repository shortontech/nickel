use std::collections::BTreeMap;

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
}

impl WindowRegistry {
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn insert(&mut self) -> WindowId {
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
        self.raise(id);
        eprintln!("nickel-session: mapped window {}", id.0);
        id
    }

    pub fn update_metadata(&mut self, id: WindowId, title: Option<String>, app_id: Option<String>) {
        if let Some(window) = self.windows.get_mut(&id) {
            if let Some(title) = title {
                window.title = title;
            }
            if let Some(app_id) = app_id {
                window.app_id = app_id;
            }
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

    pub fn remove(&mut self, id: WindowId) {
        self.windows.remove(&id);
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

    pub fn title(&self, id: WindowId) -> Option<&str> {
        self.windows.get(&id).map(|window| window.title.as_str())
    }

    pub fn app_id(&self, id: WindowId) -> Option<&str> {
        self.windows.get(&id).map(|window| window.app_id.as_str())
    }

    #[cfg(test)]
    fn test_snapshot(&self) -> Vec<&WindowInfo> {
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::WindowRegistry;

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

        registry.raise(terminal);
        registry.remove(browser);
        let windows = registry.test_snapshot();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, terminal);
        assert!(windows[0].active);
        assert_eq!(registry.len(), 1);
    }
}
