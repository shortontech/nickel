//! Nickel policy adapters for Twinkle's admitted composition engine.
//!
//! Catalog resolution and capability declarations remain native host policy.
//! Twinkle receives only selected owners, implementations and bounded metadata.

use std::collections::BTreeMap;

use nickel_core::package_composition::{PackageIdentity, ResolvedShellPackage};
use nickel_core::plugins::PluginManifest;
use serde_json::{Value, json};
use twinkle_jsx_runtime::composition::{
    AdmittedContribution, AdmittedExport, AdmittedLinkPlan, CompositionHost, OwnerRuntime,
    RegisteredSelection,
};

use crate::{CapabilityRuntimeExt, SettingsRuntimeExt};

#[derive(Clone, Debug)]
pub struct NickelCompositionHost;

impl CompositionHost for NickelCompositionHost {
    type Owner = PackageIdentity;
    type State = PluginManifest;

    fn publish_snapshot(
        &self,
        context: &mut OwnerRuntime<Self>,
        data: &Value,
    ) -> Result<(), String> {
        context
            .runtime
            .borrow_mut()
            .set_capability_store(&context.host_state.capabilities, data)
            .map(|_| ())
    }

    fn resolve_registered(
        &self,
        packages: &BTreeMap<PackageIdentity, OwnerRuntime<Self>>,
        node: &Value,
    ) -> Result<Option<RegisteredSelection<PackageIdentity>>, String> {
        let Some(page) = node.get("settingPage") else {
            return Ok(None);
        };
        let provider = page
            .get("provider")
            .and_then(Value::as_str)
            .ok_or("missing page provider")?;
        let id = page
            .get("id")
            .and_then(Value::as_str)
            .ok_or("missing page identity")?;
        let (owner, _) = packages
            .iter()
            .find(|(owner, context)| {
                owner.id == provider && context.runtime.borrow().has_registered_page(id)
            })
            .ok_or("unknown registered Settings page")?;
        Ok(Some(RegisteredSelection {
            owner: owner.clone(),
            implementation: format!("@settings-page/{id}"),
            key: format!("setting-page:{provider}/{id}"),
        }))
    }

    fn register_mount(
        &self,
        context: &mut OwnerRuntime<Self>,
        mount: &str,
        implementation: &str,
    ) -> Result<(), String> {
        self.validate_entry(context, implementation)?;
        if let Some(page) = implementation.strip_prefix("@settings-page/") {
            context
                .runtime
                .borrow_mut()
                .register_component_surface(mount, page, true)
        } else {
            context.runtime.borrow_mut().register_component_mount(
                mount,
                context
                    .exports
                    .get(implementation)
                    .ok_or("component implementation is not published by its owner")?,
            )
        }
    }

    fn validate_entry(
        &self,
        context: &OwnerRuntime<Self>,
        implementation: &str,
    ) -> Result<(), String> {
        if let Some(page) = implementation.strip_prefix("@settings-page/") {
            if !context.runtime.borrow().has_registered_page(page) {
                return Err("Settings page is no longer published by its owner".into());
            }
        } else if !context.exports.contains_key(implementation) {
            return Err("component implementation is not published by its owner".into());
        }
        Ok(())
    }
}

/// Project already resolved native policy into generic execution links.
/// This does not install or approve a package or confer new capabilities.
pub fn admitted_link_plan(resolution: &ResolvedShellPackage) -> AdmittedLinkPlan<PackageIdentity> {
    AdmittedLinkPlan {
        owner_order: resolution.inheritance_chain.clone(),
        exports: resolution
            .exports
            .iter()
            .map(|(contract, entry)| {
                (
                    contract.clone(),
                    AdmittedExport {
                        implemented_by: entry.implemented_by.clone(),
                        implementation: entry.implementation.clone(),
                    },
                )
            })
            .collect(),
        contributions: resolution
            .contributions
            .iter()
            .map(|(collection, entries)| {
                (
                    collection.clone(),
                    entries
                        .iter()
                        .map(|entry| AdmittedContribution {
                            contributed_by: entry.contributed_by.clone(),
                            implementation: entry.implementation.clone(),
                            key: format!(
                                "{}/{}@{}/{}",
                                entry.collection,
                                entry.contributed_by.id,
                                entry.contributed_by.version,
                                entry.id,
                            ),
                            metadata: json!({
                                "id": entry.id,
                                "provider": entry.contributed_by.id,
                                "version": entry.contributed_by.version.to_string(),
                            }),
                        })
                        .collect(),
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::package_composition::ResolvedContribution;

    #[test]
    fn admitted_contribution_keeps_native_provider_identity_and_key_prefix() {
        let owner = PackageIdentity {
            id: "provider".into(),
            version: "1.2.3".parse().unwrap(),
        };
        let resolution = ResolvedShellPackage {
            active: owner.clone(),
            inheritance_chain: vec![owner.clone()],
            exports: BTreeMap::new(),
            contributions: BTreeMap::from([(
                "items".into(),
                vec![ResolvedContribution {
                    collection: "items".into(),
                    id: "example".into(),
                    implementation: "View".into(),
                    priority: 0,
                    contributed_by: owner.clone(),
                }],
            )]),
        };
        let plan = admitted_link_plan(&resolution);
        let entry = &plan.contributions["items"][0];
        assert_eq!(entry.contributed_by, owner);
        assert_eq!(entry.key, "items/provider@1.2.3/example");
        assert_eq!(
            entry.metadata,
            json!({
                "id":"example", "provider":"provider", "version":"1.2.3"
            })
        );
    }

    #[test]
    fn registered_settings_selection_requires_a_live_native_provider() {
        let host = NickelCompositionHost;
        assert!(
            host.resolve_registered(
                &BTreeMap::new(),
                &json!({
                    "settingPage": {"provider":"missing", "id":"page"}
                })
            )
            .is_err()
        );
        assert!(
            host.resolve_registered(
                &BTreeMap::new(),
                &json!({
                    "contract":"ordinary"
                })
            )
            .unwrap()
            .is_none()
        );
    }
}
