//! Shared appearance and wallpaper capability authority, independent of presentation.
use crate::{appearance_service as appearance, wallpaper_service as wallpaper};
use nickel_core::{
    plugins::PluginCapability, shell_settings::ShellSettings, wallpaper_settings::WallpaperSettings,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(crate) fn system_appearance() -> nickel_core::theme::Appearance {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        nickel_platform::appearance()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        nickel_core::theme::Appearance::default()
    }
}

fn resolved_appearance(settings: &ShellSettings, system: nickel_core::theme::Appearance) -> Value {
    let resolved = settings.resolve_appearance(system);
    json!({"theme": match resolved.mode { nickel_core::theme::ThemeMode::Light => "light", nickel_core::theme::ThemeMode::Dark => "dark" }, "hue": settings.displayed_hue(system), "intensity": settings.displayed_intensity(system), "accent": resolved.accent})
}

#[derive(Clone, Debug, PartialEq)]
pub enum AppearanceEffect {
    Appearance {
        generation: u64,
        prior: appearance::Preferences,
        requested: appearance::Preferences,
    },
    ChooseImage {
        generation: u64,
        prior: wallpaper::Preferences,
    },
    Wallpaper {
        generation: u64,
        prior: wallpaper::Preferences,
        change: wallpaper::Change,
    },
}

impl AppearanceEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let object = value.as_object().ok_or("invalid appearance operation")?;
        if object.len() != 2 || !object.contains_key("transaction") {
            return Err("unknown appearance operation fields".into());
        }
        match value["type"].as_str() {
            Some("appearance.set") => {
                let transaction: appearance::Transaction =
                    serde_json::from_value(value["transaction"].clone())
                        .map_err(|_| "invalid appearance transaction")?;
                if transaction.generation == 0
                    || !transaction.prior.valid()
                    || !transaction.requested.valid()
                {
                    return Err("appearance value is outside its supported range".into());
                }
                Ok(Self::Appearance {
                    generation: transaction.generation,
                    prior: transaction.prior,
                    requested: transaction.requested,
                })
            }
            Some("wallpaper.chooseImage") => {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Request {
                    generation: u64,
                    prior: wallpaper::Preferences,
                }
                let request: Request = serde_json::from_value(value["transaction"].clone())
                    .map_err(|_| "invalid native chooser request")?;
                if request.generation == 0 {
                    return Err("wallpaper observation is unavailable".into());
                }
                Ok(Self::ChooseImage {
                    generation: request.generation,
                    prior: request.prior,
                })
            }
            Some("wallpaper.change") => {
                let transaction: wallpaper::Transaction =
                    serde_json::from_value(value["transaction"].clone())
                        .map_err(|_| "invalid wallpaper transaction")?;
                if transaction.generation == 0 {
                    return Err("wallpaper observation is unavailable".into());
                }
                if let wallpaper::Change::SelectApprovedImage { image_id } = &transaction.change {
                    if image_id.is_empty() || image_id.len() > 512 {
                        return Err("invalid wallpaper identity".into());
                    }
                }
                Ok(Self::Wallpaper {
                    generation: transaction.generation,
                    prior: transaction.prior,
                    change: transaction.change,
                })
            }
            _ => Err("unsupported appearance operation".into()),
        }
    }
    pub(crate) fn resource(&self) -> &'static str {
        match self {
            Self::Appearance { .. } => "appearance",
            Self::Wallpaper { .. } | Self::ChooseImage { .. } => "wallpaper",
        }
    }
    pub(crate) fn capability(&self) -> PluginCapability {
        match self {
            Self::Appearance { .. } => PluginCapability::AppearanceControl,
            Self::Wallpaper { .. } | Self::ChooseImage { .. } => PluginCapability::WallpaperControl,
        }
    }
    pub(crate) fn read_capability(&self) -> PluginCapability {
        match self {
            Self::Appearance { .. } => PluginCapability::AppearanceRead,
            Self::Wallpaper { .. } | Self::ChooseImage { .. } => PluginCapability::WallpaperRead,
        }
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        let (generation, prior) = match self {
            Self::Appearance {
                generation, prior, ..
            } => (*generation, serde_json::to_value(prior).unwrap()),
            Self::Wallpaper {
                generation, prior, ..
            }
            | Self::ChooseImage { generation, prior } => {
                (*generation, serde_json::to_value(prior).unwrap())
            }
        };
        if snapshot["available"] != true
            || snapshot["generation"].as_u64() != Some(generation)
            || snapshot["configured"] != prior
        {
            return Err("appearance or wallpaper observation is stale".into());
        }
        if let Self::Wallpaper {
            change: wallpaper::Change::SelectApprovedImage { image_id },
            ..
        } = self
        {
            if !snapshot["images"].as_array().is_some_and(|images| {
                images
                    .iter()
                    .any(|image| image["id"].as_str() == Some(image_id))
            }) {
                return Err("wallpaper identity is stale".into());
            }
        }
        Ok(())
    }
}

pub(crate) enum CommittedAppearance {
    Appearance(ShellSettings),
    Wallpaper(WallpaperSettings),
}

pub(crate) struct AppearanceCapabilities {
    started: Instant,
    appearance: appearance::AppearanceState,
    wallpaper: wallpaper::WallpaperState,
    appearance_snapshot: Option<Value>,
    wallpaper_snapshot: Option<Value>,
    pub(crate) wallpaper_images: crate::plugin_panel::PluginImages,
    preview_revisions: Option<Vec<crate::wallpaper_selection::CandidateRevision>>,
}
impl Default for AppearanceCapabilities {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            appearance: Default::default(),
            wallpaper: Default::default(),
            appearance_snapshot: None,
            wallpaper_snapshot: None,
            wallpaper_images: Default::default(),
            preview_revisions: None,
        }
    }
}
impl AppearanceCapabilities {
    pub(crate) fn refresh(&mut self, resource: &str) -> Value {
        let observed_at_us = self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        let result = if resource == "appearance" {
            appearance::PreparedRead::prepare().and_then(|read| {
                let resolved = resolved_appearance(&read.settings, system_appearance());
                let snapshot = self.appearance.snapshot(&read, observed_at_us)?;
                let mut value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
                value["resolved"] = resolved;
                Ok(value)
            })
        } else {
            wallpaper::PreparedRead::prepare_with_check(|| Ok(())).and_then(|read| {
                let snapshot = self.wallpaper.observe(&read, observed_at_us)?;
                let revisions = read.catalog.revisions();
                if self.preview_revisions.as_ref() != Some(&revisions) {
                    self.wallpaper_images = read.catalog.previews();
                    self.preview_revisions = Some(revisions);
                }
                let labels = read.catalog.presentation();
                let mut value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
                for image in value["images"].as_array_mut().into_iter().flatten() {
                    let id = image["id"].as_str().unwrap_or_default().to_owned();
                    image["label"] = labels
                        .iter()
                        .find(|(key, _)| key == &id)
                        .map(|(_, label)| label.clone())
                        .unwrap_or_else(|| "Wallpaper".into())
                        .into();
                    let asset = format!("wallpaper:{id}");
                    if self.wallpaper_images.contains_key(&asset) {
                        image["previewAsset"] = asset.into();
                    }
                }
                Ok(value)
            })
        };
        let mut value = result.unwrap_or_else(|reason| json!({"available":false,"reason":reason}));
        if value.get("generation").is_some() {
            value["available"] = true.into();
        }
        if resource == "appearance" {
            self.appearance_snapshot = Some(value.clone());
        } else {
            self.wallpaper_snapshot = Some(value.clone());
        }
        value
    }
    pub(crate) fn snapshot(&mut self, resource: &str) -> Value {
        let cached = if resource == "appearance" {
            &self.appearance_snapshot
        } else {
            &self.wallpaper_snapshot
        };
        cached.clone().unwrap_or_else(|| self.refresh(resource))
    }
    pub(crate) fn refresh_observed(&mut self) -> bool {
        let mut changed = false;
        for resource in ["appearance", "wallpaper"] {
            let previous = if resource == "appearance" {
                self.appearance_snapshot.clone()
            } else {
                self.wallpaper_snapshot.clone()
            };
            if let Some(previous) = previous {
                let next = self.refresh(resource);
                changed |= next["generation"] != previous["generation"]
                    || next["available"] != previous["available"]
                    || next["resolved"] != previous["resolved"];
            }
        }
        changed
    }
    pub(crate) fn wallpaper_reconciled(&mut self) {
        if let Some(snapshot) = &mut self.wallpaper_snapshot {
            snapshot["runtime_reload_requested"] = true.into();
        }
    }

    pub(crate) fn commit_chosen(
        &mut self,
        effect: &AppearanceEffect,
        path: std::path::PathBuf,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<WallpaperSettings, String> {
        check()?;
        effect.validate(&self.refresh("wallpaper"))?;
        let AppearanceEffect::ChooseImage { generation, prior } = effect else {
            return Err("invalid chooser request".into());
        };
        let transaction = wallpaper::Transaction {
            generation: *generation,
            prior: prior.clone(),
            change: wallpaper::Change::ResetCustomImage {},
        };
        let read = wallpaper::PreparedRead::prepare_with_check(&mut check)?;
        let prepared = wallpaper::PreparedChange::chosen(read, &transaction, path, &mut check)?;
        let settings = self.wallpaper.commit(prepared, &transaction, || {
            check().map_err(std::io::Error::other)
        })?;
        self.refresh("wallpaper");
        if let Some(snapshot) = &mut self.wallpaper_snapshot {
            snapshot["selected_image_decoded"] = true.into();
        }
        Ok(settings)
    }

    pub(crate) fn execute(
        &mut self,
        effect: &AppearanceEffect,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<CommittedAppearance, String> {
        check()?;
        effect.validate(&self.refresh(effect.resource()))?;
        let committed = match effect {
            AppearanceEffect::ChooseImage { .. } => {
                return Err("native chooser requires asynchronous completion".into());
            }
            AppearanceEffect::Appearance {
                generation,
                prior,
                requested,
            } => {
                let transaction = appearance::Transaction {
                    generation: *generation,
                    prior: prior.clone(),
                    requested: requested.clone(),
                };
                let prepared = appearance::PreparedChange::prepare(&transaction)?;
                let (settings, _revision) = self.appearance.commit(
                    prepared,
                    &transaction,
                    Instant::now() + Duration::from_secs(2),
                    check,
                )?;
                // Reconcile accepted settings even when the post-rename metadata read fails.
                CommittedAppearance::Appearance(settings)
            }
            AppearanceEffect::Wallpaper {
                generation,
                prior,
                change,
            } => {
                let transaction = wallpaper::Transaction {
                    generation: *generation,
                    prior: prior.clone(),
                    change: change.clone(),
                };
                let prepared =
                    wallpaper::PreparedChange::prepare_with_check(&transaction, &mut check)?;
                let settings = self.wallpaper.commit(prepared, &transaction, || {
                    check().map_err(std::io::Error::other)
                })?;
                CommittedAppearance::Wallpaper(settings)
            }
        };
        self.refresh(effect.resource());
        if let AppearanceEffect::Wallpaper { change, .. } = effect {
            if let Some(snapshot) = &mut self.wallpaper_snapshot {
                snapshot["selected_image_decoded"] =
                    matches!(change, wallpaper::Change::SelectApprovedImage { .. }).into();
            }
        }
        Ok(committed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn appearance_request() -> Value {
        json!({"type":"appearance.set","transaction":{"generation":1,"prior":{"theme":"system","accent_hue":null,"accent_intensity":null,"reduce_transparency":false,"animations":"normal"},"requested":{"theme":"dark","accent_hue":359,"accent_intensity":0,"reduce_transparency":true,"animations":"reduced"}}})
    }
    #[test]
    fn native_chooser_boundary_rejects_paths_and_stale_observations() {
        let value = json!({"type":"wallpaper.chooseImage","transaction":{"generation":7,"prior":{"custom_image_configured":false,"position":"fill"}}});
        let effect = AppearanceEffect::parse(&value).unwrap();
        assert_eq!(effect.capability(), PluginCapability::WallpaperControl);
        assert_eq!(effect.read_capability(), PluginCapability::WallpaperRead);
        let snapshot = json!({"available":true,"generation":7,"configured":{"custom_image_configured":false,"position":"fill"}});
        effect.validate(&snapshot).unwrap();
        let mut stale = snapshot.clone();
        stale["generation"] = 8.into();
        assert!(effect.validate(&stale).is_err());
        let mut path = value.clone();
        path["transaction"]["path"] = "/private/image.png".into();
        assert!(AppearanceEffect::parse(&path).is_err());
        path = value;
        path["path"] = "file:///private/image.png".into();
        assert!(AppearanceEffect::parse(&path).is_err());
    }

    #[test]
    fn resolved_preferences_keep_inherited_and_custom_color_values() {
        let system = nickel_core::theme::Appearance {
            mode: nickel_core::theme::ThemeMode::Light,
            accent: [12, 34, 56],
            intensity: 42,
        };
        let mut settings = ShellSettings::default();
        let inherited = resolved_appearance(&settings, system);
        assert_eq!(inherited["theme"], "light");
        assert_eq!(inherited["accent"], json!([12, 34, 56]));
        assert_eq!(inherited["intensity"], 42);
        settings.accent_hue = Some(271);
        settings.accent_intensity = Some(63);
        let custom = resolved_appearance(&settings, system);
        assert_eq!(custom["hue"], 271);
        assert_eq!(custom["intensity"], 63);
        assert_eq!(
            custom["accent"],
            json!(nickel_core::theme::accent_from_hue(271))
        );
    }

    #[test]
    fn typed_appearance_contract_preserves_custom_hue_and_rejects_invalid_or_stale_values() {
        let request = appearance_request();
        let effect = AppearanceEffect::parse(&request).unwrap();
        assert_eq!(effect.capability(), PluginCapability::AppearanceControl);
        assert!(effect.validate(&json!({"available":true,"generation":1,"configured":request["transaction"]["prior"]})).is_ok());
        assert!(effect.validate(&json!({"available":true,"generation":2,"configured":request["transaction"]["prior"]})).is_err());
        for (field, value) in [
            ("accent_hue", json!(360)),
            ("accent_intensity", json!(101)),
            ("theme", json!("invalid")),
            ("image", json!("/private/image")),
        ] {
            let mut invalid = request.clone();
            invalid["transaction"]["requested"][field] = value;
            assert!(AppearanceEffect::parse(&invalid).is_err(), "{field}");
        }
        let mut unknown = request;
        unknown["path"] = json!("/private/path");
        assert!(AppearanceEffect::parse(&unknown).is_err());
    }
    #[test]
    fn wallpaper_contract_uses_current_catalog_identity_and_rejects_paths() {
        let mut request = json!({"type":"wallpaper.change","transaction":{"generation":3,"prior":{"custom_image_configured":false,"position":"fill"},"change":{"kind":"select_approved_image","image_id":"approved"}}});
        let effect = AppearanceEffect::parse(&request).unwrap();
        let snapshot = json!({"available":true,"generation":3,"configured":request["transaction"]["prior"],"images":[{"id":"approved","configured":false}]});
        effect.validate(&snapshot).unwrap();
        let mut stale = snapshot;
        stale["images"] = json!([]);
        assert!(effect.validate(&stale).is_err());
        request["transaction"]["change"]["path"] = json!("/private/image");
        assert!(AppearanceEffect::parse(&request).is_err());
    }
}
