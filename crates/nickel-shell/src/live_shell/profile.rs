//! Owns the storage lifetime of a shell's package preferences.
//!
//! Embedded previews and test hosts can retain a private directory while using
//! exactly the same validated settings transactions as the persistent desktop.
use nickel_core::plugins::{
    PluginActivationSettings, PluginCatalog, PluginManifest, PluginPreferences,
};
use std::{io, path::PathBuf, sync::Arc};

#[derive(Clone, Default)]
pub struct ShellProfile {
    temporary: Option<Arc<tempfile::TempDir>>,
}
impl ShellProfile {
    pub fn temporary() -> io::Result<Self> {
        Ok(Self {
            temporary: Some(Arc::new(
                tempfile::Builder::new()
                    .prefix("nickel-shell-profile-")
                    .tempdir()?,
            )),
        })
    }
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        self.temporary.as_ref().map_or_else(
            || nickel_storage::config_path(name),
            |root| Ok(root.path().join(name)),
        )
    }
    pub(super) fn catalog(&self) -> Result<PluginCatalog, String> {
        if self.temporary.is_some() {
            Ok(PluginCatalog::default())
        } else {
            PluginCatalog::discover_default().map_err(|error| error.to_string())
        }
    }
    pub(super) fn activation(&self) -> io::Result<PluginActivationSettings> {
        PluginActivationSettings::load(self.path("plugin-activation.json")?)
    }
    pub(super) fn plugin_preferences(
        &self,
        manifest: &PluginManifest,
    ) -> io::Result<PluginPreferences> {
        match &self.temporary {
            Some(root) => {
                PluginPreferences::load(root.path().join(format!("plugin-{}.json", manifest.id)))
            }
            None => PluginPreferences::load_default(manifest),
        }
    }
    pub(super) fn update_plugin_preference(
        &self,
        manifest: &PluginManifest,
        key: &str,
        value: serde_json::Value,
    ) -> io::Result<()> {
        match &self.temporary {
            Some(root) => PluginPreferences::update(
                root.path().join(format!("plugin-{}.json", manifest.id)),
                manifest,
                key,
                value,
            ),
            None => PluginPreferences::update_default(manifest, key, value),
        }
    }
    pub(super) fn update_activation(
        &self,
        manifest: &PluginManifest,
        digest: Option<&str>,
        enabled: bool,
    ) -> io::Result<()> {
        let path = self.path("plugin-activation.json")?;
        match digest {
            Some(digest) => {
                PluginActivationSettings::update_manifest(path, manifest, digest, enabled)
            }
            None => PluginActivationSettings::update(path, &manifest.id, enabled),
        }
    }
    pub(super) fn select_shell(&self, id: &str) -> io::Result<()> {
        PluginActivationSettings::select_shell(self.path("plugin-activation.json")?, id)
    }
    pub(super) fn launcher_preferences_path(&self) -> io::Result<PathBuf> {
        match &self.temporary {
            Some(root) => Ok(root.path().join("launcher-preferences.json")),
            None => nickel_core::launcher_preferences::preferences_path(),
        }
    }
    pub(super) fn launcher_preferences(
        &self,
    ) -> io::Result<nickel_core::launcher_preferences::LauncherPreferences> {
        nickel_core::launcher_preferences::LauncherPreferences::load(
            self.launcher_preferences_path()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_profile_preserves_approval_validation_and_retires_after_last_owner() {
        let profile = ShellProfile::temporary().unwrap();
        let directory = profile.temporary.as_ref().unwrap().path().to_owned();
        let manifest = crate::plugin_panel::manifest();
        let digest = "a".repeat(64);
        profile
            .update_activation(manifest, Some(&digest), true)
            .unwrap();
        let approval = profile.activation().unwrap();
        assert!(approval.approval_current(manifest, &digest));
        assert!(!approval.approval_current(manifest, &"b".repeat(64)));
        let mut changed = manifest.clone();
        changed.capabilities.clear();
        assert!(!approval.approval_current(&changed, &digest));
        let owner = profile.clone();
        drop(profile);
        assert!(directory.is_dir());
        drop(owner);
        assert!(!directory.exists());
    }
}
