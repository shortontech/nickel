//! Public association snapshots and effects shared by every package.
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use nickel_core::plugins::PluginCapability;
use nickel_platform::{
    ApplicationAssociationsClient, AssociationCapability, AssociationScope, AssociationSnapshot,
    AssociationTargetId, NativeApplicationAssociations, SetDefaultRequest, SetDefaultResult,
    association_service, is_protected_association_handler,
};
use serde_json::{Value, json};

const MAX_TARGETS: usize = 128;
const MAX_HANDLERS: usize = 128;
type CachedCatalog = Option<(Instant, Value)>;

fn catalog_cache() -> &'static Mutex<CachedCatalog> {
    static CACHE: OnceLock<Mutex<CachedCatalog>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn valid_identity(id: &str) -> bool {
    !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
}

fn text(value: &str) -> String {
    value.chars().take(512).collect()
}

fn catalog(revision: u64, snapshots: &[AssociationSnapshot], truncated: bool) -> Value {
    json!({
        "available": true, "revision": revision.to_string(), "truncated": truncated,
        "operations": {"setDefault": true, "openSystemSettings": true},
        "targets": snapshots.iter().take(MAX_TARGETS).filter(|snapshot| valid_identity(AssociationTargetId::from_target(&snapshot.target).as_str())).map(|snapshot| {
            let protected = snapshot.effective.as_ref().is_some_and(|handler| is_protected_association_handler(&handler.id));
            let effective = snapshot.effective.as_ref().filter(|handler| valid_identity(&handler.id));
            let effective_valid = effective.is_some() || snapshot.effective.is_none();
            let capability = match snapshot.capability {
                AssociationCapability::DirectUserChange => "directUserChange",
                AssociationCapability::NativeConsent => "nativeConsent",
                AssociationCapability::ReadOnly => "readOnly",
                AssociationCapability::Unsupported => "unsupported",
            };
            json!({
                "id": AssociationTargetId::from_target(&snapshot.target).as_str(),
                "family": snapshot.target.family().label(), "capability": capability,
                "scope": match snapshot.scope { AssociationScope::User => "user", AssociationScope::System => "system", AssociationScope::Policy => "policy" },
                "detail": text(&snapshot.detail), "protected": protected,
                "effectiveHandlerId": effective.map(|handler| &handler.id),
                "canSetDefault": effective_valid && !protected && matches!(snapshot.capability, AssociationCapability::DirectUserChange | AssociationCapability::NativeConsent),
                "handlersTruncated": snapshot.handlers.len() > MAX_HANDLERS,
                "handlers": snapshot.handlers.iter().take(MAX_HANDLERS).filter(|handler| valid_identity(&handler.id)).map(|handler| json!({
                    "id": handler.id, "name": text(&handler.name), "icon": handler.icon.as_deref().map(text),
                    "source": text(&handler.source), "protected": is_protected_association_handler(&handler.id),
                })).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>()
    })
}

pub(crate) fn snapshot(result: Option<&Value>) -> Value {
    // Native association inventory may involve process or registry queries.
    // Sharing a short observation cache avoids doing that work per surface render.
    let mut cache = catalog_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut value = if let Some((_, value)) = cache
        .as_ref()
        .filter(|(observed, _)| observed.elapsed() < Duration::from_secs(2))
    {
        value.clone()
    } else {
        let value = match association_service().inspect_available_bounded_versioned(MAX_TARGETS) {
            Ok((revision, snapshots, truncated)) => catalog(revision, &snapshots, truncated),
            Err(error) => {
                json!({"available":false,"reason":text(&error.to_string()),"targets":[],"operations":{"setDefault":false,"openSystemSettings":true}})
            }
        };
        *cache = Some((Instant::now(), value.clone()));
        value
    };
    value["lastResult"] = result.cloned().unwrap_or(Value::Null);
    value
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssociationsEffect {
    SetDefault {
        target_id: AssociationTargetId,
        handler_id: String,
        revision: u64,
        prior_handler_id: Option<String>,
    },
    OpenSystemSettings,
}

impl AssociationsEffect {
    pub(crate) fn parse(effect: &Value) -> Result<Self, String> {
        match effect["type"].as_str() {
            Some("associations.openSystemSettings") => Ok(Self::OpenSystemSettings),
            Some("associations.setDefault") => {
                let target_id = AssociationTargetId::parse(
                    effect["targetId"]
                        .as_str()
                        .ok_or("invalid association target")?,
                )
                .map_err(|error| error.to_string())?;
                let handler_id = effect["handlerId"]
                    .as_str()
                    .filter(|id| {
                        !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
                    })
                    .ok_or("invalid association handler")?
                    .to_owned();
                let revision = effect["revision"]
                    .as_str()
                    .and_then(|revision| revision.parse::<u64>().ok())
                    .filter(|revision| *revision > 0)
                    .ok_or("invalid association revision")?;
                let prior_handler_id = match effect
                    .get("expectedHandlerId")
                    .ok_or("missing prior association handler")?
                {
                    Value::Null => None,
                    Value::String(id) if valid_identity(id) => Some(id.clone()),
                    _ => return Err("invalid prior association handler".into()),
                };
                Ok(Self::SetDefault {
                    target_id,
                    handler_id,
                    revision,
                    prior_handler_id,
                })
            }
            _ => Err("unsupported association operation".into()),
        }
    }

    pub(crate) fn capability(&self) -> PluginCapability {
        PluginCapability::AssociationsControl
    }

    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        let Self::SetDefault {
            target_id,
            handler_id,
            revision,
            prior_handler_id,
        } = self
        else {
            return Ok(());
        };
        if snapshot["revision"].as_str() != Some(revision.to_string().as_str()) {
            return Err("association snapshot is stale".into());
        }
        let target = snapshot["targets"]
            .as_array()
            .and_then(|targets| {
                targets
                    .iter()
                    .find(|target| target["id"].as_str() == Some(target_id.as_str()))
            })
            .ok_or("association target is unavailable")?;
        if target["effectiveHandlerId"].as_str() != prior_handler_id.as_deref() {
            return Err("association default changed".into());
        }
        if snapshot["available"] != true
            || target["canSetDefault"] != true
            || target["protected"] == true
        {
            return Err("association changes are unavailable or protected".into());
        }
        let handler = target["handlers"]
            .as_array()
            .and_then(|handlers| {
                handlers
                    .iter()
                    .find(|handler| handler["id"].as_str() == Some(handler_id))
            })
            .ok_or("association handler is unavailable")?;
        if handler["protected"] == true || is_protected_association_handler(handler_id) {
            return Err("association handler is protected".into());
        }
        Ok(())
    }

    pub(crate) fn execute(
        &self,
        client: &dyn ApplicationAssociationsClient,
    ) -> Result<Value, String> {
        match self {
            Self::OpenSystemSettings => {
                client
                    .open_system_settings()
                    .map_err(|error| error.to_string())?;
                Ok(json!({"status":"systemSettingsOpened"}))
            }
            Self::SetDefault {
                target_id,
                handler_id,
                revision,
                prior_handler_id,
            } => {
                let outcome = client
                    .set_default(SetDefaultRequest {
                        target_id: target_id.clone(),
                        handler_id: handler_id.clone(),
                        expected_revision: *revision,
                        expected_handler_id: prior_handler_id.clone(),
                    })
                    .map_err(|error| error.to_string())?;
                Ok(match outcome {
                    SetDefaultResult::Applied(snapshot) => {
                        json!({"status":"applied","revision":snapshot.revision.to_string(),"targetId":target_id.as_str(),"handlerId":snapshot.target.effective_handler_id})
                    }
                    SetDefaultResult::NativeConsentRequired { revision, detail } => {
                        json!({"status":"nativeConsentRequired","revision":revision.to_string(),"targetId":target_id.as_str(),"detail":text(&detail)})
                    }
                    SetDefaultResult::Rejected { revision, detail } => {
                        json!({"status":"rejected","revision":revision.to_string(),"targetId":target_id.as_str(),"detail":text(&detail)})
                    }
                })
            }
        }
    }

    pub(crate) fn execute_native(&self) -> Result<Value, String> {
        let result = self.execute(&NativeApplicationAssociations::shared());
        *catalog_cache()
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_platform::{ApplicationHandler, AssociationTarget};

    fn fixture() -> Value {
        catalog(
            7,
            &[AssociationSnapshot {
                target: AssociationTarget::mime("text/plain"),
                effective: Some(ApplicationHandler {
                    id: "old.desktop".into(),
                    name: "Old".into(),
                    icon: None,
                    source: "fixture".into(),
                }),
                handlers: ["new.desktop", "nickel-settings.desktop"]
                    .into_iter()
                    .map(|id| ApplicationHandler {
                        id: id.into(),
                        name: id.into(),
                        icon: None,
                        source: "fixture".into(),
                    })
                    .collect(),
                capability: AssociationCapability::NativeConsent,
                scope: AssociationScope::User,
                detail: "Native consent required".into(),
            }],
            false,
        )
    }

    fn request() -> Value {
        json!({"type":"associations.setDefault","targetId":"mime:text/plain","handlerId":"new.desktop","revision":"7","expectedHandlerId":"old.desktop"})
    }

    #[test]
    fn associations_effects_validate_revision_identity_protection_and_native_availability() {
        let snapshot = fixture();
        let effect = AssociationsEffect::parse(&request()).unwrap();
        assert_eq!(effect.capability(), PluginCapability::AssociationsControl);
        assert_eq!(snapshot["targets"][0]["capability"], "nativeConsent");
        assert!(effect.validate(&snapshot).is_ok());
        for change in [
            json!({"revision":"8"}),
            json!({"handlerId":"missing.desktop"}),
            json!({"handlerId":"nickel-settings.desktop"}),
            json!({"expectedHandlerId":null}),
            json!({"targetId":"mime:text/html"}),
        ] {
            let mut request = request();
            request
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert!(
                AssociationsEffect::parse(&request)
                    .unwrap()
                    .validate(&snapshot)
                    .is_err()
            );
        }
        let mut readonly = snapshot;
        readonly["targets"][0]["canSetDefault"] = false.into();
        assert!(effect.validate(&readonly).is_err());
        assert!(AssociationsEffect::parse(&json!({"type":"associations.setDefault","targetId":"mime:text/plain","handlerId":"new.desktop","revision":7})).is_err());
    }

    #[test]
    fn associations_snapshot_bounds_targets_handlers_and_descriptions() {
        let target = AssociationSnapshot {
            target: AssociationTarget::mime("text/plain"),
            effective: None,
            handlers: (0..200)
                .map(|id| ApplicationHandler {
                    id: format!("{id}.desktop"),
                    name: "n".repeat(2000),
                    icon: None,
                    source: "fixture".into(),
                })
                .collect(),
            capability: AssociationCapability::ReadOnly,
            scope: AssociationScope::Policy,
            detail: "d".repeat(2000),
        };
        let snapshot = catalog(1, &vec![target; 200], true);
        assert_eq!(snapshot["targets"].as_array().unwrap().len(), MAX_TARGETS);
        assert_eq!(
            snapshot["targets"][0]["handlers"].as_array().unwrap().len(),
            MAX_HANDLERS
        );
        assert_eq!(
            snapshot["targets"][0]["detail"].as_str().unwrap().len(),
            512
        );
        assert_eq!(snapshot["targets"][0]["handlersTruncated"], true);
        assert_eq!(snapshot["targets"][0]["canSetDefault"], false);
    }

    #[test]
    fn associations_native_consent_result_does_not_claim_an_applied_change() {
        struct Consent;
        impl ApplicationAssociationsClient for Consent {
            fn list(
                &self,
            ) -> Result<
                nickel_platform::AssociationCatalog,
                nickel_platform::AssociationCapabilityError,
            > {
                unreachable!()
            }
            fn get_handlers(
                &self,
                _: &AssociationTargetId,
            ) -> Result<
                nickel_platform::AssociationHandlers,
                nickel_platform::AssociationCapabilityError,
            > {
                unreachable!()
            }
            fn set_default(
                &self,
                request: SetDefaultRequest,
            ) -> Result<SetDefaultResult, nickel_platform::AssociationCapabilityError> {
                assert_eq!(request.expected_revision, 7);
                assert_eq!(request.expected_handler_id.as_deref(), Some("old.desktop"));
                Ok(SetDefaultResult::NativeConsentRequired {
                    revision: 7,
                    detail: "approve in native Settings".into(),
                })
            }
            fn open_system_settings(
                &self,
            ) -> Result<(), nickel_platform::AssociationCapabilityError> {
                Ok(())
            }
        }
        let result = AssociationsEffect::parse(&request())
            .unwrap()
            .execute(&Consent)
            .unwrap();
        assert_eq!(result["status"], "nativeConsentRequired");
        assert!(result.get("handlerId").is_none());
        assert_eq!(
            AssociationsEffect::OpenSystemSettings
                .execute(&Consent)
                .unwrap()["status"],
            "systemSettingsOpened"
        );
    }
}
