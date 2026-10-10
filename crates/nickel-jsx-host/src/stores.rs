use crate::JsxRuntime;
use serde_json::Value;
pub trait DomainStoreRuntimeExt {
    fn set_windows_store(&mut self, snapshot: &Value) -> Result<bool, String>;
    fn set_applications_store(&mut self, snapshot: &Value) -> Result<bool, String>;
    fn set_notifications_store(&mut self, snapshot: &Value) -> Result<bool, String>;
    fn set_workspaces_store(&mut self, snapshot: &Value) -> Result<bool, String>;
    fn set_outputs_store(&mut self, snapshot: &Value) -> Result<bool, String>;
}
impl DomainStoreRuntimeExt for JsxRuntime {
    /// Publish the package owner's already capability-filtered public window
    /// observation. The JavaScript store retains identity for unchanged values
    /// and advances its monotonic generation only for a public change.
    fn set_windows_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        self.invalidate_projection("windows");
        self.call_host_observation("__nickelSetWindowsStore", std::slice::from_ref(snapshot))
    }

    /// Publish the package owner's already capability-filtered application
    /// catalog. Launch and activation remain separately revalidated effects.
    fn set_applications_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        // A separate native publication supersedes the mount's store even
        // when its immutable compatibility inventory has not changed.
        self.invalidate_projection("applications");
        self.call_host_observation(
            "__nickelSetApplicationsStore",
            std::slice::from_ref(snapshot),
        )
    }

    /// Publish the owner's already filtered public notification feed. Invoke
    /// and dismiss effects continue through native authority validation.
    fn set_notifications_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        self.call_host_observation(
            "__nickelSetNotificationsStore",
            std::slice::from_ref(snapshot),
        )
    }

    fn set_workspaces_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        self.call_host_observation("__nickelSetWorkspacesStore", std::slice::from_ref(snapshot))
    }

    fn set_outputs_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        self.call_host_observation("__nickelSetOutputsStore", std::slice::from_ref(snapshot))
    }
}
pub(crate) fn publish_data(
    runtime: &mut JsxRuntime,
    data: &Value,
    reused: &[&str],
) -> Result<(), String> {
    // Window data has already been capability-filtered by the native owner.
    // Publish it through its retained store before replacing the compatibility
    // projection so hooks never inspect an unfiltered host catalog.
    if let Some(windows) = data.get("windows") {
        if reused.contains(&"windows") {
            runtime.validate_host_observation("__nickelValidateWindowsStorePublication")?;
        } else {
            runtime.set_windows_store(windows)?;
        }
    }
    if let Some(window_previews) = data.get("windowPreviews") {
        runtime.call_host_observation(
            "__nickelSetProjectionStore",
            &[
                Value::String("windowPreviews".into()),
                window_previews.clone(),
            ],
        )?;
    }
    if let Some(window_menu) = data.get("windowMenu") {
        runtime.call_host_observation(
            "__nickelSetProjectionStore",
            &[Value::String("windowMenu".into()), window_menu.clone()],
        )?;
    }
    if !reused.contains(&"applications")
        && let Some(applications) = data.get("applications")
        && applications.as_array().is_some_and(|applications| {
            applications.first().is_none_or(|application| {
                application.get("name").is_some()
                    && application.get("icon").is_some()
                    && application.get("kind").is_some()
                    && application.get("launchClass").is_some()
            })
        })
    {
        runtime.set_applications_store(applications)?;
    }
    if let Some(notifications) = data.get("notifications")
        && notifications
            .get("history")
            .and_then(Value::as_array)
            .is_some()
        && notifications
            .get("notification")
            .is_none_or(|notification| {
                notification.is_null()
                    || (notification.get("appName").is_some()
                        && notification.get("body").is_some()
                        && notification.get("actions").is_some())
            })
    {
        runtime.set_notifications_store(notifications)?;
    }
    if let Some(workspaces) = data.get("workspaces") {
        runtime.set_workspaces_store(workspaces)?;
    }
    if let Some(outputs) = data.get("outputs").or_else(|| data.get("displays"))
        && outputs
            .get("outputs")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.first().is_none_or(|output| {
                    output.get("geometry").is_some()
                        && output.get("work_area").is_some()
                        && output.get("modes").is_some()
                })
            })
    {
        runtime.set_outputs_store(outputs)?;
    }
    if let Some(locale) = data.get("locale") {
        runtime.set_locale_store(locale)?;
    }
    // Effective presentation state is globally readable and deliberately
    // separate from the capability-gated appearance configuration client.
    if let Some(appearance) = data.get("appearance") {
        let resolved = appearance.get("resolved").unwrap_or(&Value::Null);
        let configured = appearance.get("configured").unwrap_or(&Value::Null);
        let accent = resolved.get("accent").and_then(|value| {
            let channels = value.as_array()?;
            if channels.len() != 3 {
                return None;
            }
            Some(
                0xff00_0000u64
                    | (channels[0].as_u64()? << 16)
                    | (channels[1].as_u64()? << 8)
                    | channels[2].as_u64()?,
            )
        });
        let animations = configured.get("animations").and_then(Value::as_str);
        let theme = serde_json::json!({
            "mode": resolved.get("theme").and_then(Value::as_str).unwrap_or("unknown"),
            "accent": accent,
            "accentHue": resolved.get("hue").and_then(Value::as_u64),
            "accentIntensity": resolved.get("intensity").and_then(Value::as_u64),
            "reducedMotion": match animations {
                Some("off" | "reduced") => Some(true),
                Some("normal") => Some(false),
                _ => None,
            },
            "reducedTransparency": configured.get("reduce_transparency").and_then(Value::as_bool),
            // The runtime data projection does not currently carry the
            // renderer's semantic palette. Absence is explicit, not guessed.
            "palette": Value::Null,
        });
        runtime.set_theme_store(&theme)?;
    }
    Ok(())
}
