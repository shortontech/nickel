//! Default-application MCP adapter shared by the Linux and Windows owners.
use nickel_platform::{
    AssociationCapability, AssociationScope, AssociationSnapshot, AssociationTarget,
    VersionedAssociationSnapshot, VersionedChangeOutcome, association_service,
    is_protected_association_handler,
};
use nickel_remote_control::{
    DesktopPermit,
    default_associations::{
        self as api, Capability, Handler, Scope, Snapshot, Target, TargetKind, Transaction,
        TransactionOutcome,
    },
};

fn native_target(target: &Target) -> Result<AssociationTarget, String> {
    if !target.valid() {
        return Err("association target is invalid or outside its supported bound".into());
    }
    Ok(match target.kind {
        TargetKind::Extension => AssociationTarget::extension(target.id.clone()),
        TargetKind::Mime => AssociationTarget::mime(target.id.clone()),
        TargetKind::Scheme => AssociationTarget::scheme(target.id.clone()),
    })
}

fn api_target(target: &AssociationTarget) -> Target {
    match target {
        AssociationTarget::Extension(id) => Target {
            kind: TargetKind::Extension,
            id: id.clone(),
        },
        AssociationTarget::Mime(id) => Target {
            kind: TargetKind::Mime,
            id: id.clone(),
        },
        AssociationTarget::Scheme(id) => Target {
            kind: TargetKind::Scheme,
            id: id.clone(),
        },
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    let end = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max_bytes)
        .last()
        .unwrap_or(0);
    if value.len() <= max_bytes {
        value.to_owned()
    } else {
        value[..end].to_owned()
    }
}

fn handler(handler: &nickel_platform::ApplicationHandler) -> Option<Handler> {
    (api::valid_catalog_id(&handler.id) && !is_protected_association_handler(&handler.id)).then(
        || Handler {
            catalog_id: handler.id.clone(),
            name: truncate_utf8(&handler.name, api::MAX_HANDLER_NAME_BYTES),
        },
    )
}

fn project(observed: VersionedAssociationSnapshot) -> Snapshot {
    let AssociationSnapshot {
        target,
        effective,
        handlers,
        capability,
        scope,
        detail,
    } = observed.snapshot;
    Snapshot {
        generation: observed.generation,
        target: api_target(&target),
        effective: effective.as_ref().and_then(handler),
        handlers: handlers
            .iter()
            .filter_map(handler)
            .take(api::MAX_HANDLERS)
            .collect(),
        capability: match capability {
            AssociationCapability::DirectUserChange => Capability::DirectUserChange,
            AssociationCapability::NativeConsent => Capability::NativeConsent,
            AssociationCapability::ReadOnly => Capability::ReadOnly,
            AssociationCapability::Unsupported => Capability::Unsupported,
        },
        scope: match scope {
            AssociationScope::User => Scope::User,
            AssociationScope::System => Scope::System,
            AssociationScope::Policy => Scope::Policy,
        },
        detail: truncate_utf8(&detail, 512),
    }
}

pub(crate) fn read(permit: DesktopPermit, target: Target) -> Result<Snapshot, String> {
    let target = native_target(&target)?;
    permit.with_debug(false, || {
        association_service()
            .inspect_versioned(&target)
            .map(project)
            .map_err(|error| error.to_string())
    })
}

pub(crate) fn transact(
    permit: DesktopPermit,
    transaction: Transaction,
) -> Result<TransactionOutcome, String> {
    if !transaction.valid() {
        return Err("association transaction is invalid or outside its supported bound".into());
    }
    let target = native_target(&transaction.target)?;
    permit.with_debug_input_deadline(false, |boundary| {
        let service = association_service();
        let current = project(service.inspect_versioned(&target).map_err(|error| error.to_string())?);
        if current.generation != transaction.generation
            || current.effective.as_ref().map(|handler| handler.catalog_id.as_str())
                != transaction.prior_catalog_id.as_deref()
        {
            return Ok(TransactionOutcome::Rejected {
                generation: current.generation,
                detail: "the association changed; read current state before retrying".into(),
            });
        }
        if !current.handlers.iter().any(|handler| handler.catalog_id == transaction.requested_catalog_id) {
            return Ok(TransactionOutcome::Rejected {
                generation: current.generation,
                detail: "the requested handler is unavailable, protected, or outside the bounded catalog".into(),
            });
        }
        permit.check_commit_boundary(boundary)?;
        service
            .request_change_if_current(
                &target,
                transaction.generation,
                transaction.prior_catalog_id.as_deref(),
                &transaction.requested_catalog_id,
            )
            .map(|outcome| match outcome {
                VersionedChangeOutcome::Confirmed(snapshot) => TransactionOutcome::Confirmed { snapshot: project(snapshot) },
                VersionedChangeOutcome::NativeConsentRequired { generation, detail } => TransactionOutcome::NativeConsentRequired { generation, detail },
                VersionedChangeOutcome::Rejected { generation, detail } => TransactionOutcome::Rejected { generation, detail },
            })
            .map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_excludes_protected_handlers_and_native_paths() {
        let projected = project(VersionedAssociationSnapshot {
            generation: 7,
            snapshot: AssociationSnapshot {
                target: AssociationTarget::mime("text/plain"),
                effective: None,
                handlers: vec![
                    nickel_platform::ApplicationHandler {
                        id: "nickel-settings.desktop".into(),
                        name: "Nickel Settings".into(),
                        icon: Some("/secret/icon".into()),
                        source: "/secret/source".into(),
                    },
                    nickel_platform::ApplicationHandler {
                        id: "editor.desktop".into(),
                        name: "Editor".into(),
                        icon: Some("/icons/editor".into()),
                        source: "/apps/editor.desktop".into(),
                    },
                ],
                capability: AssociationCapability::DirectUserChange,
                scope: AssociationScope::User,
                detail: "User association".into(),
            },
        });
        assert_eq!(projected.handlers.len(), 1);
        assert_eq!(projected.handlers[0].catalog_id, "editor.desktop");
        let json = serde_json::to_string(&projected).unwrap();
        assert!(!json.contains("/secret"));
        assert!(!json.contains("/apps"));
    }
}
