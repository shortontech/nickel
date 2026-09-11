//! Typed wallpaper preferences without image paths or image contents.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Center,
    Tile,
    Stretch,
    Fit,
    Span,
    Fill,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// Whether Nickel has a custom image configured. Its path is never disclosed.
    pub custom_image_configured: bool,
    pub position: Position,
}

/// An opaque entry from the bounded, owner-built wallpaper catalog. The ID is
/// meaningful only with this snapshot generation and never contains a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageChoice {
    pub id: String,
    pub configured: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    SetPosition { position: Position },
    ResetCustomImage {},
    SelectApprovedImage { image_id: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Preferences,
    pub change: Change,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured: Preferences,
    /// Bounded choices discovered below Nickel's approved image directory or
    /// the platform's installed-wallpaper roots. Paths are never returned.
    pub images: Vec<ImageChoice>,
    /// True only when this result follows a successful bounded full decode of
    /// the selected source before commit. It is not presentation confirmation.
    pub selected_image_decoded: bool,
    /// The live shell was notified to reload; this is not pixel confirmation.
    pub runtime_reload_requested: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_rejects_paths_contents_and_unknown_changes() {
        let base = serde_json::json!({
            "generation": 1,
            "prior": {"custom_image_configured": true, "position": "fill"},
            "change": {"kind": "reset_custom_image"}
        });
        assert!(serde_json::from_value::<Transaction>(base.clone()).is_ok());
        let mut path = base.clone();
        path["change"]["path"] = serde_json::json!("/etc/passwd");
        assert!(serde_json::from_value::<Transaction>(path).is_err());
        let mut contents = base.clone();
        contents["prior"]["image_contents"] = serde_json::json!("secret");
        assert!(serde_json::from_value::<Transaction>(contents).is_err());
        let mut select = base;
        select["change"] = serde_json::json!({"kind": "select_image", "path": "/tmp/x"});
        assert!(serde_json::from_value::<Transaction>(select).is_err());

        let selected = serde_json::json!({
            "generation": 1,
            "prior": {"custom_image_configured": true, "position": "fill"},
            "change": {"kind": "select_approved_image", "image_id": "opaque-id"}
        });
        assert!(serde_json::from_value::<Transaction>(selected.clone()).is_ok());
        let mut smuggled = selected;
        smuggled["change"]["path"] = serde_json::json!("/etc/passwd");
        assert!(serde_json::from_value::<Transaction>(smuggled).is_err());
    }
}
