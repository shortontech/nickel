//! Nickel capability schema and host-owned availability projection.
use crate::{JsxModuleGraph, JsxRuntime};
use serde_json::Value;
pub const PLUGIN_CAPABILITY_NAMES: &[&str] = &[
    "launcher-show",
    "control-center-show",
    "on-screen-keyboard-show",
    "on-screen-keyboard-read",
    "on-screen-keyboard-input",
    "applications-read",
    "applications-launch",
    "applications-pin",
    "associations-read",
    "associations-control",
    "plugins-read",
    "plugins-control",
    "features-read",
    "features-control",
    "shortcuts-read",
    "preferences-read",
    "preferences-control",
    "windows-read",
    "windows-focus",
    "windows-context",
    "desktop-read",
    "desktop-arrange",
    "desktop-files-open",
    "desktop-files-manage",
    "tray-read",
    "tray-activate",
    "tray-context",
    "appearance-read",
    "appearance-control",
    "wallpaper-read",
    "wallpaper-control",
    "audio-read",
    "audio-control",
    "network-read",
    "network-control",
    "bluetooth-read",
    "bluetooth-control",
    "desktop-control",
    "display-control",
    "session-control",
    "workspaces-read",
    "workspaces-switch",
    "notifications-read",
    "notifications-act",
    "settings-read",
    "settings-write",
    "settings-show",
    "projects-menu-show",
    "session-logout-request",
    "run-command",
];
pub trait CapabilityRuntimeExt {
    fn set_capability_store(
        &mut self,
        declared: &[nickel_core::plugins::PluginCapability],
        data: &Value,
    ) -> Result<bool, String>;
}
impl CapabilityRuntimeExt for JsxRuntime {
    /// Publish the current owner's declared grants and bounded runtime
    /// availability. This observation never participates in action authority.
    fn set_capability_store(
        &mut self,
        declared: &[nickel_core::plugins::PluginCapability],
        data: &Value,
    ) -> Result<bool, String> {
        let resource = |name: &str| -> Option<&Value> {
            let field = match name {
                "on-screen-keyboard-read" | "on-screen-keyboard-input" => "keyboard",
                "applications-read" | "applications-launch" | "applications-pin" => "applications",
                "associations-read" | "associations-control" => "associations",
                "plugins-read" | "plugins-control" => "plugins",
                "features-read" | "features-control" => "features",
                "shortcuts-read" => "shortcuts",
                "preferences-read" | "preferences-control" => "preferences",
                "windows-read" | "windows-focus" | "windows-context" => "windows",
                "desktop-read"
                | "desktop-arrange"
                | "desktop-files-open"
                | "desktop-files-manage"
                | "desktop-control" => "desktop",
                "tray-read" | "tray-activate" | "tray-context" => "tray",
                "appearance-read" | "appearance-control" => "appearance",
                "wallpaper-read" | "wallpaper-control" => "wallpaper",
                "audio-read" | "audio-control" => "audio",
                "network-read" | "network-control" => "wifi",
                "bluetooth-read" | "bluetooth-control" => "bluetooth",
                "display-control" => "displays",
                "session-control" | "session-logout-request" => "session",
                "workspaces-read" | "workspaces-switch" => "workspaces",
                "notifications-read" | "notifications-act" => "notifications",
                "settings-read" | "settings-write" | "settings-show" => "navigation",
                "run-command" => "run",
                _ => return None,
            };
            data.get(field)
        };
        let mut entries = serde_json::Map::new();
        for &name in PLUGIN_CAPABILITY_NAMES {
            let declared = declared
                .iter()
                .any(|capability| capability.as_str() == name);
            let (available, reason) = if !declared {
                (Some(false), None)
            } else if let Some(snapshot) = resource(name) {
                let available = snapshot
                    .get("available")
                    .and_then(Value::as_bool)
                    .or(Some(true));
                let reason = snapshot
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|reason| reason.chars().take(480).collect::<String>());
                (available, reason)
            } else {
                (None, None)
            };
            entries.insert(
                name.into(),
                serde_json::json!({"declared":declared,"available":available,"reason":reason}),
            );
        }
        let value = serde_json::json!({"known":PLUGIN_CAPABILITY_NAMES,"entries":entries});
        self.set_capability_snapshot(&value)
    }
}

/// Initializes Nickel's observation vocabulary before evaluating a package.
/// Empty declared grants remain denied; admission and action authority stay in Nickel.
pub fn create_runtime(source: &str, data: Option<&str>) -> Result<JsxRuntime, String> {
    let bindings = crate::HostBindings::new(
        crate::HostBindings::ABI_VERSION,
        include_str!("bootstrap.js"),
        crate::stores::publish_data,
        &["windows", "applications"],
    )?;
    let mut runtime = JsxRuntime::new_with_bindings("", data, &bindings)?;
    runtime.set_capability_store(&[], &serde_json::json!({}))?;
    runtime.eval(source)?;
    Ok(runtime)
}
pub fn create_module_runtime(
    graph: &JsxModuleGraph,
    data: Option<&str>,
) -> Result<JsxRuntime, String> {
    create_runtime(&graph.compile()?, data)
}
