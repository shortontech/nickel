//! Presentation-neutral default-application capability boundary.
//!
//! Stable string identities make this API suitable for JavaScript and other
//! clients without exposing a Settings page model. Native platform policy and
//! consent remain owned by [`AssociationService`].

use std::{fmt, sync::Arc};

use crate::{
    AssociationCapability, AssociationError, AssociationFamily, AssociationScope,
    AssociationService, AssociationSnapshot, AssociationTarget, VersionedAssociationSnapshot,
    VersionedChangeOutcome, association_service, is_protected_association_handler,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AssociationTargetId(String);

impl AssociationTargetId {
    pub fn from_target(target: &AssociationTarget) -> Self {
        let (kind, value) = match target {
            AssociationTarget::Extension(value) => ("extension", value.as_str()),
            AssociationTarget::Mime(value) => ("mime", value.as_str()),
            AssociationTarget::Scheme(value) => ("scheme", value.as_str()),
        };
        Self(format!("{kind}:{value}"))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, AssociationCapabilityError> {
        let value = value.into();
        let Some((kind, identity)) = value.split_once(':') else {
            return Err(AssociationCapabilityError::InvalidTargetId(value));
        };
        if identity.is_empty()
            || identity.len() > 512
            || identity.chars().any(char::is_control)
            || !matches!(kind, "extension" | "mime" | "scheme")
        {
            return Err(AssociationCapabilityError::InvalidTargetId(value));
        }
        if kind == "extension" && !identity.starts_with('.') {
            return Err(AssociationCapabilityError::InvalidTargetId(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn target(&self) -> AssociationTarget {
        let (kind, value) = self.0.split_once(':').expect("validated target id");
        match kind {
            "extension" => AssociationTarget::extension(value),
            "mime" => AssociationTarget::mime(value),
            "scheme" => AssociationTarget::scheme(value),
            _ => unreachable!("validated target kind"),
        }
    }
}

impl fmt::Display for AssociationTargetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationHandlerInfo {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub source: String,
    /// Host policy excludes this handler from remote default changes.
    pub protected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationTargetInfo {
    pub id: AssociationTargetId,
    pub family: AssociationFamily,
    pub effective_handler_id: Option<String>,
    /// Protection is separate from the operating system's reported capability.
    pub protected: bool,
    pub capability: AssociationCapability,
    pub scope: AssociationScope,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationCatalog {
    pub revision: u64,
    pub targets: Vec<AssociationTargetInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationHandlers {
    pub revision: u64,
    pub target: AssociationTargetInfo,
    pub handlers: Vec<AssociationHandlerInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetDefaultRequest {
    pub target_id: AssociationTargetId,
    pub handler_id: String,
    pub expected_revision: u64,
    pub expected_handler_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetDefaultResult {
    Applied(AssociationHandlers),
    NativeConsentRequired { revision: u64, detail: String },
    Rejected { revision: u64, detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssociationCapabilityError {
    InvalidTargetId(String),
    Platform(String),
}

impl fmt::Display for AssociationCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTargetId(id) => write!(formatter, "invalid association target id: {id}"),
            Self::Platform(detail) => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for AssociationCapabilityError {}

impl From<AssociationError> for AssociationCapabilityError {
    fn from(error: AssociationError) -> Self {
        Self::Platform(error.0)
    }
}

/// Client contract intended for capability-checked runtime bindings.
pub trait ApplicationAssociationsClient: Send + Sync {
    fn list(&self) -> Result<AssociationCatalog, AssociationCapabilityError>;
    fn get_handlers(
        &self,
        target_id: &AssociationTargetId,
    ) -> Result<AssociationHandlers, AssociationCapabilityError>;
    fn set_default(
        &self,
        request: SetDefaultRequest,
    ) -> Result<SetDefaultResult, AssociationCapabilityError>;
    fn open_system_settings(&self) -> Result<(), AssociationCapabilityError>;
}

pub struct NativeApplicationAssociations {
    service: Arc<AssociationService>,
    open_system_settings: fn() -> Result<(), AssociationError>,
}

impl NativeApplicationAssociations {
    pub fn shared() -> Self {
        Self {
            service: association_service(),
            open_system_settings: open_native_default_apps_settings,
        }
    }

    #[cfg(test)]
    fn with_service(service: Arc<AssociationService>) -> Self {
        Self {
            service,
            open_system_settings: || Ok(()),
        }
    }
}

impl Default for NativeApplicationAssociations {
    fn default() -> Self {
        Self::shared()
    }
}

impl ApplicationAssociationsClient for NativeApplicationAssociations {
    fn list(&self) -> Result<AssociationCatalog, AssociationCapabilityError> {
        let (revision, snapshots) = self.service.inspect_available_versioned()?;
        Ok(AssociationCatalog {
            revision,
            targets: snapshots.iter().map(target_info).collect(),
        })
    }

    fn get_handlers(
        &self,
        target_id: &AssociationTargetId,
    ) -> Result<AssociationHandlers, AssociationCapabilityError> {
        let versioned = self.service.inspect_versioned(&target_id.target())?;
        Ok(handlers(versioned))
    }

    fn set_default(
        &self,
        request: SetDefaultRequest,
    ) -> Result<SetDefaultResult, AssociationCapabilityError> {
        let outcome = self.service.request_change_if_current(
            &request.target_id.target(),
            request.expected_revision,
            request.expected_handler_id.as_deref(),
            &request.handler_id,
        )?;
        Ok(match outcome {
            VersionedChangeOutcome::Confirmed(snapshot) => {
                SetDefaultResult::Applied(handlers(snapshot))
            }
            VersionedChangeOutcome::NativeConsentRequired { generation, detail } => {
                SetDefaultResult::NativeConsentRequired {
                    revision: generation,
                    detail,
                }
            }
            VersionedChangeOutcome::Rejected { generation, detail } => SetDefaultResult::Rejected {
                revision: generation,
                detail,
            },
        })
    }

    fn open_system_settings(&self) -> Result<(), AssociationCapabilityError> {
        (self.open_system_settings)().map_err(Into::into)
    }
}

fn target_info(snapshot: &AssociationSnapshot) -> AssociationTargetInfo {
    AssociationTargetInfo {
        id: AssociationTargetId::from_target(&snapshot.target),
        family: snapshot.target.family(),
        effective_handler_id: snapshot
            .effective
            .as_ref()
            .map(|handler| handler.id.clone()),
        protected: snapshot
            .effective
            .as_ref()
            .is_some_and(|handler| is_protected_association_handler(&handler.id)),
        capability: snapshot.capability,
        scope: snapshot.scope,
        detail: snapshot.detail.clone(),
    }
}

fn handlers(versioned: VersionedAssociationSnapshot) -> AssociationHandlers {
    AssociationHandlers {
        revision: versioned.generation,
        target: target_info(&versioned.snapshot),
        handlers: versioned
            .snapshot
            .handlers
            .into_iter()
            .map(|handler| AssociationHandlerInfo {
                protected: is_protected_association_handler(&handler.id),
                id: handler.id,
                name: handler.name,
                icon: handler.icon,
                source: handler.source,
            })
            .collect(),
    }
}

#[cfg(target_os = "windows")]
fn open_native_default_apps_settings() -> Result<(), AssociationError> {
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:defaultapps")
        .spawn()
        .map(|_| ())
        .map_err(|error| AssociationError(format!("could not open Windows Default apps: {error}")))
}

#[cfg(target_os = "linux")]
fn open_native_default_apps_settings() -> Result<(), AssociationError> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (program, argument) = if desktop.contains("kde") {
        ("systemsettings", "kcm_componentchooser")
    } else {
        ("gnome-control-center", "default-apps")
    };
    std::process::Command::new(program)
        .arg(argument)
        .spawn()
        .map(|_| ())
        .map_err(|error| AssociationError(format!("could not open system Default apps: {error}")))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn open_native_default_apps_settings() -> Result<(), AssociationError> {
    Err(AssociationError(
        "system Default apps settings are unsupported on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{ApplicationHandler, AssociationBackend, ChangeOutcome};

    struct Fixture {
        current: Arc<Mutex<String>>,
    }

    impl Fixture {
        fn snapshot(&self, target: &AssociationTarget) -> AssociationSnapshot {
            let handlers = ["old.desktop", "new.desktop", "nickel-settings.desktop"]
                .into_iter()
                .map(|id| ApplicationHandler {
                    id: id.into(),
                    name: id.into(),
                    icon: None,
                    source: "fixture".into(),
                })
                .collect::<Vec<_>>();
            let current = self.current.lock().unwrap().clone();
            AssociationSnapshot {
                target: target.clone(),
                effective: handlers
                    .iter()
                    .find(|handler| handler.id == current)
                    .cloned(),
                handlers,
                capability: AssociationCapability::DirectUserChange,
                scope: AssociationScope::User,
                detail: String::new(),
            }
        }
    }

    impl AssociationBackend for Fixture {
        fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
            Ok(vec![AssociationTarget::mime("text/plain")])
        }

        fn inspect(
            &self,
            target: &AssociationTarget,
        ) -> Result<AssociationSnapshot, AssociationError> {
            Ok(self.snapshot(target))
        }

        fn request_change(
            &self,
            target: &AssociationTarget,
            handler_id: &str,
        ) -> Result<ChangeOutcome, AssociationError> {
            *self.current.lock().unwrap() = handler_id.into();
            Ok(ChangeOutcome::Confirmed(self.snapshot(target)))
        }
    }

    fn client_with_current(current: Arc<Mutex<String>>) -> NativeApplicationAssociations {
        NativeApplicationAssociations::with_service(Arc::new(AssociationService::new(Box::new(
            Fixture { current },
        ))))
    }

    fn client() -> NativeApplicationAssociations {
        client_with_current(Arc::new(Mutex::new("old.desktop".into())))
    }

    #[test]
    fn target_ids_are_stable_and_strictly_validated() {
        let target = AssociationTarget::mime("text/plain");
        let id = AssociationTargetId::from_target(&target);
        assert_eq!(id.as_str(), "mime:text/plain");
        assert_eq!(
            AssociationTargetId::parse(id.to_string()).unwrap().target(),
            target
        );
        assert!(AssociationTargetId::parse("extension:txt").is_err());
        assert!(AssociationTargetId::parse("unknown:value").is_err());
    }

    #[test]
    fn list_and_handlers_share_stable_native_identities() {
        let client = client();
        let catalog = client.list().unwrap();
        assert_eq!(catalog.targets[0].id.as_str(), "mime:text/plain");
        let handlers = client.get_handlers(&catalog.targets[0].id).unwrap();
        assert_eq!(handlers.revision, catalog.revision);
        assert_eq!(
            handlers.target.effective_handler_id.as_deref(),
            Some("old.desktop")
        );
        assert!(
            handlers
                .handlers
                .iter()
                .any(|handler| handler.id == "new.desktop")
        );
    }

    #[test]
    fn set_default_requires_the_observed_revision_and_handler() {
        let client = client();
        let observed = client
            .get_handlers(&AssociationTargetId::parse("mime:text/plain").unwrap())
            .unwrap();
        let stale = client
            .set_default(SetDefaultRequest {
                target_id: observed.target.id.clone(),
                handler_id: "new.desktop".into(),
                expected_revision: observed.revision + 1,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(matches!(stale, SetDefaultResult::Rejected { .. }));

        let applied = client
            .set_default(SetDefaultRequest {
                target_id: observed.target.id,
                handler_id: "new.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(matches!(
            applied,
            SetDefaultResult::Applied(AssociationHandlers {
                target: AssociationTargetInfo { effective_handler_id: Some(id), .. },
                ..
            }) if id == "new.desktop"
        ));
    }

    #[test]
    fn external_default_change_rejects_an_observed_request() {
        let current = Arc::new(Mutex::new("old.desktop".into()));
        let client = client_with_current(Arc::clone(&current));
        let observed = client.list().unwrap();
        *current.lock().unwrap() = "new.desktop".into();
        let result = client
            .set_default(SetDefaultRequest {
                target_id: observed.targets[0].id.clone(),
                handler_id: "old.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(
            matches!(result, SetDefaultResult::Rejected { revision, .. } if revision > observed.revision)
        );
        assert_eq!(*current.lock().unwrap(), "new.desktop");
    }

    #[test]
    fn protected_current_default_reports_policy_without_overriding_platform_truth() {
        let current = Arc::new(Mutex::new("nickel-settings.desktop".into()));
        let client = client_with_current(Arc::clone(&current));
        let observed = client
            .get_handlers(&AssociationTargetId::parse("mime:text/plain").unwrap())
            .unwrap();
        assert!(observed.target.protected);
        assert_eq!(
            observed.target.capability,
            AssociationCapability::DirectUserChange
        );
        assert!(
            observed
                .handlers
                .iter()
                .find(|handler| handler.id == "nickel-settings.desktop")
                .unwrap()
                .protected
        );
        let result = client
            .set_default(SetDefaultRequest {
                target_id: observed.target.id,
                handler_id: "new.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("nickel-settings.desktop".into()),
            })
            .unwrap();
        assert!(matches!(result, SetDefaultResult::Rejected { .. }));
        assert_eq!(*current.lock().unwrap(), "nickel-settings.desktop");
    }

    #[test]
    fn native_consent_is_reported_without_claiming_a_default_change() {
        struct ConsentBackend(Fixture);
        impl AssociationBackend for ConsentBackend {
            fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
                self.0.available_targets()
            }
            fn inspect(
                &self,
                target: &AssociationTarget,
            ) -> Result<AssociationSnapshot, AssociationError> {
                let mut snapshot = self.0.snapshot(target);
                snapshot.capability = AssociationCapability::NativeConsent;
                Ok(snapshot)
            }
            fn request_change(
                &self,
                _: &AssociationTarget,
                _: &str,
            ) -> Result<ChangeOutcome, AssociationError> {
                Ok(ChangeOutcome::NativeConsentRequired {
                    detail: "approve in native Settings".into(),
                })
            }
        }
        let client = NativeApplicationAssociations::with_service(Arc::new(
            AssociationService::new(Box::new(ConsentBackend(Fixture {
                current: Arc::new(Mutex::new("old.desktop".into())),
            }))),
        ));
        let observed = client.list().unwrap();
        assert_eq!(
            observed.targets[0].capability,
            AssociationCapability::NativeConsent
        );
        let result = client
            .set_default(SetDefaultRequest {
                target_id: observed.targets[0].id.clone(),
                handler_id: "new.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(
            matches!(result, SetDefaultResult::NativeConsentRequired { revision, .. } if revision == observed.revision)
        );
        assert_eq!(
            client.list().unwrap().targets[0]
                .effective_handler_id
                .as_deref(),
            Some("old.desktop")
        );
    }

    #[test]
    fn unavailable_handler_is_rejected_before_platform_mutation() {
        let client = client();
        let observed = client.list().unwrap();
        let result = client
            .set_default(SetDefaultRequest {
                target_id: observed.targets[0].id.clone(),
                handler_id: "missing.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(matches!(result, SetDefaultResult::Rejected { .. }));
        assert_eq!(
            client.list().unwrap().targets[0]
                .effective_handler_id
                .as_deref(),
            Some("old.desktop")
        );
    }

    #[test]
    fn protected_handlers_remain_unselectable_at_the_public_boundary() {
        let client = client();
        let observed = client.list().unwrap();
        let result = client
            .set_default(SetDefaultRequest {
                target_id: observed.targets[0].id.clone(),
                handler_id: "nickel-settings.desktop".into(),
                expected_revision: observed.revision,
                expected_handler_id: Some("old.desktop".into()),
            })
            .unwrap();
        assert!(matches!(result, SetDefaultResult::Rejected { .. }));
    }
}
