//! Read-only shell shortcut reference; Nickel has no shortcut remapping preferences yet.
use serde_json::{Value, json};

pub(crate) fn snapshot(global_available: bool, reason: Option<&str>) -> Value {
    json!({"available":true,"editable":false,"reason":"Shortcut remapping is not supported.",
    "globalAvailable":global_available,"globalReason":reason.map(|reason|reason.chars().take(256).collect::<String>()),
    "shortcuts":[
        {"id":"launcher","action":"Open launcher","keys":"Super","scope":"global","available":global_available},
        {"id":"search","action":"Search applications","keys":"Type while the launcher is open","scope":"launcher","available":true},
        {"id":"navigate","action":"Navigate controls","keys":"Arrow keys · Tab · Shift+Tab","scope":"focused window","available":true},
        {"id":"activate","action":"Activate selection","keys":"Enter","scope":"focused window","available":true},
        {"id":"back","action":"Go back or close","keys":"Escape","scope":"focused window","available":true},
        {"id":"workspaces","action":"Switch workspaces","keys":"Ctrl+Alt+Left/Right · Ctrl+Alt+0–9 · Super+Ctrl+Left/Right","scope":"global","available":global_available&&cfg!(target_os="linux")}
    ]})
}
#[cfg(test)]
mod tests {
    #[test]
    fn shortcut_reference_does_not_claim_remapping_or_unavailable_global_registration() {
        let snapshot = super::snapshot(false, Some("No shortcut owner"));
        assert_eq!(snapshot["editable"], false);
        assert_eq!(snapshot["shortcuts"][0]["available"], false);
        assert_eq!(snapshot["shortcuts"][1]["available"], true);
    }
}
