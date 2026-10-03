//! Deterministic, platform-neutral composition of shell packages.

use crate::plugins::PluginCapability;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PACKAGE_COMPOSITION_API_VERSION: u16 = 1;
pub const MAX_COMPOSITION_PACKAGES: usize = 64;
pub const MAX_COMPOSITION_DEPTH: usize = 8;
pub const MAX_EXPORTS_PER_PACKAGE: usize = 128;
pub const MAX_CONTRIBUTIONS_PER_PACKAGE: usize = 128;
pub const MAX_RESOLVED_CONTRIBUTIONS: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShellPackageComposition {
    pub api_version: u16,
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requires: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exports: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub replaces: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub uses: BTreeMap<String, PublicExportReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributions: Vec<SemanticContribution>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicExportReference {
    pub package: String,
    pub export: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticContribution {
    pub collection: String,
    pub id: String,
    pub implementation: String,
    #[serde(default)]
    pub priority: i16,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageIdentity {
    pub id: String,
    pub version: Version,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPublicExport {
    pub contract: String,
    pub implementation: String,
    /// Origin of the stable public contract.
    pub declared_by: PackageIdentity,
    /// Identity used to enforce capabilities for effects from this code.
    pub implemented_by: PackageIdentity,
    /// Package that made the export/replacement/reuse selection.
    pub selected_by: PackageIdentity,
    pub selection: ExportSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportSelection {
    Export,
    Replacement,
    Reuse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedContribution {
    pub collection: String,
    pub id: String,
    pub implementation: String,
    pub priority: i16,
    /// Identity used to enforce capabilities for this contribution.
    pub contributed_by: PackageIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedShellPackage {
    pub active: PackageIdentity,
    /// Root package first, active package last.
    pub inheritance_chain: Vec<PackageIdentity>,
    pub exports: BTreeMap<String, ResolvedPublicExport>,
    pub contributions: BTreeMap<String, Vec<ResolvedContribution>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionError {
    TooManyPackages {
        count: usize,
        max: usize,
    },
    UnknownPackage(String),
    IdentityMismatch {
        catalog_id: String,
        manifest_id: String,
    },
    InvalidManifest {
        package: String,
        reason: String,
    },
    MissingBaseRequirement {
        package: String,
        base: String,
    },
    UnexpectedRequirement {
        package: String,
        dependency: String,
    },
    VersionMismatch {
        package: String,
        required: String,
        found: String,
    },
    InheritanceCycle(Vec<String>),
    InheritanceTooDeep {
        package: String,
        max: usize,
    },
    UnknownReplacement {
        package: String,
        contract: String,
    },
    UnknownReuse {
        package: String,
        contract: String,
    },
    InvalidReuseSource {
        package: String,
        source: String,
        contract: String,
    },
    ConflictingDeclaration {
        package: String,
        contract: String,
    },
    TooManyResolvedContributions {
        count: usize,
        max: usize,
    },
}

pub fn resolve_shell_package(
    catalog: &BTreeMap<String, ShellPackageComposition>,
    active_id: &str,
) -> Result<ResolvedShellPackage, CompositionError> {
    if catalog.len() > MAX_COMPOSITION_PACKAGES {
        return Err(CompositionError::TooManyPackages {
            count: catalog.len(),
            max: MAX_COMPOSITION_PACKAGES,
        });
    }
    for (key, package) in catalog {
        if key != &package.id {
            return Err(CompositionError::IdentityMismatch {
                catalog_id: key.clone(),
                manifest_id: package.id.clone(),
            });
        }
        package.validate()?;
    }

    let mut chain = Vec::new();
    let mut positions = BTreeMap::new();
    let mut current = active_id;
    loop {
        let package = catalog
            .get(current)
            .ok_or_else(|| CompositionError::UnknownPackage(current.into()))?;
        if let Some(position) = positions.insert(current.to_owned(), chain.len()) {
            let mut cycle = chain[position..]
                .iter()
                .map(|p: &&ShellPackageComposition| p.id.clone())
                .collect::<Vec<_>>();
            cycle.push(current.into());
            return Err(CompositionError::InheritanceCycle(cycle));
        }
        chain.push(package);
        if chain.len() > MAX_COMPOSITION_DEPTH {
            return Err(CompositionError::InheritanceTooDeep {
                package: active_id.into(),
                max: MAX_COMPOSITION_DEPTH,
            });
        }
        let Some(base_id) = &package.extends else {
            break;
        };
        let requirement = package.requires.get(base_id).ok_or_else(|| {
            CompositionError::MissingBaseRequirement {
                package: package.id.clone(),
                base: base_id.clone(),
            }
        })?;
        let base = catalog
            .get(base_id)
            .ok_or_else(|| CompositionError::UnknownPackage(base_id.clone()))?;
        let requirement = VersionReq::parse(requirement)
            .map_err(|e| invalid(package, format!("invalid base version requirement: {e}")))?;
        let found = parse_version(base)?;
        if !requirement.matches(&found) {
            return Err(CompositionError::VersionMismatch {
                package: base.id.clone(),
                required: requirement.to_string(),
                found: found.to_string(),
            });
        }
        current = base_id;
    }
    chain.reverse();
    let identities = chain
        .iter()
        .map(|p| identity(p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ancestor_exports: BTreeMap<String, BTreeMap<String, ResolvedPublicExport>> =
        BTreeMap::new();
    let mut exports: BTreeMap<String, ResolvedPublicExport> = BTreeMap::new();
    let mut contributions: BTreeMap<String, Vec<ResolvedContribution>> = BTreeMap::new();
    let mut contribution_count = 0;

    for package in chain {
        let owner = identity(package)?;
        for (contract, implementation) in &package.exports {
            if exports.contains_key(contract) {
                return Err(CompositionError::ConflictingDeclaration {
                    package: package.id.clone(),
                    contract: contract.clone(),
                });
            }
            exports.insert(
                contract.clone(),
                ResolvedPublicExport {
                    contract: contract.clone(),
                    implementation: implementation.clone(),
                    declared_by: owner.clone(),
                    implemented_by: owner.clone(),
                    selected_by: owner.clone(),
                    selection: ExportSelection::Export,
                },
            );
        }
        for (contract, implementation) in &package.replaces {
            let inherited =
                exports
                    .get(contract)
                    .ok_or_else(|| CompositionError::UnknownReplacement {
                        package: package.id.clone(),
                        contract: contract.clone(),
                    })?;
            let declared_by = inherited.declared_by.clone();
            exports.insert(
                contract.clone(),
                ResolvedPublicExport {
                    contract: contract.clone(),
                    implementation: implementation.clone(),
                    declared_by,
                    implemented_by: owner.clone(),
                    selected_by: owner.clone(),
                    selection: ExportSelection::Replacement,
                },
            );
        }
        for (contract, reference) in &package.uses {
            let inherited =
                exports
                    .get(contract)
                    .ok_or_else(|| CompositionError::UnknownReuse {
                        package: package.id.clone(),
                        contract: contract.clone(),
                    })?;
            let declared_by = inherited.declared_by.clone();
            let source = ancestor_exports
                .get(&reference.package)
                .and_then(|exports| exports.get(&reference.export))
                .ok_or_else(|| CompositionError::InvalidReuseSource {
                    package: package.id.clone(),
                    source: reference.package.clone(),
                    contract: contract.clone(),
                })?;
            // Reuse is a selection of the same public contract, never a private
            // source import or a transfer of the selector's permissions.
            if reference.export != *contract {
                return Err(CompositionError::InvalidReuseSource {
                    package: package.id.clone(),
                    source: reference.package.clone(),
                    contract: contract.clone(),
                });
            }
            let implementation = source.implementation.clone();
            let implemented_by = source.implemented_by.clone();
            exports.insert(
                contract.clone(),
                ResolvedPublicExport {
                    contract: contract.clone(),
                    implementation,
                    declared_by,
                    implemented_by,
                    selected_by: owner.clone(),
                    selection: ExportSelection::Reuse,
                },
            );
        }
        for entry in &package.contributions {
            contribution_count += 1;
            if contribution_count > MAX_RESOLVED_CONTRIBUTIONS {
                return Err(CompositionError::TooManyResolvedContributions {
                    count: contribution_count,
                    max: MAX_RESOLVED_CONTRIBUTIONS,
                });
            }
            contributions
                .entry(entry.collection.clone())
                .or_default()
                .push(ResolvedContribution {
                    collection: entry.collection.clone(),
                    id: entry.id.clone(),
                    implementation: entry.implementation.clone(),
                    priority: entry.priority,
                    contributed_by: owner.clone(),
                });
        }
        ancestor_exports.insert(package.id.clone(), exports.clone());
    }
    for entries in contributions.values_mut() {
        entries.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.contributed_by.id.cmp(&b.contributed_by.id))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    Ok(ResolvedShellPackage {
        active: identities.last().expect("nonempty chain").clone(),
        inheritance_chain: identities,
        exports,
        contributions,
    })
}

/// Grants belong to a concrete code identity, including its version. A
/// selecting shell cannot authorize a dependency or contribution with its own
/// grants. The host must supply the actual execution owner at dispatch time.
#[derive(Clone, Debug, Default)]
pub struct CompositionGrants {
    grants: BTreeMap<PackageIdentity, Vec<PluginCapability>>,
}

impl CompositionGrants {
    pub fn grant(&mut self, owner: PackageIdentity, capabilities: Vec<PluginCapability>) {
        self.grants.insert(owner, capabilities);
    }

    pub fn revoke(&mut self, owner: &PackageIdentity) {
        self.grants.remove(owner);
    }

    pub fn permits(&self, owner: &PackageIdentity, capability: PluginCapability) -> bool {
        self.grants
            .get(owner)
            .is_some_and(|grants| grants.contains(&capability))
    }

    pub fn permits_export(
        &self,
        export: &ResolvedPublicExport,
        capability: PluginCapability,
    ) -> bool {
        self.permits(&export.implemented_by, capability)
    }

    pub fn permits_contribution(
        &self,
        contribution: &ResolvedContribution,
        capability: PluginCapability,
    ) -> bool {
        self.permits(&contribution.contributed_by, capability)
    }
}

impl ShellPackageComposition {
    pub fn validate(&self) -> Result<(), CompositionError> {
        if self.api_version != PACKAGE_COMPOSITION_API_VERSION {
            return Err(invalid(self, "unsupported composition API version"));
        }
        if !valid_name(&self.id, false) {
            return Err(invalid(self, "invalid package ID"));
        }
        parse_version(self)?;
        if self.requires.len() > 1 {
            return Err(invalid(self, "only the base package may be required"));
        }
        match &self.extends {
            Some(base) if base == &self.id || !valid_name(base, false) => {
                return Err(invalid(self, "invalid base package ID"));
            }
            Some(base) if self.requires.keys().next() != Some(base) => {
                return Err(CompositionError::MissingBaseRequirement {
                    package: self.id.clone(),
                    base: base.clone(),
                });
            }
            None if !self.requires.is_empty() => {
                return Err(CompositionError::UnexpectedRequirement {
                    package: self.id.clone(),
                    dependency: self.requires.keys().next().unwrap().clone(),
                });
            }
            _ => {}
        }
        if self.exports.len() > MAX_EXPORTS_PER_PACKAGE
            || self.replaces.len() > MAX_EXPORTS_PER_PACKAGE
            || self.uses.len() > MAX_EXPORTS_PER_PACKAGE
        {
            return Err(invalid(self, "too many public export declarations"));
        }
        let mut contracts = BTreeSet::new();
        for (contract, path) in self.exports.iter().chain(&self.replaces) {
            if !valid_contract(contract) || !valid_module_path(path) {
                return Err(invalid(self, format!("invalid public export {contract:?}")));
            }
            if !contracts.insert(contract) {
                return Err(CompositionError::ConflictingDeclaration {
                    package: self.id.clone(),
                    contract: contract.clone(),
                });
            }
        }
        for (contract, reference) in &self.uses {
            if !valid_contract(contract)
                || !valid_name(&reference.package, false)
                || !valid_contract(&reference.export)
            {
                return Err(invalid(
                    self,
                    format!("invalid public export reuse {contract:?}"),
                ));
            }
            if !contracts.insert(contract) {
                return Err(CompositionError::ConflictingDeclaration {
                    package: self.id.clone(),
                    contract: contract.clone(),
                });
            }
        }
        if self.contributions.len() > MAX_CONTRIBUTIONS_PER_PACKAGE {
            return Err(invalid(self, "too many semantic contributions"));
        }
        let mut entries = BTreeSet::new();
        for entry in &self.contributions {
            if !valid_name(&entry.collection, true)
                || !valid_local_id(&entry.id)
                || !valid_module_path(&entry.implementation)
                || !entries.insert((&entry.collection, &entry.id))
            {
                return Err(invalid(self, "invalid or duplicate semantic contribution"));
            }
        }
        Ok(())
    }
}

fn invalid(package: &ShellPackageComposition, reason: impl Into<String>) -> CompositionError {
    CompositionError::InvalidManifest {
        package: package.id.clone(),
        reason: reason.into(),
    }
}
fn parse_version(package: &ShellPackageComposition) -> Result<Version, CompositionError> {
    Version::parse(&package.version)
        .map_err(|e| invalid(package, format!("invalid semantic version: {e}")))
}
fn identity(package: &ShellPackageComposition) -> Result<PackageIdentity, CompositionError> {
    Ok(PackageIdentity {
        id: package.id.clone(),
        version: parse_version(package)?,
    })
}
fn valid_contract(value: &str) -> bool {
    valid_name(value, true)
}
fn valid_name(value: &str, uppercase: bool) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 64
                && part.as_bytes()[0].is_ascii_lowercase()
                && part.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b == b'-'
                        || uppercase && b.is_ascii_uppercase()
                })
        })
}
fn valid_local_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn valid_module_path(value: &str) -> bool {
    let (path, export) = value
        .split_once('#')
        .map_or((value, None), |(path, export)| (path, Some(export)));
    if export.is_some_and(|name| {
        name.is_empty()
            || name.len() > 128
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphabetic()
                    || matches!(byte, b'_' | b'$')
                    || (index > 0 && byte.is_ascii_digit())
            })
    }) {
        return false;
    }
    value.len() <= 512
        && !value.chars().any(char::is_control)
        && value.starts_with("./")
        && !value.contains('\\')
        && !value.contains(':')
        && value[2..]
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
        && (path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".ts")
            || path.ends_with(".tsx"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package(id: &str, version: &str) -> ShellPackageComposition {
        ShellPackageComposition {
            api_version: 1,
            id: id.into(),
            version: version.into(),
            extends: None,
            requires: BTreeMap::new(),
            exports: BTreeMap::new(),
            replaces: BTreeMap::new(),
            uses: BTreeMap::new(),
            contributions: vec![],
        }
    }

    #[test]
    fn resolves_public_composition_with_capability_provenance() {
        let mut base = package("nickel-default", "0.2.4");
        base.exports
            .insert("shell.taskbar".into(), "./Taskbar.jsx".into());
        base.exports
            .insert("shell.quickSettings".into(), "./QuickSettings.jsx".into());
        base.contributions.push(SemanticContribution {
            collection: "settings.pages".into(),
            id: "display".into(),
            implementation: "./Display.jsx".into(),
            priority: 0,
        });
        let mut derived = package("my-shell", "1.0.0");
        derived.extends = Some("nickel-default".into());
        derived
            .requires
            .insert("nickel-default".into(), "^0.2".into());
        derived
            .replaces
            .insert("shell.taskbar".into(), "./MyTaskbar.jsx".into());
        derived.uses.insert(
            "shell.quickSettings".into(),
            PublicExportReference {
                package: "nickel-default".into(),
                export: "shell.quickSettings".into(),
            },
        );
        derived.contributions.push(SemanticContribution {
            collection: "settings.pages".into(),
            id: "theme".into(),
            implementation: "./Theme.jsx".into(),
            priority: 10,
        });
        let catalog = BTreeMap::from([(base.id.clone(), base), (derived.id.clone(), derived)]);
        let resolved = resolve_shell_package(&catalog, "my-shell").unwrap();
        assert_eq!(
            resolved
                .inheritance_chain
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["nickel-default", "my-shell"]
        );
        let taskbar = &resolved.exports["shell.taskbar"];
        assert_eq!(
            (
                &taskbar.declared_by.id,
                &taskbar.implemented_by.id,
                taskbar.selection
            ),
            (
                &"nickel-default".into(),
                &"my-shell".into(),
                ExportSelection::Replacement
            )
        );
        let quick = &resolved.exports["shell.quickSettings"];
        assert_eq!(
            (
                &quick.implemented_by.id,
                &quick.selected_by.id,
                quick.selection
            ),
            (
                &"nickel-default".into(),
                &"my-shell".into(),
                ExportSelection::Reuse
            )
        );
        assert_eq!(
            resolved.contributions["settings.pages"]
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["theme", "display"]
        );
        assert_eq!(
            resolved.contributions["settings.pages"][0]
                .contributed_by
                .id,
            "my-shell"
        );
    }

    #[test]
    fn reuse_of_inherited_export_preserves_original_grants_and_version() {
        let mut base = package("base", "1.0.0");
        base.exports
            .insert("shell.taskbar".into(), "./Taskbar.jsx".into());
        let mut middle = package("middle", "1.0.0");
        middle.extends = Some("base".into());
        middle.requires.insert("base".into(), "^1".into());
        let mut child = package("child", "1.0.0");
        child.extends = Some("middle".into());
        child.requires.insert("middle".into(), "^1".into());
        child.uses.insert(
            "shell.taskbar".into(),
            PublicExportReference {
                package: "middle".into(),
                export: "shell.taskbar".into(),
            },
        );
        let catalog = BTreeMap::from([
            (base.id.clone(), base),
            (middle.id.clone(), middle),
            (child.id.clone(), child),
        ]);
        let resolved = resolve_shell_package(&catalog, "child").unwrap();
        let export = &resolved.exports["shell.taskbar"];
        assert_eq!(export.implemented_by.id, "base");
        let mut grants = CompositionGrants::default();
        grants.grant(
            resolved.active.clone(),
            vec![PluginCapability::WindowsFocus],
        );
        assert!(!grants.permits_export(export, PluginCapability::WindowsFocus));
        let mut wrong_version = export.implemented_by.clone();
        wrong_version.version = Version::new(2, 0, 0);
        grants.grant(wrong_version, vec![PluginCapability::WindowsFocus]);
        assert!(!grants.permits_export(export, PluginCapability::WindowsFocus));
        grants.grant(
            export.implemented_by.clone(),
            vec![PluginCapability::WindowsFocus],
        );
        assert!(grants.permits_export(export, PluginCapability::WindowsFocus));
        grants.revoke(&export.implemented_by);
        assert!(!grants.permits_export(export, PluginCapability::WindowsFocus));
    }

    #[test]
    fn contributions_have_independent_owner_namespaces_and_deterministic_order() {
        let mut base = package("base", "1.0.0");
        let entry = SemanticContribution {
            collection: "settings.pages".into(),
            id: "appearance".into(),
            implementation: "./Appearance.jsx".into(),
            priority: 0,
        };
        base.contributions.push(entry.clone());
        let mut child = package("child", "1.0.0");
        child.extends = Some("base".into());
        child.requires.insert("base".into(), "*".into());
        child.contributions.push(entry);
        let catalog = BTreeMap::from([(base.id.clone(), base), (child.id.clone(), child)]);
        let resolved = resolve_shell_package(&catalog, "child").unwrap();
        let entries = &resolved.contributions["settings.pages"];
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.contributed_by.id.as_str())
                .collect::<Vec<_>>(),
            ["base", "child"]
        );
        let mut grants = CompositionGrants::default();
        grants.grant(
            resolved.active.clone(),
            vec![PluginCapability::SettingsWrite],
        );
        assert!(!grants.permits_contribution(&entries[0], PluginCapability::SettingsWrite));
        assert!(grants.permits_contribution(&entries[1], PluginCapability::SettingsWrite));
    }

    #[test]
    fn rejects_identity_version_cycle_and_unknown_replacement() {
        assert!(matches!(
            resolve_shell_package(
                &BTreeMap::from([("alias".into(), package("actual", "1.0.0"))]),
                "alias"
            ),
            Err(CompositionError::IdentityMismatch { .. })
        ));
        let mut a = package("a", "1.0.0");
        a.extends = Some("b".into());
        a.requires.insert("b".into(), "*".into());
        let mut b = package("b", "1.0.0");
        b.extends = Some("a".into());
        b.requires.insert("a".into(), "*".into());
        assert!(matches!(
            resolve_shell_package(&BTreeMap::from([("a".into(), a), ("b".into(), b)]), "a"),
            Err(CompositionError::InheritanceCycle(_))
        ));
        let base = package("base", "1.0.0");
        let mut child = package("child", "1.0.0");
        child.extends = Some("base".into());
        child.requires.insert("base".into(), "^2".into());
        assert!(matches!(
            resolve_shell_package(
                &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
                "child"
            ),
            Err(CompositionError::VersionMismatch { .. })
        ));
        let base = package("base", "1.0.0");
        let mut child = package("child", "1.0.0");
        child.extends = Some("base".into());
        child.requires.insert("base".into(), "*".into());
        child
            .replaces
            .insert("shell.missing".into(), "./Missing.jsx".into());
        assert!(matches!(
            resolve_shell_package(
                &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
                "child"
            ),
            Err(CompositionError::UnknownReplacement { .. })
        ));
    }

    #[test]
    fn rejects_private_paths_and_ambiguous_contracts() {
        let mut p = package("theme", "1.0.0");
        p.exports
            .insert("shell.taskbar".into(), "../private.jsx".into());
        assert!(matches!(
            p.validate(),
            Err(CompositionError::InvalidManifest { .. })
        ));
        p.exports
            .insert("shell.taskbar".into(), "./Taskbar.jsx".into());
        p.replaces
            .insert("shell.taskbar".into(), "./Other.jsx".into());
        assert!(matches!(
            p.validate(),
            Err(CompositionError::ConflictingDeclaration { .. })
        ));
    }

    #[test]
    fn rejects_non_ancestor_reuse_and_bounded_depth() {
        let mut base = package("base", "1.0.0");
        base.exports
            .insert("shell.taskbar".into(), "./Taskbar.jsx".into());
        let mut child = package("child", "1.0.0");
        child.extends = Some("base".into());
        child.requires.insert("base".into(), "*".into());
        child.uses.insert(
            "shell.taskbar".into(),
            PublicExportReference {
                package: "unrelated".into(),
                export: "shell.taskbar".into(),
            },
        );
        let unrelated = package("unrelated", "1.0.0");
        let catalog = BTreeMap::from([
            ("base".into(), base),
            ("child".into(), child),
            ("unrelated".into(), unrelated),
        ]);
        assert!(matches!(
            resolve_shell_package(&catalog, "child"),
            Err(CompositionError::InvalidReuseSource { .. })
        ));

        let mut catalog = BTreeMap::new();
        for index in 0..=MAX_COMPOSITION_DEPTH {
            let id = format!("package-{index}");
            let mut current = package(&id, "1.0.0");
            if index > 0 {
                let base = format!("package-{}", index - 1);
                current.extends = Some(base.clone());
                current.requires.insert(base, "*".into());
            }
            catalog.insert(id, current);
        }
        assert!(matches!(
            resolve_shell_package(&catalog, &format!("package-{MAX_COMPOSITION_DEPTH}")),
            Err(CompositionError::InheritanceTooDeep { .. })
        ));
    }
}
