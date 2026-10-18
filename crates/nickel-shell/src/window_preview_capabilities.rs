//! Bounded native preview feed shared by authorized ordinary packages.
//! Presentation, labels and card dimensions belong to the package.
use crate::model::{WindowGroup, WindowId};

pub(crate) fn snapshot(
    group: Option<&WindowGroup>,
    switcher: bool,
    selected: Option<WindowId>,
    generation: u64,
    preview_generation: u64,
) -> serde_json::Value {
    let limit = if switcher { 5 } else { 12 };
    let windows = group.map(|group| group.windows.iter().take(limit).map(|window| serde_json::json!({
            "id":window.id.0.to_string(), "title":window.title.chars().take(120).collect::<String>(),
            "applicationName":group.application_name.chars().take(120).collect::<String>(),
            "selected":switcher && selected == Some(window.id), "canClose":window.state.capabilities.close,
            "canActivate":window.state.capabilities.activate,
            "image":format!("window:{}",window.id.0)
        })).collect::<Vec<_>>()).unwrap_or_default();
    // The token includes current native identities, selection and package lifecycle.
    use std::hash::{Hash, Hasher};
    let mut revision = std::collections::hash_map::DefaultHasher::new();
    generation.hash(&mut revision);
    preview_generation.hash(&mut revision);
    switcher.hash(&mut revision);
    if let Some(group) = group {
        for window in group.windows.iter().take(limit) {
            window
                .application_id
                .as_ref()
                .map(|id| id.as_str())
                .hash(&mut revision);
        }
    }
    serde_json::Value::Array(windows.clone())
        .to_string()
        .hash(&mut revision);
    serde_json::json!({"available":true,"open":group.is_some(),"taskSwitcher":switcher,
            "revision":format!("{:x}",revision.finish()),"windows":windows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OpenWindow;
    #[test]
    fn preview_feed_bounds_windows_text_and_revisions() {
        let group = WindowGroup {
            application_id: None,
            application_name: "x".repeat(200),
            windows: (1..=30)
                .map(|id| OpenWindow {
                    id: WindowId(id),
                    application_id: None,
                    active: false,
                    title: "x".repeat(200),
                    state: Default::default(),
                })
                .collect(),
        };
        let hover = snapshot(Some(&group), false, None, 1, 1);
        let flip = snapshot(Some(&group), true, Some(WindowId(2)), 1, 1);
        assert_eq!(hover["windows"].as_array().unwrap().len(), 12);
        assert_eq!(flip["windows"].as_array().unwrap().len(), 5);
        assert_eq!(hover["windows"][0]["title"].as_str().unwrap().len(), 120);
        assert_eq!(flip["windows"][1]["selected"], true);
        assert_ne!(hover["revision"], flip["revision"]);
        assert_ne!(
            flip["revision"],
            snapshot(Some(&group), true, Some(WindowId(2)), 1, 2)["revision"]
        );
        assert_ne!(
            flip["revision"],
            snapshot(Some(&group), true, Some(WindowId(3)), 1, 1)["revision"]
        );
        assert_ne!(
            flip["revision"],
            snapshot(Some(&group), true, Some(WindowId(2)), 2, 1)["revision"]
        );
        assert_eq!(
            snapshot(None, false, None, 1, 1)["windows"],
            serde_json::json!([])
        );
        assert!(hover["windows"][0].get("imageWidth").is_none());
    }
}
