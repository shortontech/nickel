use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WindowId(pub u32);

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
}

impl WindowRegistry {
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn insert(&mut self, id: WindowId) {
        self.windows.entry(id).or_insert_with(|| WindowInfo {
            id,
            title: String::new(),
            app_id: String::new(),
            active: false,
        });
        self.raise(id);
        eprintln!("nickel-session: mapped window {}", id.0);
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

    pub fn remove(&mut self, id: WindowId) {
        self.windows.remove(&id);
        self.stacking_order.retain(|candidate| *candidate != id);
        eprintln!("nickel-session: unmapped window {}", id.0);
    }

    #[cfg(test)]
    fn snapshot(&self) -> Vec<&WindowInfo> {
        self.stacking_order
            .iter()
            .filter_map(|id| self.windows.get(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowId, WindowRegistry};

    #[test]
    fn lifecycle_tracks_metadata_focus_and_stacking() {
        let mut registry = WindowRegistry::default();
        registry.insert(WindowId(7));
        registry.update_metadata(
            WindowId(7),
            Some("Terminal".into()),
            Some("terminal".into()),
        );
        registry.insert(WindowId(8));

        let windows = registry.snapshot();
        assert_eq!(
            windows.iter().map(|window| window.id.0).collect::<Vec<_>>(),
            [7, 8]
        );
        assert!(!windows[0].active);
        assert!(windows[1].active);
        assert_eq!(windows[0].title, "Terminal");

        registry.raise(WindowId(7));
        registry.remove(WindowId(8));
        let windows = registry.snapshot();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, WindowId(7));
        assert!(windows[0].active);
        assert_eq!(registry.len(), 1);
    }
}
