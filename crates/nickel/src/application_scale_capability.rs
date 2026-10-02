//! Reusable application UI scale policy backed by the existing toolkit journal.
use nickel_core::dpi::{
    ApplicationScaleJournal, ApplicationScalePolicy, ApplicationScaleSettings, Scale120,
};
#[cfg(any(test, not(target_os = "linux")))]
use nickel_platform::ToolkitCapability;
use nickel_platform::{ToolkitFamily, ToolkitScaleBackend};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    hash::{Hash, Hasher},
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ApplicationScaleEffect {
    pub(crate) revision: String,
    requested: ApplicationScalePolicy,
}
impl ApplicationScaleEffect {
    pub(crate) fn parse(effect: &Value) -> Result<Self, String> {
        let object = effect
            .as_object()
            .ok_or("invalid application scale operation")?;
        if object.len() != 3 || effect["type"] != "displays.setApplicationScale" {
            return Err("unknown application scale operation fields".into());
        }
        let revision = effect["revision"]
            .as_str()
            .filter(|value| value.len() == 16)
            .ok_or("application scale observation is unavailable")?
            .to_owned();
        let policy = effect["policy"]
            .as_object()
            .ok_or("invalid application scale policy")?;
        let requested = match policy.get("policy").and_then(Value::as_str) {
            Some("follow") if policy.len() == 1 => ApplicationScalePolicy::FollowNickel,
            Some("unchanged") if policy.len() == 1 => ApplicationScalePolicy::Unchanged,
            Some("custom") if policy.len() == 2 => {
                let units = policy
                    .get("scale_120")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or("invalid custom application scale")?;
                let scale = nickel_platform::supported_custom_scales()
                    .into_iter()
                    .find(|scale| scale.units() == units)
                    .ok_or("unsupported custom application scale")?;
                ApplicationScalePolicy::Custom(scale)
            }
            _ => return Err("invalid application scale policy".into()),
        };
        Ok(Self {
            revision,
            requested,
        })
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true || snapshot["revision"].as_str() != Some(&self.revision) {
            return Err("application scale observation is stale".into());
        }
        Ok(())
    }
}
fn configured(policy: ApplicationScalePolicy) -> Value {
    match policy {
        ApplicationScalePolicy::FollowNickel => json!({"policy":"follow"}),
        ApplicationScalePolicy::Unchanged => json!({"policy":"unchanged"}),
        ApplicationScalePolicy::Custom(scale) => {
            json!({"policy":"custom","scale_120":scale.units()})
        }
    }
}
fn observation(
    path: &Path,
) -> Result<(Option<RegularFileRevision>, ApplicationScaleSettings), String> {
    let before =
        regular_file_revision(path).map_err(|_| "application scale journal is unavailable")?;
    let settings = ApplicationScaleSettings::load(path)
        .map_err(|_| "application scale journal is unavailable")?;
    if regular_file_revision(path).map_err(|_| "application scale journal is unavailable")?
        != before
    {
        return Err("application scale changed during observation".into());
    }
    Ok((before, settings))
}
fn revision(file: &Option<RegularFileRevision>, settings: &ApplicationScaleSettings) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    format!("{file:?}{settings:?}").hash(&mut hash);
    format!("{:016x}", hash.finish())
}
fn family_name(family: ToolkitFamily) -> &'static str {
    match family {
        ToolkitFamily::Gtk => "gtk",
        ToolkitFamily::Qt => "qt",
    }
}
#[cfg(not(target_os = "linux"))]
struct UnavailableToolkitBackend;
#[cfg(not(target_os = "linux"))]
impl ToolkitScaleBackend for UnavailableToolkitBackend {
    fn capabilities(&self) -> Vec<ToolkitCapability> {
        [ToolkitFamily::Gtk, ToolkitFamily::Qt]
            .into_iter()
            .map(|family| ToolkitCapability {
                family,
                available: false,
                live: false,
                restart_required: true,
            })
            .collect()
    }
    fn read(&self, _family: ToolkitFamily) -> Result<String, String> {
        Err("native toolkit scale is unavailable".into())
    }
    fn write(&self, _family: ToolkitFamily, _value: &str) -> Result<(), String> {
        Err("native toolkit scale is unavailable".into())
    }
}
fn backend() -> Box<dyn ToolkitScaleBackend> {
    #[cfg(target_os = "linux")]
    {
        Box::new(nickel_platform::LinuxToolkitScaleBackend::detect())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(UnavailableToolkitBackend)
    }
}
#[derive(Default)]
pub(crate) struct ApplicationScaleService {
    last_result: Option<(String, Value)>,
}
impl ApplicationScaleService {
    pub(crate) fn snapshot(&self) -> Value {
        let result = nickel_storage::config_path("application-scale.conf")
            .map_err(|_| "application scale journal is unavailable".to_owned())
            .and_then(|path| Self::snapshot_at(&path, &*backend()));
        match result {
            Ok(mut snapshot) => {
                if let Some((revision, result)) = &self.last_result {
                    if snapshot["revision"].as_str() == Some(revision) {
                        snapshot["last_result"] = result.clone();
                    }
                }
                snapshot
            }
            Err(reason) => json!({"available":false,"reason":reason}),
        }
    }
    fn snapshot_at(path: &Path, backend: &dyn ToolkitScaleBackend) -> Result<Value, String> {
        let (file, settings) = observation(path)?;
        let toolkits=backend.capabilities().into_iter().take(2).map(|capability| {
            let (owned,pending) = match capability.family {ToolkitFamily::Gtk => (settings.owned_gtk_applied.is_some(),settings.pending_gtk.is_some()), ToolkitFamily::Qt => (settings.owned_qt_applied.is_some(),settings.pending_qt.is_some())};
            json!({"family":family_name(capability.family),"available":capability.available,"live":capability.live,"restart_required":capability.restart_required,"owned":owned,"pending":pending})
        }).collect::<Vec<_>>();
        Ok(
            json!({"available":true,"revision":revision(&file,&settings),"configured":configured(settings.policy),"supported_scales":nickel_platform::supported_custom_scales().into_iter().map(Scale120::units).collect::<Vec<_>>(),"native_per_monitor":cfg!(target_os="windows"),"uncertain":settings.pending_gtk.is_some() || settings.pending_qt.is_some(),"toolkits":toolkits}),
        )
    }
    pub(crate) fn execute(
        &mut self,
        effect: &ApplicationScaleEffect,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let path = nickel_storage::config_path("application-scale.conf")
            .map_err(|_| "application scale journal is unavailable")?;
        let result = Self::execute_at(&path, &*backend(), effect, &mut check);
        let snapshot = self.snapshot();
        if let Some(revision) = snapshot["revision"].as_str() {
            let value = match &result {
                Ok(value) => value.clone(),
                Err(_) => {
                    json!({"rejected":true,"uncertain":snapshot["uncertain"]==true,"refresh_required":true})
                }
            };
            self.last_result = Some((revision.to_owned(), value));
        }
        result.map(|_| ())
    }
    fn execute_at(
        path: &Path,
        backend: &dyn ToolkitScaleBackend,
        effect: &ApplicationScaleEffect,
        check: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Value, String> {
        check()?;
        let mut journal = ApplicationScaleJournal::open(path.to_owned())
            .map_err(|_| "application scale ownership is busy or unavailable")?;
        let (file, mut settings) = observation(path)?;
        journal
            .load()
            .map_err(|_| "application scale journal changed")?;
        if revision(&file, &settings) != effect.revision {
            return Err("application scale observation is stale".into());
        }
        let check = RefCell::new(check);
        let expected = RefCell::new(file);
        let boundary = || {
            (check.borrow_mut())()?;
            if regular_file_revision(path)
                .map_err(|_| "application scale journal is unavailable")?
                != *expected.borrow()
            {
                return Err("application scale journal changed".into());
            }
            Ok(())
        };
        let report = nickel_platform::transact_application_scale(
            backend,
            &mut settings,
            effect.requested,
            |settings| {
                boundary()?;
                journal
                    .persist(settings)
                    .map_err(|_| "could not persist application scale ownership")?;
                *expected.borrow_mut() = regular_file_revision(path)
                    .map_err(|_| "application scale persistence is uncertain")?;
                Ok(())
            },
            boundary,
        )?;
        let outcomes=report.outcomes.into_iter().map(|outcome| json!({"family":family_name(outcome.family),"kind":match outcome.kind {nickel_platform::ToolkitOutcomeKind::Unchanged=>"unchanged",nickel_platform::ToolkitOutcomeKind::Confirmed=>"confirmed",nickel_platform::ToolkitOutcomeKind::ExternalConflict=>"external_conflict",nickel_platform::ToolkitOutcomeKind::Unavailable=>"unavailable",nickel_platform::ToolkitOutcomeKind::Failed=>"failed",nickel_platform::ToolkitOutcomeKind::Uncertain=>"uncertain"},"restart_required":outcome.restart_required})).collect::<Vec<_>>();
        Ok(json!({"outcomes":outcomes}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Offline;
    impl ToolkitScaleBackend for Offline {
        fn capabilities(&self) -> Vec<ToolkitCapability> {
            vec![ToolkitCapability {
                family: ToolkitFamily::Gtk,
                available: false,
                live: false,
                restart_required: true,
            }]
        }
        fn read(&self, _: ToolkitFamily) -> Result<String, String> {
            panic!("unavailable toolkit must not be read")
        }
        fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
            panic!("unavailable toolkit must not be written")
        }
    }
    fn path() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nickel-scale-capability-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("scale.conf")
    }
    fn effect(snapshot: &Value) -> ApplicationScaleEffect {
        ApplicationScaleEffect::parse(&json!({"type":"displays.setApplicationScale","revision":snapshot["revision"],"policy":{"policy":"custom","scale_120":180}})).unwrap()
    }
    #[test]
    fn application_scale_rejects_unknown_fields_and_unsupported_units() {
        for policy in [
            json!({"policy":"custom","scale_120":0}),
            json!({"policy":"custom","scale_120":181}),
            json!({"policy":"custom","scale_120":180.5}),
            json!({"policy":"follow","extra":true}),
        ] {
            assert!(ApplicationScaleEffect::parse(&json!({"type":"displays.setApplicationScale","revision":"0123456789abcdef","policy":policy})).is_err());
        }
    }
    #[test]
    fn application_scale_persists_policy_without_touching_unavailable_toolkits() {
        let path = path();
        let snapshot = ApplicationScaleService::snapshot_at(&path, &Offline).unwrap();
        let request = effect(&snapshot);
        let result =
            ApplicationScaleService::execute_at(&path, &Offline, &request, &mut || Ok(())).unwrap();
        assert_eq!(result["outcomes"][0]["kind"], "unavailable");
        let next = ApplicationScaleService::snapshot_at(&path, &Offline).unwrap();
        assert_eq!(
            next["configured"],
            json!({"policy":"custom","scale_120":180})
        );
        assert_ne!(snapshot["revision"], next["revision"]);
        assert!(
            ApplicationScaleService::execute_at(&path, &Offline, &request, &mut || Ok(())).is_err()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn application_scale_rechecks_authority_before_persistence() {
        let path = path();
        let request = effect(&ApplicationScaleService::snapshot_at(&path, &Offline).unwrap());
        let mut checks = 0;
        let result = ApplicationScaleService::execute_at(&path, &Offline, &request, &mut || {
            checks += 1;
            if checks >= 4 {
                Err("revoked".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(!path.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn application_scale_preserves_existing_toolkit_ownership() {
        let path = path();
        let mut settings = ApplicationScaleSettings::default();
        settings.owned_gtk_previous = Some("1".into());
        settings.owned_gtk_applied = Some("2".into());
        let mut journal = ApplicationScaleJournal::open(path.clone()).unwrap();
        journal.load().unwrap();
        journal.persist(&settings).unwrap();
        drop(journal);
        let snapshot = ApplicationScaleService::snapshot_at(&path, &Offline).unwrap();
        assert_eq!(snapshot["toolkits"][0]["owned"], true);
        assert!(!snapshot.to_string().contains("owned_gtk_previous"));
        ApplicationScaleService::execute_at(&path, &Offline, &effect(&snapshot), &mut || Ok(()))
            .unwrap();
        let saved = ApplicationScaleSettings::load(&path).unwrap();
        assert_eq!(saved.owned_gtk_previous, settings.owned_gtk_previous);
        assert_eq!(saved.owned_gtk_applied, settings.owned_gtk_applied);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn application_scale_same_contents_replacement_invalidates_revision() {
        let path = path();
        let mut journal = ApplicationScaleJournal::open(path.clone()).unwrap();
        journal.load().unwrap();
        journal
            .persist(&ApplicationScaleSettings::default())
            .unwrap();
        drop(journal);
        let request = effect(&ApplicationScaleService::snapshot_at(&path, &Offline).unwrap());
        let replacement = path.with_extension("new");
        std::fs::write(&replacement, std::fs::read(&path).unwrap()).unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert!(
            ApplicationScaleService::execute_at(&path, &Offline, &request, &mut || Ok(())).is_err()
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
