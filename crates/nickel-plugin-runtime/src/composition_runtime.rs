//! Package-lifecycle composition with isolated JavaScript authority.
//!
//! Packages never exchange JavaScript functions, globals or handler indices.
//! The generic host invokes public components and contributions through opaque
//! Rust references, and validates native effects using their Rust-owned origin.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use nickel_core::package_composition::{
    PackageIdentity, ResolvedShellPackage, resolve_shell_package,
};
use nickel_core::plugins::PluginPackage;
use serde_json::Value;

use crate::{
    JsxModuleGraph, JsxRuntime, ModuleSource, NativePatchCounters, NativePatchEnvelope,
    NativePatchOperation, ScheduledPatch, ScheduledRender,
};

static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);
static NEXT_SHARED_MOUNT: AtomicU64 = AtomicU64::new(1);
const MAX_MOUNTS: usize = 512;
const MAX_COMPONENT_NODES: usize = 4096;
const MAX_TREE_DEPTH: usize = 128;
const MAX_HANDLERS: usize = 4096;
const MAX_JSON_BYTES: usize = 1024 * 1024;
const MAX_EFFECTS: usize = 1024;

#[derive(Clone, Debug)]
pub struct ComponentReference {
    runtime: u64,
    owner: PackageIdentity,
    implementation: String,
    incarnation: u64,
}
impl ComponentReference {
    pub fn owner(&self) -> &PackageIdentity {
        &self.owner
    }
}

#[derive(Clone, Debug)]
pub struct ComponentMount {
    runtime: u64,
    id: u64,
}

#[derive(Clone, Debug)]
pub struct ComponentEventHandle {
    runtime: u64,
    mount: u64,
    generation: u64,
    action: u64,
    slot: String,
    owner: PackageIdentity,
}
impl ComponentEventHandle {
    pub fn owner(&self) -> &PackageIdentity {
        &self.owner
    }
}

#[derive(Clone, Debug)]
pub struct OwnedComponentEffect {
    runtime: u64,
    owner: PackageIdentity,
    value: Value,
    incarnation: u64,
}
impl OwnedComponentEffect {
    pub fn owner(&self) -> &PackageIdentity {
        &self.owner
    }
    pub fn value(&self) -> &Value {
        &self.value
    }
}

#[derive(Clone)]
pub struct RenderedComponent {
    pub node: Value,
    /// Node action indices address this host-owned table, not a JS runtime.
    pub events: BTreeMap<u64, ComponentEventHandle>,
    generation: u64,
}

impl RenderedComponent {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

pub struct ScheduledComponentDispatch {
    pub rendered: Option<RenderedComponent>,
    pub reconciliation_requested: bool,
    /// Composition callbacks may update another mount while draining effects.
    /// The expanded root must be rebuilt even when this mount itself was clean.
    pub requires_expansion: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PackageReloadSignatures {
    /// Signatures for the graph which is currently accepted.
    pub previous: BTreeMap<String, String>,
    /// Signatures for the candidate graph. Keys are normalized module#export names.
    pub replacement: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct ReloadedMountPatch {
    pub mount: ComponentMount,
    pub patch: NativePatchEnvelope,
    pub events: BTreeMap<u64, ComponentEventHandle>,
    pub generation: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CompositionReload {
    pub preserved_packages: Vec<PackageIdentity>,
    pub restarted_packages: Vec<PackageIdentity>,
    pub patches: Vec<ReloadedMountPatch>,
}

pub enum ScheduledExpandedBatch<T> {
    Unchanged,
    Rendered {
        rendered: RenderedComponent,
        validated: T,
        reconciliation_requested: bool,
    },
    Patched {
        patch: NativePatchEnvelope,
        events: BTreeMap<u64, ComponentEventHandle>,
        generation: u64,
        validated: T,
        reconciliation_requested: bool,
    },
}

struct ExpansionState {
    root: u64,
    events: BTreeMap<u64, ComponentEventHandle>,
    visited: std::collections::BTreeSet<String>,
    native_ids: std::collections::BTreeSet<String>,
    handler_slots: std::collections::BTreeSet<String>,
    next_event: u64,
}

struct PatchExpansion {
    retained: Value,
    dirty: std::collections::BTreeSet<u64>,
    rendered: BTreeMap<u64, RenderedComponent>,
}

struct PatchedMountDispatch {
    patch: Option<NativePatchEnvelope>,
    dirty_components: Vec<String>,
    reconciliation_requested: bool,
}

type NativePatchValidator<'a, T> = dyn FnMut(&NativePatchEnvelope, &BTreeMap<u64, ComponentEventHandle>, u64) -> Result<T, String>
    + 'a;

struct NativeRejection {
    owners: Vec<(u64, Vec<String>)>,
    message: String,
}

#[derive(Clone)]
struct CallbackGrant {
    receiver: u64,
    generation: u64,
    source: ComponentEventHandle,
}

#[derive(Clone)]
struct OwnedChildGrant {
    receiver: u64,
    generation: u64,
    source: u64,
    source_generation: u64,
    node: Value,
    events: BTreeMap<u64, ComponentEventHandle>,
}

#[derive(Clone)]
struct PackageRuntime {
    runtime: std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
    // Immutable trusted snapshot; transactions retain ownership, not a deep copy
    // of every inventory row. Updates replace the snapshot atomically.
    data: std::rc::Rc<Value>,
    exports: BTreeMap<String, String>,
    assets: BTreeMap<String, String>,
    incarnation: u64,
    source_digest: String,
    manifest: nickel_core::plugins::PluginManifest,
}
/// An opaque package context retained by native lifecycle ownership. Importing
/// it into another composition host never copies executable code or grants.
#[derive(Clone)]
pub struct ProviderContext {
    owner: PackageIdentity,
    package: PackageRuntime,
}

#[derive(Clone)]
struct MountState {
    reference: ComponentReference,
    generation: u64,
    props: Value,
    surface: Option<Value>,
}

#[derive(Clone, Default)]
struct MountPatchAuthority {
    /// Package-local handler slot -> stable expanded event token.
    slots: BTreeMap<String, u64>,
    /// Package-local native identity -> retained composition expansion paths
    /// contained by that native subtree.
    splices: BTreeMap<String, std::collections::BTreeSet<String>>,
    /// Exact unexpanded native identity -> composition traversal path. Keeping
    /// this mapping avoids inventing a new mount namespace on parent replacement.
    paths: BTreeMap<String, String>,
}

fn composition_child_identity(value: &Value, index: usize) -> String {
    value.get("key").map_or_else(
        || format!("#{index}"),
        |key| {
            format!(
                "@{}",
                key.to_string()
                    .bytes()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        },
    )
}

fn native_slot_in_subtree(slot: &str, target: &str) -> bool {
    slot.strip_prefix(target).is_some_and(|suffix| {
        suffix.is_empty() || suffix.starts_with('/') || suffix.starts_with(':')
    })
}

impl MountPatchAuthority {
    fn forget_paths(&mut self, target: &str) {
        let prefix = format!("{target}/");
        self.paths
            .retain(|id, _| id != target && !id.starts_with(&prefix));
    }

    fn record_paths(&mut self, value: &Value, path: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_TREE_DEPTH {
            return Err("component tree exceeds depth limit".into());
        }
        if let Some(id) = value.get("__nativeId").and_then(Value::as_str) {
            let local = id.rsplit_once("::").map_or(id, |(_, local)| local);
            self.paths.insert(local.to_owned(), path.to_owned());
            if self.paths.len() > MAX_COMPONENT_NODES {
                return Err("component path index exceeds node limit".into());
            }
        }
        match value {
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    self.record_paths(
                        value,
                        &format!("{path}/{}", composition_child_identity(value, index)),
                        depth + 1,
                    )?;
                }
            }
            Value::Object(object) => {
                for (key, value) in object {
                    if key != "__handlerSlots" {
                        self.record_paths(value, &format!("{path}/{key}"), depth + 1)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

struct CompositionCheckpoint {
    mounts: BTreeMap<u64, MountState>,
    effects: Vec<OwnedComponentEffect>,
    nested_mounts: BTreeMap<(u64, String), ComponentMount>,
    callbacks: BTreeMap<u64, CallbackGrant>,
    children: BTreeMap<u64, OwnedChildGrant>,
    data: BTreeMap<PackageIdentity, std::rc::Rc<Value>>,
    scheduled_renders: BTreeMap<u64, RenderedComponent>,
    patch_authority: BTreeMap<u64, MountPatchAuthority>,
}

/// One generic composition host, one context/module cache per package.
/// This API deliberately has no per-page or per-component JS host lifecycle.
/// Hosts retain responsibility for capability/value/revision checks on effects.
pub struct ShellCompositionRuntime {
    id: u64,
    resolution: ResolvedShellPackage,
    packages: BTreeMap<PackageIdentity, PackageRuntime>,
    mounts: BTreeMap<u64, MountState>,
    next_mount: u64,
    next_generation: u64,
    effects: Vec<OwnedComponentEffect>,
    nested_mounts: BTreeMap<(u64, String), ComponentMount>,
    callbacks: BTreeMap<u64, CallbackGrant>,
    children: BTreeMap<u64, OwnedChildGrant>,
    next_callback: u64,
    callback_depth: usize,
    scheduled_renders: BTreeMap<u64, RenderedComponent>,
    patch_authority: BTreeMap<u64, MountPatchAuthority>,
    checkpoint: Option<CompositionCheckpoint>,
}

fn extend_installed_contributions(
    resolution: &mut ResolvedShellPackage,
    manifests: &BTreeMap<String, nickel_core::package_composition::ShellPackageComposition>,
) -> Result<(), String> {
    use nickel_core::package_composition::{MAX_RESOLVED_CONTRIBUTIONS, ResolvedContribution};
    let mut count = resolution
        .contributions
        .values()
        .map(Vec::len)
        .sum::<usize>();
    for package in manifests.values() {
        if resolution
            .inheritance_chain
            .iter()
            .any(|owner| owner.id == package.id)
        {
            continue;
        }
        // Validate the provider's own dependency requirements/replacements too;
        // they never replace the active shell's public contracts.
        let own = resolve_shell_package(manifests, &package.id)
            .map_err(|e| format!("contributor composition failed: {e:?}"))?;
        for entry in &package.contributions {
            count += 1;
            if count > MAX_RESOLVED_CONTRIBUTIONS {
                return Err("too many installed contributions".into());
            }
            resolution
                .contributions
                .entry(entry.collection.clone())
                .or_default()
                .push(ResolvedContribution {
                    collection: entry.collection.clone(),
                    id: entry.id.clone(),
                    implementation: entry.implementation.clone(),
                    priority: entry.priority,
                    contributed_by: own.active.clone(),
                });
        }
    }
    for entries in resolution.contributions.values_mut() {
        entries.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.contributed_by.id.cmp(&b.contributed_by.id))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    Ok(())
}

fn package_graph(
    package: &PluginPackage,
    resolution: &ResolvedShellPackage,
) -> Result<JsxModuleGraph, String> {
    let composition = package
        .manifest
        .composition
        .as_ref()
        .ok_or("package has no composition")?;
    let mut exports = BTreeMap::new();
    let mut keys = BTreeMap::new();
    for implementation in composition
        .exports
        .values()
        .chain(composition.replaces.values())
        .chain(
            composition
                .contributions
                .iter()
                .map(|entry| &entry.implementation),
        )
    {
        if !keys.contains_key(implementation) {
            let key = format!("owned.component.{}", keys.len());
            exports.insert(key.clone(), implementation.clone());
            keys.insert(implementation.clone(), key);
        }
    }
    exports.extend(composition.exports.clone());
    exports.extend(composition.replaces.clone());
    let contributions = resolution.contributions.iter().map(|(collection, entries)| (
        collection.clone(), Value::Array(entries.iter().map(|entry| serde_json::json!({
            "id":entry.id,"provider":entry.contributed_by.id,"version":entry.contributed_by.version.to_string(),
            "key":format!("{}/{}@{}/{}/{}", entry.collection, entry.contributed_by.id,
                entry.contributed_by.version, entry.id, entry.implementation)
        })).collect()))).collect::<serde_json::Map<_,_>>();
    JsxModuleGraph::new(
        &package.manifest.entry,
        package
            .modules
            .iter()
            .filter(|module| module.path != package.manifest.entry)
            .map(|module| ModuleSource {
                path: &module.path,
                source: &module.source,
            })
            .chain(std::iter::once(ModuleSource {
                path: &package.manifest.entry,
                source: &package.source,
            })),
    )?
    .with_public_exports(&exports)
    .map(|graph| {
        graph.with_component_bridge(
            Value::Object(contributions),
            Value::Object(serde_json::Map::new()),
        )
    })
}

impl ShellCompositionRuntime {
    fn advance_mount_generation_grants(
        &mut self,
        id: u64,
        previous_generation: u64,
        generation: u64,
    ) {
        for grant in self.callbacks.values_mut() {
            if grant.receiver == id && grant.generation == previous_generation {
                grant.generation = generation;
            }
            if grant.source.mount == id && grant.source.generation == previous_generation {
                grant.source.generation = generation;
            }
        }
        for grant in self.children.values_mut() {
            if grant.receiver == id && grant.generation == previous_generation {
                grant.generation = generation;
            }
            if grant.source == id && grant.source_generation == previous_generation {
                grant.source_generation = generation;
            }
        }
    }

    /// Atomically validates and admits a trusted development catalog. Candidate
    /// code is first evaluated in isolated contexts. The accepted graph remains
    /// untouched if validation or evaluation fails.
    pub fn reload_catalog(
        &mut self,
        catalog: &BTreeMap<String, PluginPackage>,
        signatures: &BTreeMap<PackageIdentity, PackageReloadSignatures>,
    ) -> Result<CompositionReload, String> {
        if self.checkpoint.is_some() {
            return Err("cannot hot reload during a composition transaction".into());
        }
        let snapshots = self
            .packages
            .iter()
            .map(|(owner, package)| (owner.clone(), package.data.as_ref().clone()))
            .collect::<BTreeMap<_, _>>();
        // This performs manifest, ancestry, composition, capability, module and
        // eager module evaluation checks without mutating the accepted host.
        let mut candidate = Self::new(catalog, &self.resolution.active.id, &snapshots)?;
        let old_owners = self.packages.keys().cloned().collect::<Vec<_>>();
        let changed = old_owners
            .iter()
            .filter(|owner| {
                self.packages
                    .get(*owner)
                    .zip(candidate.packages.get(*owner))
                    .is_none_or(|(old, new)| {
                        old.source_digest != new.source_digest || old.manifest != new.manifest
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        let affected_roots = self
            .mounts
            .iter()
            .filter(|(_, mount)| changed.contains(&mount.reference.owner))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut result = CompositionReload::default();

        // Exercise every affected production root against the isolated graph,
        // including its real props and cross-package expansion, before any
        // accepted context is touched.
        for id in &affected_roots {
            let state = &self.mounts[id];
            let replacement = candidate
                .packages
                .get(&state.reference.owner)
                .ok_or("candidate removed a mounted package")?;
            let reference = ComponentReference {
                runtime: candidate.id,
                owner: state.reference.owner.clone(),
                implementation: state.reference.implementation.clone(),
                incarnation: replacement.incarnation,
            };
            let mount = candidate.mount(&reference)?;
            candidate.render_expanded(&mount, &state.props, |_| Ok(()))?;
        }

        for owner in &changed {
            let compatible = signatures.get(owner).is_some_and(|set| {
                set.previous == set.replacement
                    && self.packages.get(owner).is_some_and(|old| {
                        candidate
                            .packages
                            .get(owner)
                            .is_some_and(|new| old.manifest == new.manifest)
                    })
            });
            if compatible {
                let package = &catalog[&owner.id];
                let graph = package_graph(package, &candidate.resolution)?;
                self.packages
                    .get_mut(owner)
                    .unwrap()
                    .runtime
                    .borrow_mut()
                    .hot_install_modules(&graph, &signatures[owner].replacement)?;
                let replacement = candidate.packages.remove(owner).unwrap();
                let current = self.packages.get_mut(owner).unwrap();
                current.source_digest = replacement.source_digest;
                current.manifest = replacement.manifest;
                current.exports = replacement.exports;
                current.assets = replacement.assets;
                result.preserved_packages.push(owner.clone());
            } else {
                // Dropping the old context performs effect/subscription cleanup;
                // all typed authorities sourced by it are retired below.
                let surfaces = self
                    .mounts
                    .iter()
                    .filter(|(_, mount)| &mount.reference.owner == owner)
                    .map(|(id, _)| surface(*id))
                    .collect::<Vec<_>>();
                if let Some(old) = self.packages.get_mut(owner) {
                    for surface in surfaces {
                        old.runtime.borrow_mut().drop_surface(&surface)?;
                    }
                }
                let mut replacement = candidate
                    .packages
                    .remove(owner)
                    .ok_or("candidate package missing")?;
                replacement.incarnation = NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed);
                self.packages.insert(owner.clone(), replacement);
                result.restarted_packages.push(owner.clone());
            }
        }
        self.resolution = candidate.resolution;
        self.callbacks
            .retain(|_, grant| !changed.contains(&grant.source.owner));
        self.children.retain(|_, grant| {
            self.mounts.contains_key(&grant.receiver) && self.mounts.contains_key(&grant.source)
        });
        self.effects
            .retain(|effect| !changed.contains(&effect.owner));
        self.scheduled_renders
            .retain(|mount, _| !affected_roots.contains(mount));

        for id in affected_roots {
            let state = self
                .mounts
                .get(&id)
                .cloned()
                .ok_or("reload mount disappeared")?;
            let package = self
                .packages
                .get_mut(&state.reference.owner)
                .ok_or("reload owner disappeared")?;
            package
                .runtime
                .borrow_mut()
                .register_component_surface(
                    &surface(id),
                    package
                        .exports
                        .get(&state.reference.implementation)
                        .ok_or("reloaded implementation is unpublished")?,
                    false,
                )
                .or_else(|error| {
                    if result.preserved_packages.contains(&state.reference.owner) {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })?;
            self.next_generation = self
                .next_generation
                .checked_add(1)
                .ok_or("component generation exhausted")?;
            self.mounts.get_mut(&id).unwrap().generation = self.next_generation;
            let mount = ComponentMount {
                runtime: self.id,
                id,
            };
            let props = state.props;
            let rendered = self.render_expanded(&mount, &props, |_| Ok(()))?;
            result.patches.push(ReloadedMountPatch {
                mount,
                patch: NativePatchEnvelope {
                    version: 1,
                    operations: vec![NativePatchOperation::ReplaceSubtree {
                        target: "root".into(),
                        node: rendered.node,
                    }],
                    counters: NativePatchCounters {
                        nodes_visited: 1,
                        nodes_mutated: 1,
                        local_materializations: 1,
                        expansion_nodes: 1,
                        tree_bytes: 0,
                    },
                },
                events: rendered.events,
                generation: rendered.generation,
            });
        }
        self.publish_contribution_catalog()?;
        Ok(result)
    }
    /// Catalog contents and per-package capability snapshots must already have
    /// passed the host's installation/approval checks. No dependency receives
    /// the active shell's snapshot or grants implicitly.
    pub fn new(
        catalog: &BTreeMap<String, PluginPackage>,
        active: &str,
        snapshots: &BTreeMap<PackageIdentity, Value>,
    ) -> Result<Self, String> {
        Self::new_with_contexts(catalog, active, snapshots, &BTreeMap::new())
    }

    pub fn new_with_contexts(
        catalog: &BTreeMap<String, PluginPackage>,
        active: &str,
        snapshots: &BTreeMap<PackageIdentity, Value>,
        contexts: &BTreeMap<PackageIdentity, ProviderContext>,
    ) -> Result<Self, String> {
        let manifests = catalog
            .iter()
            .filter(|(_, package)| package.manifest.composition.is_some())
            .map(|(id, package)| {
                let composition = package.manifest.composition.clone().unwrap();
                if package.manifest.id != composition.id
                    || package.manifest.version.as_deref() != Some(&composition.version)
                {
                    return Err("installed composition identity mismatch".into());
                }
                Ok((id.clone(), composition))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut resolution = resolve_shell_package(&manifests, active)
            .map_err(|error| format!("composition failed: {error:?}"))?;
        extend_installed_contributions(&mut resolution, &manifests)?;
        let mut host = Self {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            resolution,
            packages: BTreeMap::new(),
            mounts: BTreeMap::new(),
            next_mount: 0,
            next_generation: 0,
            effects: Vec::new(),
            nested_mounts: BTreeMap::new(),
            callbacks: BTreeMap::new(),
            children: BTreeMap::new(),
            next_callback: 0,
            callback_depth: 0,
            scheduled_renders: BTreeMap::new(),
            patch_authority: BTreeMap::new(),
            checkpoint: None,
        };
        let contribution_catalog = host
            .resolution
            .contributions
            .iter()
            .map(|(collection, entries)| {
                (
                    collection.clone(),
                    Value::Array(
                        entries
                            .iter()
                            .map(|entry| {
                                serde_json::json!({
                                    "id":entry.id, "provider":entry.contributed_by.id,
                                    "version":entry.contributed_by.version.to_string(),
                                    "key":host.contribution_key(entry),
                                })
                            })
                            .collect(),
                    ),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let mut asset_count = 0usize;
        let mut source_bytes = 0usize;
        let mut owners = host.resolution.inheritance_chain.clone();
        for (id, package) in &manifests {
            if !owners.iter().any(|owner| owner.id == *id) {
                let provider = resolve_shell_package(&manifests, &package.id)
                    .map_err(|e| format!("contributor composition failed: {e:?}"))?;
                owners.push(provider.active);
            }
        }
        for owner in owners {
            let package = &catalog[&owner.id];
            if package.modules.len() > nickel_core::plugins::MAX_PLUGIN_MODULES {
                return Err("too many package source modules".into());
            }
            source_bytes = source_bytes
                .checked_add(package.source.len())
                .ok_or("composition source size overflow")?;
            for module in &package.modules {
                source_bytes = source_bytes
                    .checked_add(module.source.len())
                    .ok_or("composition source size overflow")?;
            }
            if source_bytes > 16 * 1024 * 1024 {
                return Err("composition source exceeds size limit".into());
            }
            if let Some(context) = contexts.get(&owner) {
                if context.owner != owner
                    || context.package.source_digest != catalog[&owner.id].source_digest()
                    || context.package.manifest != catalog[&owner.id].manifest
                {
                    return Err("provider context identity mismatch".into());
                }
                let mut package = context.package.clone();
                if let Some(data) = snapshots.get(&owner) {
                    bounded_json(data)?;
                    if !data.is_object() {
                        return Err("package snapshot must be an object".into());
                    }
                    package.data = std::rc::Rc::new(data.clone());
                }
                host.packages.insert(owner.clone(), package);
                continue;
            }
            let composition = &manifests[&owner.id];
            let mut exports = BTreeMap::new();
            let mut export_keys = BTreeMap::new();
            for implementation in composition
                .exports
                .values()
                .chain(composition.replaces.values())
                .chain(
                    composition
                        .contributions
                        .iter()
                        .map(|entry| &entry.implementation),
                )
            {
                if !export_keys.contains_key(implementation) {
                    let key = format!("owned.component.{}", export_keys.len());
                    exports.insert(key.clone(), implementation.clone());
                    export_keys.insert(implementation.clone(), key);
                }
            }
            exports.extend(composition.exports.clone());
            exports.extend(composition.replaces.clone());
            // Public exports always resolve through the invoking host. An
            // imported context must not retain its original shell's selection.
            let local_components = serde_json::Map::new();
            // Only local modules enter this context. Cross-package reuse is
            // mediated by Rust references below, never by copying foreign code.
            let graph = JsxModuleGraph::new(
                &package.manifest.entry,
                package
                    .modules
                    .iter()
                    .filter(|module| module.path != package.manifest.entry)
                    .map(|module| ModuleSource {
                        path: &module.path,
                        source: &module.source,
                    })
                    .chain(std::iter::once(ModuleSource {
                        path: &package.manifest.entry,
                        source: &package.source,
                    })),
            )?
            .with_public_exports(&exports)?
            .with_component_bridge(
                Value::Object(contribution_catalog.clone()),
                Value::Object(local_components),
            );
            let data = snapshots
                .get(&owner)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            bounded_json(&data)?;
            if !data.is_object() {
                return Err("package snapshot must be an object".into());
            }
            let mut runtime = JsxRuntime::new_modules(&graph, Some(&data.to_string()))?;
            runtime.set_diagnostic_owner(&owner.id)?;
            runtime.set_capability_store(&package.manifest.capabilities, &data)?;
            let runtime = std::rc::Rc::new(std::cell::RefCell::new(runtime));
            let assets = package
                .images
                .keys()
                .map(|name| {
                    asset_count += 1;
                    (
                        name.clone(),
                        format!("composition.asset.{}.{asset_count}", host.id),
                    )
                })
                .collect();
            host.packages.insert(
                owner.clone(),
                PackageRuntime {
                    runtime,
                    data: std::rc::Rc::new(data),
                    exports: export_keys,
                    assets,
                    incarnation: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
                    source_digest: package.source_digest(),
                    manifest: package.manifest.clone(),
                },
            );
            host.drain_effects(&owner, 0, 0, false)?;
        }
        host.publish_contribution_catalog()?;
        Ok(host)
    }

    /// All live contexts, including independently installed contributors, once each.
    pub fn participating_owners(&self) -> impl Iterator<Item = &PackageIdentity> {
        self.resolution
            .inheritance_chain
            .iter()
            .filter(|owner| self.packages.contains_key(*owner))
            .chain(
                self.packages
                    .keys()
                    .filter(|owner| !self.resolution.inheritance_chain.contains(owner)),
            )
    }

    pub fn provider_context(&self, owner: &PackageIdentity) -> Result<ProviderContext, String> {
        Ok(ProviderContext {
            owner: owner.clone(),
            package: self
                .packages
                .get(owner)
                .ok_or("retired package owner")?
                .clone(),
        })
    }

    /// Reconcile already admitted native provider contexts. The active shell's
    /// inheritance and replacement selections remain unchanged.
    pub fn sync_contributors(
        &mut self,
        catalog: &BTreeMap<String, PluginPackage>,
        contexts: &BTreeMap<PackageIdentity, ProviderContext>,
    ) -> Result<bool, String> {
        let mut source_bytes = 0usize;
        for package in catalog
            .values()
            .filter(|package| package.manifest.composition.is_some())
        {
            source_bytes = source_bytes
                .checked_add(package.source.len())
                .ok_or("composition source size overflow")?;
            for module in &package.modules {
                source_bytes = source_bytes
                    .checked_add(module.source.len())
                    .ok_or("composition source size overflow")?;
            }
        }
        if source_bytes > 16 * 1024 * 1024 {
            return Err("composition source exceeds size limit".into());
        }
        let manifests = catalog
            .iter()
            .filter_map(|(id, p)| p.manifest.composition.clone().map(|c| (id.clone(), c)))
            .collect();
        let mut next = resolve_shell_package(&manifests, &self.resolution.active.id)
            .map_err(|e| format!("composition failed: {e:?}"))?;
        if next.exports != self.resolution.exports
            || next.inheritance_chain != self.resolution.inheritance_chain
        {
            return Err("shell ancestry changed during contributor reconciliation".into());
        }
        extend_installed_contributions(&mut next, &manifests)?;
        let owners = manifests
            .values()
            .map(|package| {
                resolve_shell_package(&manifests, &package.id)
                    .map(|resolution| resolution.active)
                    .map_err(|e| format!("contributor composition failed: {e:?}"))
            })
            .collect::<Result<std::collections::BTreeSet<_>, String>>()?;
        for owner in &owners {
            if let Some(existing) = self.packages.get(owner) {
                let package = &catalog[&owner.id];
                if existing.source_digest != package.source_digest()
                    || existing.manifest != package.manifest
                {
                    return Err("running contributor code or grants changed".into());
                }
            }
            if !self.packages.contains_key(owner) && !contexts.contains_key(owner) {
                return Err("admitted contributor context is unavailable".into());
            }
        }
        let changed = next.contributions != self.resolution.contributions
            || owners != self.packages.keys().cloned().collect();
        // Removed owners remain live until the host has admitted the consumer
        // tree produced from this catalog. Their mounts and callback grants
        // are still required to validate that transition; the host retires
        // them explicitly after native application succeeds.
        for owner in owners {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.packages.entry(owner.clone())
            {
                let context = &contexts[&owner];
                if context.owner != owner
                    || context.package.source_digest != catalog[&owner.id].source_digest()
                    || context.package.manifest != catalog[&owner.id].manifest
                {
                    return Err("provider context identity mismatch".into());
                }
                entry.insert(context.package.clone());
            }
        }
        self.resolution = next;
        self.publish_contribution_catalog()?;
        Ok(changed)
    }

    fn contribution_catalog(&self) -> Value {
        let catalog=self.resolution.contributions.iter().map(|(collection,entries)|(collection.clone(),Value::Array(entries.iter().filter(|e|self.packages.contains_key(&e.contributed_by)).map(|entry|serde_json::json!({"id":entry.id,"provider":entry.contributed_by.id,"version":entry.contributed_by.version.to_string(),"key":self.contribution_key(entry)})).collect()))).collect::<serde_json::Map<_,_>>();
        Value::Object(catalog)
    }

    fn publish_contribution_catalog(&self) -> Result<(), String> {
        let catalog = self.contribution_catalog();
        for package in self.packages.values() {
            package
                .runtime
                .borrow_mut()
                .set_contribution_catalog(&catalog)?;
        }
        Ok(())
    }

    /// Trusted native adapters may share the owner context without creating a
    /// second package lifecycle. Guest code never receives this Rust reference.
    pub fn shared_owner_runtime(
        &self,
        owner: &PackageIdentity,
    ) -> Result<std::rc::Rc<std::cell::RefCell<JsxRuntime>>, String> {
        Ok(self
            .packages
            .get(owner)
            .ok_or("retired package owner")?
            .runtime
            .clone())
    }

    /// Service one live owner only after the complete composition transaction
    /// has finished. Host scheduling chooses owners fairly and admits any
    /// resulting component changes/effects through the normal validated path.
    pub fn service_owner_platform_tasks(
        &mut self,
        owner: &PackageIdentity,
    ) -> Result<usize, String> {
        if self.transaction_pending() {
            return Err("cannot service platform tasks during a composition transaction".into());
        }
        self.shared_owner_runtime(owner)?
            .borrow_mut()
            .service_platform_tasks()
    }

    /// Read native scheduling state without selecting a surface, executing
    /// JavaScript, allocating an owner list, or observing provisional work.
    pub fn platform_maintenance_pending(&self) -> bool {
        !self.transaction_pending()
            && self
                .packages
                .values()
                .any(|package| package.runtime.borrow().platform_maintenance_pending())
    }

    pub fn owner_platform_maintenance_pending(
        &self,
        owner: &PackageIdentity,
    ) -> Result<bool, String> {
        let package = self.packages.get(owner).ok_or("retired package owner")?;
        Ok(!self.transaction_pending() && package.runtime.borrow().platform_maintenance_pending())
    }

    /// Service at most one pending owner, resuming after the previous selection.
    /// The cursor is a hint only: owner retirement changes the live inventory.
    pub fn service_next_pending_platform_owner(
        &mut self,
        cursor: &mut usize,
    ) -> Result<Option<(PackageIdentity, usize)>, String> {
        if self.transaction_pending() {
            return Err("cannot service platform tasks during a composition transaction".into());
        }
        let owners = self.participating_owners().collect::<Vec<_>>();
        if owners.is_empty() {
            return Ok(None);
        }
        let start = *cursor % owners.len();
        let selected = (0..owners.len()).find_map(|offset| {
            let index = (start + offset) % owners.len();
            self.packages
                .get(owners[index])
                .filter(|package| package.runtime.borrow().platform_maintenance_pending())
                .map(|_| (index, owners[index].clone()))
        });
        let Some((index, owner)) = selected else {
            return Ok(None);
        };
        let next = (index + 1) % owners.len();
        let count = self.service_owner_platform_tasks(&owner)?;
        *cursor = next;
        Ok(Some((owner, count)))
    }

    pub fn mount_work_pending(&self, root: &ComponentMount) -> Result<bool, String> {
        if self.transaction_pending() {
            return Err("cannot query idle work during a composition transaction".into());
        }
        self.validate_mount(root)?;
        for id in std::iter::once(root.id).chain(
            self.nested_mounts
                .iter()
                .filter(|((owner, _), _)| *owner == root.id)
                .map(|(_, mount)| mount.id),
        ) {
            let mount = self.mounts.get(&id).ok_or("retired component mount")?;
            if self
                .shared_owner_runtime(&mount.reference.owner)?
                .borrow_mut()
                .surface_work_pending(&surface(id))?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn resolution(&self) -> &ResolvedShellPackage {
        &self.resolution
    }

    /// Number of live native component mounts owned by this composition host.
    /// This is payload-free lifecycle telemetry used to prove that native-only
    /// interaction and warm surface activation do not create guest mounts.
    pub fn mount_count(&self) -> usize {
        self.mounts.len()
    }

    pub fn component(&self, contract: &str) -> Option<ComponentReference> {
        self.resolution
            .exports
            .get(contract)
            .map(|export| ComponentReference {
                runtime: self.id,
                owner: export.implemented_by.clone(),
                implementation: export.implementation.clone(),
                incarnation: self
                    .packages
                    .get(&export.implemented_by)
                    .map_or(0, |p| p.incarnation),
            })
    }

    fn contribution_key(
        &self,
        entry: &nickel_core::package_composition::ResolvedContribution,
    ) -> String {
        format!(
            "{}/{}@{}/{}/{}",
            entry.collection,
            entry.contributed_by.id,
            entry.contributed_by.version,
            entry.id,
            self.packages
                .get(&entry.contributed_by)
                .map_or(0, |package| package.incarnation)
        )
    }

    fn contribution(&self, key: &str) -> Option<ComponentReference> {
        self.resolution
            .contributions
            .values()
            .flatten()
            .find(|entry| {
                self.contribution_key(entry) == key
                    && self.packages.contains_key(&entry.contributed_by)
            })
            .map(|entry| ComponentReference {
                runtime: self.id,
                owner: entry.contributed_by.clone(),
                implementation: entry.implementation.clone(),
                incarnation: self.packages[&entry.contributed_by].incarnation,
            })
    }

    pub fn contributions(&self, collection: &str) -> Vec<ComponentReference> {
        self.resolution
            .contributions
            .get(collection)
            .into_iter()
            .flatten()
            .filter(|entry| self.packages.contains_key(&entry.contributed_by))
            .map(|entry| ComponentReference {
                runtime: self.id,
                owner: entry.contributed_by.clone(),
                implementation: entry.implementation.clone(),
                incarnation: self.packages[&entry.contributed_by].incarnation,
            })
            .collect()
    }

    fn registered_page(&self, provider: &str, id: &str) -> Option<ComponentReference> {
        let (owner, package) = self
            .packages
            .iter()
            .find(|(owner, _)| owner.id == provider)?;
        package
            .runtime
            .borrow()
            .has_registered_page(id)
            .then(|| ComponentReference {
                runtime: self.id,
                owner: owner.clone(),
                implementation: format!("@settings-page/{id}"),
                incarnation: package.incarnation,
            })
    }

    pub fn mount(&mut self, reference: &ComponentReference) -> Result<ComponentMount, String> {
        if reference.runtime != self.id
            || self
                .packages
                .get(&reference.owner)
                .is_none_or(|p| p.incarnation != reference.incarnation)
        {
            return Err("foreign or retired component reference".into());
        }
        if self.mounts.len() >= MAX_MOUNTS {
            return Err("too many component mounts".into());
        }
        // Shared provider contexts can serve several native composition hosts;
        // surface IDs must remain unique across them to keep hooks isolated.
        let id = NEXT_SHARED_MOUNT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| "component mount identity exhausted")?;
        self.next_mount = id;
        let package = self.packages.get_mut(&reference.owner).unwrap();
        let (component, registered_page) =
            if let Some(id) = reference.implementation.strip_prefix("@settings-page/") {
                if !package.runtime.borrow().has_registered_page(id) {
                    return Err("Settings page is no longer published by its owner".into());
                }
                (id, true)
            } else {
                let implementation = package
                    .exports
                    .get(&reference.implementation)
                    .ok_or("component implementation is not published by its owner")?;
                (implementation.as_str(), false)
            };
        package.runtime.borrow_mut().register_component_surface(
            &surface(id),
            component,
            registered_page,
        )?;
        self.mounts.insert(
            id,
            MountState {
                reference: reference.clone(),
                generation: 0,
                props: serde_json::json!({}),
                surface: package.data.get("surface").cloned(),
            },
        );
        Ok(ComponentMount {
            runtime: self.id,
            id,
        })
    }

    /// Replace the host-owned observation attached to one native mount.
    pub fn update_mount_surface(
        &mut self,
        mount: &ComponentMount,
        surface: Value,
    ) -> Result<bool, String> {
        self.validate_mount(mount)?;
        let state = self
            .mounts
            .get_mut(&mount.id)
            .ok_or("retired component mount")?;
        if state.surface.as_ref() == Some(&surface) {
            return Ok(false);
        }
        state.surface = Some(surface);
        Ok(true)
    }

    /// A host that is about to perform a complete mount render may consume the
    /// store notification directly instead of carrying it into the incremental
    /// event patch queue.
    pub fn consume_mount_reconciliation(&mut self, mount: &ComponentMount) -> Result<(), String> {
        self.validate_mount(mount)?;
        let owner = self.mounts[&mount.id].reference.owner.clone();
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.select_surface(&surface(mount.id))?;
        runtime.consume_reconciliation()
    }

    /// Publish one root mount's current surface authority to every component
    /// ownership boundary on that native surface and report whether a hook
    /// selector actually observed a change.
    pub fn publish_mount_surface_authority(
        &mut self,
        root: &ComponentMount,
    ) -> Result<bool, String> {
        self.validate_mount(root)?;
        let root_surface = self.mounts[&root.id]
            .surface
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let mounts = std::iter::once(root.id)
            .chain(
                self.nested_mounts
                    .iter()
                    .filter(|((owner, _), _)| *owner == root.id)
                    .map(|(_, mount)| mount.id),
            )
            .collect::<Vec<_>>();
        let mut requested = false;
        for id in mounts {
            if id != root.id {
                self.mounts.get_mut(&id).unwrap().surface = Some(root_surface.clone());
            }
            let owner = self.mounts[&id].reference.owner.clone();
            let package = self
                .packages
                .get_mut(&owner)
                .ok_or("retired component owner")?;
            let mut runtime = package.runtime.borrow_mut();
            runtime.select_surface(&surface(id))?;
            let was_requested = runtime.reconciliation_requested()?;
            runtime.set_surface_store(&surface(id), &root_surface)?;
            let now_requested = runtime.reconciliation_requested()?;
            requested |= !was_requested && now_requested;
        }
        Ok(requested)
    }

    /// Begin a bounded transaction spanning every participating owner context.
    /// IDs remain monotonic so rolled-back mount/callback handles cannot be reused.
    pub fn begin_transaction(&mut self) -> Result<(), String> {
        if self.checkpoint.is_some() {
            return Err("composition transaction already pending".into());
        }
        let mut started = Vec::new();
        for (owner, package) in &self.packages {
            if let Err(error) = package.runtime.borrow_mut().begin_transaction() {
                for owner in started {
                    let _ = self.packages[&owner]
                        .runtime
                        .borrow_mut()
                        .finish_transaction(false);
                }
                return Err(error);
            }
            started.push(owner.clone());
        }
        self.checkpoint = Some(CompositionCheckpoint {
            mounts: self.mounts.clone(),
            effects: self.effects.clone(),
            nested_mounts: self.nested_mounts.clone(),
            callbacks: self.callbacks.clone(),
            children: self.children.clone(),
            data: self
                .packages
                .iter()
                .map(|(owner, package)| (owner.clone(), package.data.clone()))
                .collect(),
            scheduled_renders: self.scheduled_renders.clone(),
            patch_authority: self.patch_authority.clone(),
        });
        Ok(())
    }

    pub fn transaction_pending(&self) -> bool {
        self.checkpoint.is_some()
    }

    pub fn finish_transaction(&mut self, accepted: bool) -> Result<(), String> {
        let checkpoint = self
            .checkpoint
            .take()
            .ok_or("composition transaction is unavailable")?;
        let mut failure = None;
        for package in self.packages.values() {
            if let Err(error) = package.runtime.borrow_mut().finish_transaction(accepted) {
                failure = Some(error);
            }
        }
        if !accepted {
            self.mounts = checkpoint.mounts;
            self.effects = checkpoint.effects;
            self.nested_mounts = checkpoint.nested_mounts;
            self.callbacks = checkpoint.callbacks;
            self.children = checkpoint.children;
            for (owner, data) in checkpoint.data {
                if let Some(package) = self.packages.get_mut(&owner) {
                    package.data = data;
                }
            }
            self.scheduled_renders = checkpoint.scheduled_renders;
            self.patch_authority = checkpoint.patch_authority;
        }
        if let Some(error) = failure {
            self.effects.clear();
            let owners = self.packages.keys().cloned().collect::<Vec<_>>();
            for owner in owners {
                self.retire(&owner);
            }
            return Err(format!("composition checkpoint failed: {error}"));
        }
        Ok(())
    }

    fn transactional<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.checkpoint.is_some() {
            return operation(self);
        }
        self.begin_transaction()?;
        let result = operation(self);
        self.finish_transaction(result.is_ok())?;
        result
    }

    pub fn render(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
    ) -> Result<RenderedComponent, String> {
        self.render_validated(mount, props, |_| Ok(()))
    }

    pub fn dispatch(
        &mut self,
        handle: &ComponentEventHandle,
        value: &Value,
    ) -> Result<RenderedComponent, String> {
        self.dispatch_validated(handle, value, |_| Ok(()))
    }

    /// Legacy/cold dispatch for a mount without admitted patch authority.
    /// Production retained mounts route through `render_mount_patched`; this
    /// path remains for initial admission, cold-oracle comparison, and newly
    /// discovered ownership boundaries. Referential no-ops still skip render.
    pub fn dispatch_scheduled(
        &mut self,
        handle: &ComponentEventHandle,
        value: &Value,
    ) -> Result<ScheduledComponentDispatch, String> {
        self.transactional(|host| {
            bounded_json(value)?;
            let mount = host
                .mounts
                .get(&handle.mount)
                .ok_or("retired component event")?;
            if handle.runtime != host.id
                || mount.generation != handle.generation
                || mount.reference.owner != handle.owner
            {
                return Err("foreign or stale component event".into());
            }
            host.render_mount_scheduled(handle.mount, serde_json::json!([[handle.action, value]]))
        })
    }

    /// Accept a production renderer's tree validation before committing hooks,
    /// handlers or effects. Rejection rolls back the existing render transaction.
    pub fn render_validated(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.transactional(|host| host.render_validated_inner(mount, props, validate))
    }

    fn render_validated_inner(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        bounded_json(props)?;
        if !props.is_object() {
            return Err("component props must be an object".into());
        }
        self.validate_mount(mount)?;
        let previous_props = std::mem::replace(
            &mut self.mounts.get_mut(&mount.id).unwrap().props,
            props.clone(),
        );
        let result = self.render_mount(mount.id, None, validate);
        if result.is_err()
            && let Some(state) = self.mounts.get_mut(&mount.id)
        {
            state.props = previous_props;
        }
        result
    }

    pub fn dispatch_validated(
        &mut self,
        handle: &ComponentEventHandle,
        value: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.transactional(|host| host.dispatch_validated_inner(handle, value, validate))
    }

    fn dispatch_validated_inner(
        &mut self,
        handle: &ComponentEventHandle,
        value: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        bounded_json(value)?;
        let mount = self
            .mounts
            .get(&handle.mount)
            .ok_or("retired component event")?;
        if handle.runtime != self.id
            || mount.generation != handle.generation
            || mount.reference.owner != handle.owner
        {
            return Err("foreign or stale component event".into());
        }
        self.render_mount(
            handle.mount,
            Some(serde_json::json!([[handle.action, value]])),
            validate,
        )
    }

    /// Expand public component mount requests through their owning contexts.
    /// Data props cross as bounded snapshots. Void callable props are routed
    /// through Rust-owned grants back to their originating surface/owner.
    pub fn render_expanded(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.render_expanded_validated(mount, props, validate)
            .map(|(rendered, ())| rendered)
    }

    pub fn render_expanded_validated<T>(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<(RenderedComponent, T), String> {
        self.render_expanded_validated_with_generation(mount, props, |value, _| validate(value))
    }

    pub fn render_expanded_validated_with_generation<T>(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value, u64) -> Result<T, String>,
    ) -> Result<(RenderedComponent, T), String> {
        self.transactional(|host| {
            host.validate_mount(mount)?;
            if let Some(surface) = host.mounts[&mount.id].surface.clone() {
                for package in host.packages.values_mut() {
                    if package.data.get("surface") == Some(&surface) {
                        continue;
                    }
                    std::rc::Rc::make_mut(&mut package.data)
                        .as_object_mut()
                        .ok_or("package snapshot must be an object")?
                        .insert("surface".into(), surface.clone());
                }
            }
            let rendered = host.render(mount, props)?;
            host.expand_rendered(mount.id, rendered, validate)
        })
    }

    pub fn dispatch_expanded(
        &mut self,
        root: &ComponentMount,
        handle: &ComponentEventHandle,
        value: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.transactional(|host| {
            host.validate_mount(root)?;
            host.dispatch(handle, value)?;
            let props = host.mounts[&root.id].props.clone();
            host.render_expanded(root, &props, validate)
        })
    }

    /// Leave hooks/handlers/effects provisional until the native adapter has
    /// approved the entire effect batch. The adapter must finish_transaction.
    pub fn dispatch_expanded_pending(
        &mut self,
        root: &ComponentMount,
        handle: &ComponentEventHandle,
        value: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.begin_transaction()?;
        let result = self.dispatch_expanded(root, handle, value, validate);
        if result.is_err() {
            self.finish_transaction(false)?;
        }
        result
    }

    /// Dispatch every handler emitted by one native UI transition against the
    /// render generation that produced it, then rebuild the expanded root once.
    /// Handles retain their package owner and mount, so a transition containing
    /// both a blur callback and a click cannot cross an ownership boundary.
    pub fn dispatch_expanded_batch_pending(
        &mut self,
        root: &ComponentMount,
        events: &[(ComponentEventHandle, Value)],
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.dispatch_expanded_batch_pending_validated(root, events, validate)
            .map(|(rendered, ())| rendered)
    }

    pub fn dispatch_expanded_batch_pending_validated<T>(
        &mut self,
        root: &ComponentMount,
        events: &[(ComponentEventHandle, Value)],
        validate: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<(RenderedComponent, T), String> {
        self.begin_transaction()?;
        let result = (|| {
            self.validate_mount(root)?;
            for (handle, value) in events {
                bounded_json(value)?;
                let mount = self
                    .mounts
                    .get(&handle.mount)
                    .ok_or("retired component event")?;
                if handle.runtime != self.id
                    || mount.generation != handle.generation
                    || mount.reference.owner != handle.owner
                {
                    return Err("foreign or stale component event".into());
                }
            }

            let mut offset = 0;
            while offset < events.len() {
                let mount = events[offset].0.mount;
                let mut end = offset + 1;
                while end < events.len() && events[end].0.mount == mount {
                    end += 1;
                }
                let batch = events[offset..end]
                    .iter()
                    .map(|(handle, value)| serde_json::json!([handle.action, value]))
                    .collect::<Vec<_>>();
                self.render_mount(mount, Some(Value::Array(batch)), |_| Ok(()))?;
                offset = end;
            }

            let props = self.mounts[&root.id].props.clone();
            self.render_expanded_validated(root, &props, validate)
        })();
        if result.is_err() {
            self.finish_transaction(false)?;
        }
        result
    }

    /// Scheduled production bridge. A batch whose handlers do not change any
    /// hook value retains the caller's accepted expanded tree and event table.
    /// Every mount in an admitted expanded tree must retain component-owned
    /// typed patch authority. Initial and newly discovered boundary admission
    /// happens through `render_expanded`; losing authority afterwards is a
    /// contained error rather than permission to fall back to a complete tree.
    pub fn dispatch_expanded_batch_scheduled_pending_validated<T>(
        &mut self,
        root: &ComponentMount,
        events: &[(ComponentEventHandle, Value)],
        accepted_events: &BTreeMap<u64, ComponentEventHandle>,
        accepted_source: &Value,
        mut validate: impl FnMut(
            &NativePatchEnvelope,
            &BTreeMap<u64, ComponentEventHandle>,
            u64,
        ) -> Result<T, String>,
    ) -> Result<ScheduledExpandedBatch<T>, String> {
        self.dispatch_expanded_batch_scheduled_pending_with_validator(
            root,
            events,
            accepted_events,
            accepted_source,
            &mut validate,
            true,
        )
    }

    /// Atomically publish one owner's host snapshot and reconcile only the
    /// mounted components whose versioned selectors observed a change. The
    /// accepted expanded tree remains native-owned and crosses this boundary
    /// solely as a typed patch; rejecting that patch restores both the package
    /// stores and the host snapshot.
    pub fn update_snapshot_and_reconcile_expanded_pending_validated<T>(
        &mut self,
        owner: &PackageIdentity,
        data: &Value,
        root: &ComponentMount,
        accepted_events: &BTreeMap<u64, ComponentEventHandle>,
        accepted_source: &Value,
        mut validate: impl FnMut(
            &NativePatchEnvelope,
            &BTreeMap<u64, ComponentEventHandle>,
            u64,
        ) -> Result<T, String>,
    ) -> Result<ScheduledExpandedBatch<T>, String> {
        self.begin_transaction()?;
        if let Err(error) = self.update_snapshot(owner, data) {
            self.finish_transaction(false)?;
            return Err(error);
        }
        self.dispatch_expanded_batch_scheduled_pending_with_validator(
            root,
            &[],
            accepted_events,
            accepted_source,
            &mut validate,
            false,
        )
    }

    fn dispatch_expanded_batch_scheduled_pending_with_validator<T>(
        &mut self,
        root: &ComponentMount,
        events: &[(ComponentEventHandle, Value)],
        accepted_events: &BTreeMap<u64, ComponentEventHandle>,
        accepted_source: &Value,
        validate: &mut NativePatchValidator<'_, T>,
        begin_transaction: bool,
    ) -> Result<ScheduledExpandedBatch<T>, String> {
        if begin_transaction {
            self.begin_transaction()?;
        } else if !self.transaction_pending() {
            return Err("composition transaction is unavailable".into());
        }
        let mut rejected_native = None;
        let result = (|| {
            self.validate_mount(root)?;
            let owned_mounts = std::iter::once(root.id)
                .chain(
                    self.nested_mounts
                        .iter()
                        .filter(|((owner, _), _)| *owner == root.id)
                        .map(|(_, mount)| mount.id),
                )
                .collect::<std::collections::BTreeSet<_>>();
            if let Some(mount) = owned_mounts
                .iter()
                .find(|mount| !self.patch_authority.contains_key(mount))
            {
                return Err(format!(
                    "admitted composition mount {mount} lost native patch authority"
                ));
            }
            for (handle, value) in events {
                bounded_json(value)?;
                let mount = self
                    .mounts
                    .get(&handle.mount)
                    .ok_or("retired component event")?;
                if handle.runtime != self.id
                    || !owned_mounts.contains(&handle.mount)
                    || mount.generation != handle.generation
                    || mount.reference.owner != handle.owner
                {
                    return Err("foreign or stale component event".into());
                }
            }

            let generations_before = self
                .mounts
                .iter()
                .filter(|(id, _)| owned_mounts.contains(id))
                .map(|(id, mount)| (*id, mount.generation))
                .collect::<BTreeMap<_, _>>();
            let mut rendered_mounts = BTreeMap::new();
            let mut patched_mounts = BTreeMap::new();
            let mut dirty_components = BTreeMap::new();
            let mut reconciliation_requested = false;
            if events.is_empty() {
                let mount_ids = owned_mounts.iter().copied().collect::<Vec<_>>();
                for mount in mount_ids {
                    let outcome = self.render_mount_patched(mount, Vec::new())?;
                    dirty_components.insert(mount, outcome.dirty_components);
                    if let Some(patch) = outcome.patch {
                        patched_mounts.insert(mount, patch);
                    }
                    rendered_mounts.append(&mut self.scheduled_renders);
                    reconciliation_requested |= outcome.reconciliation_requested;
                }
            }
            let mut offset = 0;
            while offset < events.len() {
                let mount = events[offset].0.mount;
                let mut end = offset + 1;
                while end < events.len() && events[end].0.mount == mount {
                    end += 1;
                }
                let batch = events[offset..end]
                    .iter()
                    .map(|(handle, value)| serde_json::json!([handle.slot, value]))
                    .collect::<Vec<_>>();
                let outcome = self.render_mount_slots_patched(mount, batch)?;
                dirty_components.insert(mount, outcome.dirty_components);
                if let Some(patch) = outcome.patch {
                    patched_mounts.insert(mount, patch);
                }
                rendered_mounts.append(&mut self.scheduled_renders);
                reconciliation_requested |= outcome.reconciliation_requested;
                offset = end;
            }

            let changed = self
                .mounts
                .iter()
                .filter(|(id, _)| owned_mounts.contains(id))
                .filter(|(id, mount)| generations_before.get(id) != Some(&mount.generation))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            if changed.is_empty() {
                return Ok(ScheduledExpandedBatch::Unchanged);
            }
            let dirty = changed
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            let mut boundaries = changed
                .iter()
                .map(|mount_id| {
                    if *mount_id == root.id {
                        return Ok(("root".to_owned(), *mount_id, false));
                    }
                    self.nested_mounts
                        .iter()
                        .find(|((owner, _), mount)| *owner == root.id && mount.id == *mount_id)
                        .map(|((_, path), _)| (path.clone(), *mount_id, true))
                        .ok_or("dirty composition mount has no ownership boundary")
                })
                .collect::<Result<Vec<_>, _>>()?;
            boundaries.sort_by(|left, right| left.0.cmp(&right.0));
            boundaries = boundaries
                .clone()
                .into_iter()
                .filter(|candidate| {
                    !boundaries.iter().any(|ancestor| {
                        ancestor.0 != candidate.0
                            && candidate
                                .0
                                .strip_prefix(&ancestor.0)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                    })
                })
                .collect();
            let mut retained_events = accepted_events
                .iter()
                .filter(|(_, handle)| self.mounts.contains_key(&handle.mount))
                .map(|(token, handle)| (*token, handle.clone()))
                .collect::<BTreeMap<_, _>>();
            for handle in retained_events.values_mut() {
                if patched_mounts.contains_key(&handle.mount) {
                    handle.generation = self.mounts[&handle.mount].generation;
                }
            }
            let next_event = retained_events
                .last_key_value()
                .map_or(0, |(token, _)| token.saturating_add(1));
            let mut expansion = ExpansionState {
                root: root.id,
                events: std::mem::take(&mut retained_events),
                visited: std::collections::BTreeSet::new(),
                native_ids: std::collections::BTreeSet::new(),
                handler_slots: std::collections::BTreeSet::new(),
                next_event,
            };
            let mut patch_expansion = Some(PatchExpansion {
                retained: accepted_source.clone(),
                dirty,
                rendered: rendered_mounts,
            });
            let mut operations = Vec::with_capacity(boundaries.len());
            let mut patch_nodes_visited = 0;
            let mut local_materializations = 0u64;
            let mut expansion_nodes = 0u64;
            let mut tree_bytes = 0u64;
            let mut generation = 0;
            let mut rebuilt_boundaries = std::collections::BTreeSet::new();
            for (boundary, mount_id, namespace) in &boundaries {
                if let Some(mut local_patch) = patched_mounts.remove(mount_id) {
                    generation = generation.max(self.mounts[mount_id].generation);
                    let structurally_expanded = self.expand_structural_patch_payloads(
                        root.id,
                        *mount_id,
                        boundary,
                        &mut local_patch,
                        &mut expansion,
                    )?;
                    let patch_owner = self.mounts[mount_id].reference.owner.clone();
                    let asset_aliases = &self.packages[&patch_owner].assets;
                    translate_package_patch(
                        local_patch,
                        if *namespace {
                            Some(boundary.as_str())
                        } else {
                            None
                        },
                        *mount_id,
                        &patch_owner,
                        self.id,
                        self.mounts[mount_id].generation,
                        &mut self.patch_authority,
                        &mut expansion,
                        &mut operations,
                        &mut patch_nodes_visited,
                        structurally_expanded,
                        asset_aliases,
                    )?;
                    continue;
                }
                local_materializations = local_materializations.saturating_add(1);
                let rendered = patch_expansion
                    .as_mut()
                    .unwrap()
                    .rendered
                    .remove(mount_id)
                    .ok_or("dirty composition mount omitted its scheduled render")?;
                tree_bytes = tree_bytes.saturating_add(
                    serde_json::to_vec(&rendered.node)
                        .map_err(|error| error.to_string())?
                        .len() as u64,
                );
                generation = generation.max(rendered.generation);
                let mut replacement = rendered.node;
                if *namespace {
                    namespace_native_metadata(&mut replacement, boundary, 0)?;
                }
                // This is a complete owner render, not a local patch. Rebuild
                // its live event set while retaining the slot-to-token index
                // long enough to preserve surviving native action identities.
                expansion
                    .events
                    .retain(|_, handle| handle.mount != *mount_id);
                replacement = self.expand_node(
                    boundary,
                    replacement,
                    &rendered.events,
                    *mount_id,
                    &mut expansion,
                    0,
                    &mut patch_expansion,
                )?;
                if let Some(authority) = self.patch_authority.get_mut(mount_id) {
                    authority.slots.retain(|slot, token| {
                        expansion
                            .events
                            .get(token)
                            .is_some_and(|handle| handle.mount == *mount_id && handle.slot == *slot)
                    });
                }
                rebuilt_boundaries.insert(boundary.clone());
                let target = replacement
                    .get("__nativeId")
                    .and_then(Value::as_str)
                    .ok_or("composition ownership boundary has no native identity")?
                    .to_owned();
                let previous = find_native_id(accepted_source, &target)
                    .ok_or("accepted ownership boundary target disappeared")?;
                diff_native_subtree(
                    previous,
                    &replacement,
                    &mut operations,
                    &mut patch_nodes_visited,
                )?;
                expansion_nodes = patch_nodes_visited;
            }
            // Expanded handler tokens are stable host authority. Both direct
            // patches and complete callback renders update their handle table;
            // the renderer does not need a slot mutation for the same token.
            operations.retain(|operation| {
                !matches!(operation, NativePatchOperation::ReplaceHandlerSlot { .. })
            });
            operations = normalize_atomic_operations(operations);
            let nodes_mutated = operations.len() as u64;
            let patch = NativePatchEnvelope {
                version: 1,
                operations,
                counters: NativePatchCounters {
                    nodes_visited: patch_nodes_visited,
                    nodes_mutated,
                    local_materializations,
                    expansion_nodes,
                    tree_bytes,
                },
            };
            // Expanding a dirty parent may re-render an owned child after the
            // initial dirty-generation scan (for example, transported native
            // children or callback props). Stable handler tokens remain native
            // authority, but their host handles must follow the mount's newly
            // provisional generation before this batch can be admitted.
            for handle in expansion.events.values_mut() {
                if let Some(mount) = self.mounts.get(&handle.mount) {
                    handle.generation = mount.generation;
                }
            }
            let application_started = Instant::now();
            let validation = validate(&patch, &expansion.events, generation);
            let application_micros = application_started
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            self.report_mount_patch_application(root.id, application_micros, validation.is_ok())?;
            let validated = match validation {
                Ok(validated) => validated,
                Err(error) => {
                    rejected_native = Some(NativeRejection {
                        owners: boundaries
                            .iter()
                            .map(|(_, mount, _)| {
                                (*mount, dirty_components.remove(mount).unwrap_or_default())
                            })
                            .collect(),
                        message: error.clone(),
                    });
                    return Err(error);
                }
            };
            let removed = self
                .nested_mounts
                .keys()
                .filter(|(owner, path)| {
                    *owner == root.id
                        && rebuilt_boundaries.iter().any(|boundary| {
                            path != boundary
                                && path
                                    .strip_prefix(boundary)
                                    .is_some_and(|suffix| suffix.starts_with('/'))
                        })
                        && !expansion.visited.contains(path)
                })
                .cloned()
                .collect::<Vec<_>>();
            for key in removed {
                let mount = self.nested_mounts.remove(&key).unwrap();
                self.unmount(&mount)?;
            }
            Ok(ScheduledExpandedBatch::Patched {
                patch,
                events: expansion.events,
                generation,
                validated,
                reconciliation_requested,
            })
        })();
        if result.is_err() {
            self.finish_transaction(false)?;
            if let Some(rejection) = rejected_native {
                let mut captured = false;
                for (mount, component_owners) in rejection.owners {
                    let Some(state) = self.mounts.get(&mount) else {
                        continue;
                    };
                    let package = self
                        .packages
                        .get_mut(&state.reference.owner)
                        .ok_or("retired component owner")?;
                    let mut runtime = package.runtime.borrow_mut();
                    runtime.select_surface(&surface(mount))?;
                    captured |= !runtime
                        .capture_native_failure(&component_owners, &rejection.message)?
                        .is_empty();
                }
                if captured {
                    return self.dispatch_expanded_batch_scheduled_pending_with_validator(
                        root,
                        &[],
                        accepted_events,
                        accepted_source,
                        validate,
                        true,
                    );
                }
            }
        }
        result
    }

    fn report_mount_patch_application(
        &mut self,
        mount: u64,
        application_micros: u64,
        accepted: bool,
    ) -> Result<(), String> {
        let owner = self
            .mounts
            .get(&mount)
            .ok_or("retired component mount")?
            .reference
            .owner
            .clone();
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.select_surface(&surface(mount))?;
        runtime.report_typed_patch_apply(application_micros, accepted)
    }

    /// Reconcile state queued by passive effects after the preceding committed
    /// render. The empty event batch is intentional: it lets the owning mount's
    /// scheduler consume all queued hook updates once, then expands the root
    /// with the same generation and ownership checks as an input dispatch.
    pub fn reconcile_expanded_pending_validated<T>(
        &mut self,
        root: &ComponentMount,
        accepted_events: &BTreeMap<u64, ComponentEventHandle>,
        accepted_source: &Value,
        validate: impl FnMut(
            &NativePatchEnvelope,
            &BTreeMap<u64, ComponentEventHandle>,
            u64,
        ) -> Result<T, String>,
    ) -> Result<ScheduledExpandedBatch<T>, String> {
        let mut requested = false;
        let owned_mounts = std::iter::once(root.id)
            .chain(
                self.nested_mounts
                    .iter()
                    .filter(|((owner, _), _)| *owner == root.id)
                    .map(|(_, mount)| mount.id),
            )
            .collect::<std::collections::BTreeSet<_>>();
        for mount_id in owned_mounts {
            let mount = self
                .mounts
                .get(&mount_id)
                .ok_or("retired component mount")?;
            let package = self
                .packages
                .get(&mount.reference.owner)
                .ok_or("retired component owner")?;
            let mut runtime = package.runtime.borrow_mut();
            runtime.select_surface(&surface(mount_id))?;
            requested |= runtime.reconciliation_requested()?;
        }
        if !requested {
            self.begin_transaction()?;
            return Ok(ScheduledExpandedBatch::Unchanged);
        }
        self.dispatch_expanded_batch_scheduled_pending_validated(
            root,
            &[],
            accepted_events,
            accepted_source,
            validate,
        )
    }

    fn expand_rendered<T>(
        &mut self,
        root: u64,
        rendered: RenderedComponent,
        validate: impl FnOnce(&Value, u64) -> Result<T, String>,
    ) -> Result<(RenderedComponent, T), String> {
        let generation = rendered.generation;
        self.patch_authority.entry(root).or_default().paths.clear();
        let mut expansion = ExpansionState {
            root,
            events: BTreeMap::new(),
            visited: std::collections::BTreeSet::new(),
            native_ids: std::collections::BTreeSet::new(),
            handler_slots: std::collections::BTreeSet::new(),
            next_event: 0,
        };
        let node = self.expand_node(
            "root",
            rendered.node,
            &rendered.events,
            root,
            &mut expansion,
            0,
            &mut None,
        )?;
        bounded_json(&node)?;
        let validated = validate(&node, generation)?;
        let removed = self
            .nested_mounts
            .keys()
            .filter(|(owner, path)| *owner == root && !expansion.visited.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        for key in removed {
            let mount = self.nested_mounts.remove(&key).unwrap();
            self.unmount(&mount)?;
        }
        self.rebuild_patch_authority(root, &node, &expansion.events)?;
        Ok((
            RenderedComponent {
                node,
                events: expansion.events,
                generation,
            },
            validated,
        ))
    }

    fn rebuild_patch_authority(
        &mut self,
        root: u64,
        node: &Value,
        events: &BTreeMap<u64, ComponentEventHandle>,
    ) -> Result<(), String> {
        let prefixes = self
            .nested_mounts
            .iter()
            .filter(|((owner, _), _)| *owner == root)
            .map(|((_, path), mount)| (mount.id, format!("{path}::")))
            .collect::<BTreeMap<_, _>>();
        let mut authority = BTreeMap::<u64, MountPatchAuthority>::new();
        fn visit(
            value: &Value,
            events: &BTreeMap<u64, ComponentEventHandle>,
            prefixes: &BTreeMap<u64, String>,
            authority: &mut BTreeMap<u64, MountPatchAuthority>,
        ) -> Result<(), String> {
            match value {
                Value::Object(object) => {
                    if let Some(slots) = object.get("__handlerSlots").and_then(Value::as_object) {
                        for (property, slot) in slots {
                            let token = object
                                .get(property)
                                .and_then(Value::as_u64)
                                .ok_or("accepted handler slot has no event token")?;
                            let handle = events
                                .get(&token)
                                .ok_or("accepted handler token has no authority")?;
                            let global_slot = slot
                                .as_str()
                                .ok_or("accepted handler slot is not a string")?;
                            let local_slot = prefixes
                                .get(&handle.mount)
                                .and_then(|prefix| global_slot.strip_prefix(prefix))
                                .unwrap_or(global_slot);
                            authority
                                .entry(handle.mount)
                                .or_default()
                                .slots
                                .insert(local_slot.to_owned(), token);
                        }
                    }
                    for value in object.values() {
                        visit(value, events, prefixes, authority)?;
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        visit(value, events, prefixes, authority)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        visit(node, events, &prefixes, &mut authority)?;
        // Reconstruct structural ownership from the accepted native tree.
        // A full render can replace a patch-created splice while retaining the
        // same nested mount. Keeping only the old incremental splice table
        // then leaves a later ReplaceSubtree unable to retire that mount.
        let nested_paths = self
            .nested_mounts
            .iter()
            .filter(|((owner, _), _)| *owner == root)
            .map(|((_, path), mount)| (path.clone(), mount.id))
            .collect::<Vec<_>>();
        fn record_containing_splices(
            value: &Value,
            prefix: &str,
            path: &str,
            splices: &mut BTreeMap<String, std::collections::BTreeSet<String>>,
        ) -> bool {
            let mut contains = value
                .get("__nativeId")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with(prefix));
            match value {
                Value::Object(object) => {
                    for child in object.values() {
                        contains |= record_containing_splices(child, prefix, path, splices);
                    }
                }
                Value::Array(values) => {
                    for child in values {
                        contains |= record_containing_splices(child, prefix, path, splices);
                    }
                }
                _ => {}
            }
            if contains && let Some(id) = value.get("__nativeId").and_then(Value::as_str) {
                splices
                    .entry(id.to_owned())
                    .or_default()
                    .insert(path.to_owned());
            }
            contains
        }
        fn owner_native_id(path: &str) -> Option<String> {
            let boundary = path.rsplit_once('/')?.0;
            let mut native = Vec::new();
            for segment in boundary.split('/') {
                if segment == "children" {
                    continue;
                }
                if let Some(encoded) = segment.strip_prefix('@') {
                    let bytes = encoded
                        .as_bytes()
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| {
                            let text = std::str::from_utf8(pair).ok()?;
                            u8::from_str_radix(text, 16).ok()
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let key = serde_json::from_slice::<Value>(&bytes).ok()?;
                    native.push(format!("@{}", key.as_str()?));
                } else {
                    native.push(segment.to_owned());
                }
            }
            Some(native.join("/"))
        }
        for (path, _) in &nested_paths {
            let prefix = format!("{path}::");
            let parent = nested_paths
                .iter()
                .filter(|(candidate, _)| {
                    path.strip_prefix(candidate)
                        .is_some_and(|suffix| suffix.starts_with('/'))
                })
                .max_by_key(|(candidate, _)| candidate.len())
                .map_or(root, |(_, mount)| *mount);
            record_containing_splices(
                node,
                &prefix,
                path,
                &mut authority.entry(parent).or_default().splices,
            );
            if let Some(identity) = owner_native_id(path) {
                authority
                    .entry(parent)
                    .or_default()
                    .splices
                    .entry(identity)
                    .or_default()
                    .insert(path.clone());
            }
        }
        // Patch identities belong to the unexpanded owner tree, so they need
        // not appear in the accepted expanded tree. Preserve those identities
        // while rebinding their paths to the currently mounted boundary with
        // the same public-component selection.
        if let Some(previous) = self.patch_authority.get(&root) {
            for (identity, old_paths) in &previous.splices {
                for old_path in old_paths {
                    let selection = old_path.rsplit('/').next().unwrap_or(old_path);
                    let mut matches = nested_paths
                        .iter()
                        .filter(|(path, _)| path.rsplit('/').next() == Some(selection));
                    if let Some((path, _)) = matches.next()
                        && matches.next().is_none()
                    {
                        authority
                            .entry(root)
                            .or_default()
                            .splices
                            .entry(identity.clone())
                            .or_default()
                            .insert(path.clone());
                    }
                }
            }
        }
        for mount in std::iter::once(root).chain(prefixes.keys().copied()) {
            let mut next = authority.remove(&mount).unwrap_or_default();
            if let Some(previous) = self.patch_authority.get_mut(&mount) {
                next.paths = std::mem::take(&mut previous.paths);
            }
            self.patch_authority.insert(mount, next);
        }
        Ok(())
    }

    fn expand_structural_patch_payloads(
        &mut self,
        root: u64,
        mount: u64,
        boundary: &str,
        patch: &mut NativePatchEnvelope,
        expansion: &mut ExpansionState,
    ) -> Result<bool, String> {
        let retained_splices = self
            .patch_authority
            .get(&mount)
            .map(|authority| &authority.splices);
        let has_structural = patch.operations.iter().any(|operation| match operation {
            NativePatchOperation::InsertChild { node, .. } => contains_package_transport(node),
            NativePatchOperation::ReplaceSubtree { target, node } => {
                contains_package_transport(node)
                    || retained_splices.is_some_and(|splices| splices.contains_key(target))
            }
            NativePatchOperation::RemoveChild { child_id, .. } => {
                retained_splices.is_some_and(|splices| splices.contains_key(child_id))
            }
            _ => false,
        });
        if !has_structural {
            return Ok(false);
        }
        let mut source_events = expansion
            .events
            .values()
            .filter(|handle| handle.mount == mount)
            .map(|handle| (handle.action, handle.clone()))
            .collect::<BTreeMap<_, _>>();
        // Structural payloads contain package-local actions introduced by this
        // render, not just actions present in the previous expanded frame.
        // Bind those slots to the current native-owned mount before transporting
        // callback props into newly materialized package components.
        fn bind_payload_actions(
            value: &Value,
            template: &ComponentEventHandle,
            events: &mut BTreeMap<u64, ComponentEventHandle>,
            depth: usize,
        ) -> Result<(), String> {
            if depth > MAX_TREE_DEPTH {
                return Err("component tree exceeds depth limit".into());
            }
            match value {
                Value::Array(values) => {
                    for value in values {
                        bind_payload_actions(value, template, events, depth + 1)?;
                    }
                }
                Value::Object(object) => {
                    for (key, value) in object {
                        if key == "__handlerSlots" {
                            continue;
                        }
                        if is_action(key) && !value.is_null() {
                            let action = value
                                .as_u64()
                                .filter(|action| *action < MAX_HANDLERS as u64)
                                .ok_or("invalid component action")?;
                            let slot = object
                                .get("__handlerSlots")
                                .and_then(|slots| slots.get(key))
                                .and_then(Value::as_str)
                                .ok_or("component action has no stable handler slot")?;
                            let mut handle = template.clone();
                            handle.action = action;
                            handle.slot = slot.to_owned();
                            events.insert(action, handle);
                            if events.len() > MAX_HANDLERS {
                                return Err("component tree exceeds handler limit".into());
                            }
                        } else {
                            bind_payload_actions(value, template, events, depth + 1)?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        let state = &self.mounts[&mount];
        let template = ComponentEventHandle {
            runtime: self.id,
            mount,
            generation: state.generation,
            owner: state.reference.owner.clone(),
            action: 0,
            slot: String::new(),
        };
        for operation in &patch.operations {
            if let NativePatchOperation::InsertChild { node, .. }
            | NativePatchOperation::ReplaceSubtree { node, .. } = operation
            {
                bind_payload_actions(node, &template, &mut source_events, 0)?;
            }
        }
        // Retire the old subtree's slots before expansion installs replacement
        // bindings. Retiring them in translation afterwards would delete the
        // new callbacks as well, leaving live native nodes with dead tokens.
        for operation in &patch.operations {
            let target = match operation {
                NativePatchOperation::ReplaceSubtree { target, .. } => target,
                NativePatchOperation::RemoveChild { child_id, .. } => child_id,
                _ => continue,
            };
            if let Some(authority) = self.patch_authority.get_mut(&mount) {
                authority.slots.retain(|slot, token| {
                    let keep = !native_slot_in_subtree(slot, target);
                    if !keep {
                        expansion.events.remove(token);
                    }
                    keep
                });
            }
        }
        let mut expanded = false;
        for operation in &mut patch.operations {
            let (identity, node, is_insert, inserted_path) = match operation {
                NativePatchOperation::InsertChild {
                    parent,
                    child_id,
                    node,
                    index,
                    ..
                } => {
                    let path = self.patch_authority[&mount].paths.get(parent).map(|path| {
                        format!(
                            "{path}/children/{}",
                            composition_child_identity(node, *index)
                        )
                    });
                    (child_id.clone(), Some(node), true, path)
                }
                NativePatchOperation::ReplaceSubtree { target, node } => {
                    (target.clone(), Some(node), false, None)
                }
                NativePatchOperation::RemoveChild { child_id, .. } => {
                    if let Some(authority) = self.patch_authority.get_mut(&mount) {
                        authority.forget_paths(child_id);
                    }
                    if let Some(paths) = self
                        .patch_authority
                        .get_mut(&mount)
                        .and_then(|authority| authority.splices.remove(child_id))
                    {
                        for path in paths {
                            self.retire_composition_splice(root, &path)?;
                        }
                    }
                    continue;
                }
                _ => continue,
            };
            let Some(node) = node else { continue };
            let path = inserted_path
                .or_else(|| self.patch_authority[&mount].paths.get(&identity).cloned())
                .unwrap_or_else(|| format!("{boundary}/splice:{}", identity.replace('/', "%2f")));
            if is_insert && self.patch_authority[&mount].splices.contains_key(&identity) {
                return Err("duplicate retained composition splice".into());
            }
            self.patch_authority
                .get_mut(&mount)
                .unwrap()
                .forget_paths(&identity);
            let previous = if !is_insert {
                self.patch_authority
                    .get_mut(&mount)
                    .and_then(|authority| authority.splices.remove(&identity))
                    .unwrap_or_default()
            } else {
                Default::default()
            };
            self.patch_authority
                .entry(mount)
                .or_default()
                .splices
                .insert(identity, std::collections::BTreeSet::from([path.clone()]));
            if boundary != "root" {
                namespace_native_metadata(node, boundary, 0)?;
            }
            *node = self.expand_node(
                &path,
                std::mem::take(node),
                &source_events,
                mount,
                expansion,
                0,
                &mut None,
            )?;
            for old_path in previous {
                self.retire_unvisited_composition_splice(root, &old_path, &expansion.visited)?;
            }
            expanded = true;
        }
        Ok(expanded)
    }

    fn retire_composition_splice(&mut self, root: u64, path: &str) -> Result<(), String> {
        self.retire_unvisited_composition_splice(root, path, &Default::default())
    }

    fn retire_unvisited_composition_splice(
        &mut self,
        root: u64,
        path: &str,
        visited: &std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        let mut removed = self
            .nested_mounts
            .keys()
            .filter(|(owner, candidate)| {
                *owner == root
                    && !visited.contains(candidate)
                    && (candidate == path
                        || candidate
                            .strip_prefix(path)
                            .is_some_and(|suffix| suffix.starts_with('/')))
            })
            .cloned()
            .collect::<Vec<_>>();
        removed.sort_by_key(|entry| std::cmp::Reverse(entry.1.len()));
        for key in removed {
            if let Some(child) = self.nested_mounts.remove(&key) {
                self.unmount(&child)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_node(
        &mut self,
        path: &str,
        mut node: Value,
        source_events: &BTreeMap<u64, ComponentEventHandle>,
        source_mount: u64,
        expansion: &mut ExpansionState,
        depth: usize,
        patch: &mut Option<PatchExpansion>,
    ) -> Result<Value, String> {
        if depth > 64 || expansion.events.len() > MAX_HANDLERS {
            return Err("composition expansion exceeds limits".into());
        }
        if let Some(id) = node.get("__nativeId").and_then(Value::as_str) {
            let local = id.rsplit_once("::").map_or(id, |(_, local)| local);
            let authority = self.patch_authority.entry(source_mount).or_default();
            authority.paths.insert(local.to_owned(), path.to_owned());
            if authority.paths.len() > MAX_COMPONENT_NODES {
                return Err("component path index exceeds node limit".into());
            }
        }
        if node.get("kind").and_then(Value::as_str) == Some("__packageChild") {
            let token = node
                .get("child")
                .and_then(Value::as_u64)
                .ok_or("invalid owned child token")?;
            let grant = self
                .children
                .get(&token)
                .ok_or("stale owned child")?
                .clone();
            if grant.receiver != source_mount
                || self.mounts[&source_mount].generation != grant.generation
                || self
                    .mounts
                    .get(&grant.source)
                    .is_none_or(|mount| mount.generation != grant.source_generation)
            {
                return Err("foreign or stale owned child".into());
            }
            let mut owned = grant.node;
            namespace_native_metadata(&mut owned, path, 0)?;
            return self.expand_node(
                path,
                owned,
                &grant.events,
                grant.source,
                expansion,
                depth + 1,
                patch,
            );
        }
        if node.get("kind").and_then(Value::as_str) == Some("__packageComponent") {
            let (selection, reference) =
                if let Some(contract) = node.get("contract").and_then(Value::as_str) {
                    (
                        format!("export:{contract}"),
                        self.component(contract)
                            .ok_or("unknown public component contract")?,
                    )
                } else if let Some(page) = node.get("settingPage") {
                    let provider = page
                        .get("provider")
                        .and_then(Value::as_str)
                        .ok_or("missing page provider")?;
                    let id = page
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("missing page identity")?;
                    (
                        format!("setting-page:{provider}/{id}"),
                        self.registered_page(provider, id)
                            .ok_or("unknown registered Settings page")?,
                    )
                } else {
                    let key = node
                        .get("contribution")
                        .and_then(Value::as_str)
                        .ok_or("missing public component selection")?;
                    (
                        format!("contribution:{key}"),
                        self.contribution(key)
                            .ok_or("unknown public contribution")?,
                    )
                };
            let key = format!("{path}/{selection}");
            expansion.visited.insert(key.clone());
            let identity = (expansion.root, key.clone());
            let mount = if let Some(mount) = self.nested_mounts.get(&identity) {
                mount.clone()
            } else {
                let mount = self.mount(&reference)?;
                // Even a callback-free grouping component owns an admitted
                // native boundary. Structural insertion cannot rely on visiting
                // an action node to create its (possibly empty) patch index.
                self.patch_authority.entry(mount.id).or_default();
                self.nested_mounts.insert(identity, mount.clone());
                mount
            };
            // Public package components are ownership boundaries within the
            // same native surface. Give each nested mount the root mount's
            // current compositor authority so surface hooks never retain the
            // package snapshot that happened to exist when it was created.
            let root_surface = self.mounts[&expansion.root].surface.clone();
            let surface_changed = self.mounts[&mount.id].surface != root_surface;
            if surface_changed {
                self.mounts.get_mut(&mount.id).unwrap().surface = root_surface;
            }
            let props = node.get("props").ok_or("missing public component props")?;
            let can_reuse = patch.as_ref().is_some_and(|patch| {
                !surface_changed
                    && !patch.dirty.contains(&mount.id)
                    && !contains_owned_transport(props)
                    && self.mounts[&mount.id].props == *props
            });
            if can_reuse {
                let prefix = format!("{key}::");
                let retained =
                    find_unique_native_prefix(&patch.as_ref().unwrap().retained, &prefix)?
                        .ok_or("retained ownership boundary subtree disappeared")?
                        .clone();
                for (owner, retained_path) in self.nested_mounts.keys() {
                    if *owner == expansion.root
                        && (retained_path == &key
                            || retained_path
                                .strip_prefix(&key)
                                .is_some_and(|suffix| suffix.starts_with('/')))
                    {
                        expansion.visited.insert(retained_path.clone());
                    }
                }
                return Ok(retained);
            }
            self.callbacks.retain(|_, grant| grant.receiver != mount.id);
            self.children.retain(|_, grant| grant.receiver != mount.id);
            let props =
                self.transport_callback_props(props, source_events, &mount, source_mount, depth)?;
            let rendered = self.render(&mount, &props)?;
            self.patch_authority
                .entry(mount.id)
                .or_default()
                .paths
                .clear();
            // Replace this mount's event bindings, but preserve its hook state
            // and stable native slot tokens while expanding its new render.
            expansion
                .events
                .retain(|_, handle| handle.mount != mount.id);
            if let Some(patch) = patch {
                patch.rendered.remove(&mount.id);
            }
            let mut embedded = rendered.node;
            namespace_native_metadata(&mut embedded, &key, 0)?;
            return self.expand_node(
                &key,
                embedded,
                &rendered.events,
                mount.id,
                expansion,
                depth + 1,
                patch,
            );
        }
        if let Value::Object(object) = &mut node {
            validate_native_metadata(object, expansion)?;
            let owner = &self.mounts[&source_mount].reference.owner;
            for field in ["asset", "icon"] {
                if let Some(Value::String(name)) = object.get_mut(field) {
                    if name.starts_with("composition.asset.") {
                        return Err("reserved native asset reference".into());
                    }
                    if let Some(alias) = self.asset_key(owner, name) {
                        *name = alias.into();
                    }
                }
            }
        }
        match &mut node {
            Value::Array(values) => {
                for (index, value) in values.iter_mut().enumerate() {
                    let identity = composition_child_identity(value, index);
                    *value = self.expand_node(
                        &format!("{path}/{identity}"),
                        std::mem::take(value),
                        source_events,
                        source_mount,
                        expansion,
                        depth + 1,
                        patch,
                    )?;
                }
            }
            Value::Object(object) => {
                let retained_slots = object
                    .get("__handlerSlots")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (key, value) in object {
                    if key == "__handlerSlots" {
                        continue;
                    } else if is_action(key) && !value.is_null() {
                        let handle = source_events
                            .get(&value.as_u64().ok_or("invalid host event token")?)
                            .ok_or("unknown host event token")?
                            .clone();
                        let global_slot = retained_slots
                            .get(key)
                            .and_then(Value::as_str)
                            .ok_or("expanded action has no stable handler slot")?;
                        let prefix = self
                            .nested_mounts
                            .iter()
                            .find(|((owner, _), mount)| {
                                *owner == expansion.root && mount.id == source_mount
                            })
                            .map(|((_, path), _)| format!("{path}::"));
                        let local_slot = prefix
                            .as_deref()
                            .and_then(|prefix| global_slot.strip_prefix(prefix))
                            .unwrap_or(global_slot);
                        let retained_token = self
                            .patch_authority
                            .get(&source_mount)
                            .and_then(|authority| authority.slots.get(local_slot))
                            .copied()
                            .filter(|token| {
                                expansion.events.get(token).is_none_or(|previous| {
                                    previous.mount == source_mount && previous.slot == local_slot
                                })
                            });
                        let token = retained_token.unwrap_or(expansion.next_event);
                        expansion.next_event = expansion.next_event.max(
                            token
                                .checked_add(1)
                                .ok_or("expanded event identity exhausted")?,
                        );
                        expansion.events.insert(token, handle);
                        self.patch_authority
                            .entry(source_mount)
                            .or_default()
                            .slots
                            .insert(local_slot.to_owned(), token);
                        *value = Value::from(token);
                    } else {
                        *value = self.expand_node(
                            &format!("{path}/{key}"),
                            std::mem::take(value),
                            source_events,
                            source_mount,
                            expansion,
                            depth + 1,
                            patch,
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(node)
    }

    fn transport_callback_props(
        &mut self,
        value: &Value,
        source_events: &BTreeMap<u64, ComponentEventHandle>,
        receiver: &ComponentMount,
        source_mount: u64,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 64 {
            return Err("component prop depth exceeds limit".into());
        }
        match value {
            Value::Object(object) if object.contains_key("__ownedChild") => {
                if object.len() != 1 {
                    return Err("invalid owned child prop".into());
                }
                let node = object["__ownedChild"].clone();
                bounded_json(&node)?;
                if self.children.len() + self.callbacks.len() + self.children.len() >= MAX_HANDLERS
                    || self
                        .children
                        .values()
                        .map(|child| child.node.to_string().len())
                        .sum::<usize>()
                        + node.to_string().len()
                        > 4 * 1024 * 1024
                {
                    return Err("owned child props exceed limit".into());
                }
                self.next_callback = self
                    .next_callback
                    .checked_add(1)
                    .ok_or("child identity exhausted")?;
                let token = self.next_callback;
                self.children.insert(
                    token,
                    OwnedChildGrant {
                        receiver: receiver.id,
                        generation: self
                            .next_generation
                            .checked_add(1)
                            .ok_or("component generation exhausted")?,
                        source: source_mount,
                        source_generation: self.mounts[&source_mount].generation,
                        events: owned_child_events(&node, source_events)?,
                        node,
                    },
                );
                Ok(serde_json::json!({"__hostChild":token}))
            }
            Value::Object(object) if object.contains_key("__callbackAction") => {
                if object
                    .keys()
                    .any(|key| key != "__callbackAction" && key != "__handlerSlots")
                {
                    return Err("invalid callback prop".into());
                }
                let source = source_events
                    .get(
                        &object["__callbackAction"]
                            .as_u64()
                            .ok_or("invalid callback action")?,
                    )
                    .ok_or("unknown callback action")?
                    .clone();
                if self.callbacks.len() >= MAX_HANDLERS {
                    return Err("too many component callback props".into());
                }
                self.next_callback = self
                    .next_callback
                    .checked_add(1)
                    .ok_or("callback identity exhausted")?;
                let token = self.next_callback;
                self.callbacks.insert(
                    token,
                    CallbackGrant {
                        receiver: receiver.id,
                        generation: self
                            .next_generation
                            .checked_add(1)
                            .ok_or("component generation exhausted")?,
                        source,
                    },
                );
                Ok(serde_json::json!({"__hostCallback":token}))
            }
            Value::Object(object) => {
                if object.keys().any(|key| key.starts_with("__host")) {
                    return Err("forged callback prop".into());
                }
                object
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            key.clone(),
                            self.transport_callback_props(
                                value,
                                source_events,
                                receiver,
                                source_mount,
                                depth + 1,
                            )?,
                        ))
                    })
                    .collect::<Result<serde_json::Map<_, _>, String>>()
                    .map(Value::Object)
            }
            Value::Array(values) => values
                .iter()
                .map(|value| {
                    self.transport_callback_props(
                        value,
                        source_events,
                        receiver,
                        source_mount,
                        depth + 1,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            _ => Ok(value.clone()),
        }
    }

    /// Native resource alias for an asset declared by this exact live owner.
    pub fn asset_key(&self, owner: &PackageIdentity, name: &str) -> Option<&str> {
        self.packages
            .get(owner)?
            .assets
            .get(name)
            .map(String::as_str)
    }

    /// Return the host-owned data projection for an exact, live package owner.
    pub fn snapshot(&self, owner: &PackageIdentity) -> Result<&Value, String> {
        self.packages
            .get(owner)
            .map(|package| package.data.as_ref())
            .ok_or("retired snapshot owner".into())
    }

    pub fn update_snapshot(&mut self, owner: &PackageIdentity, data: &Value) -> Result<(), String> {
        bounded_json(data)?;
        if !data.is_object() {
            return Err("package snapshot must be an object".into());
        }
        let package = self
            .packages
            .get_mut(owner)
            .ok_or("retired snapshot owner")?;
        package
            .runtime
            .borrow_mut()
            .set_capability_store(&package.manifest.capabilities, data)?;
        package.data = std::rc::Rc::new(data.clone());
        Ok(())
    }

    /// Check this immediately before the host validates and executes the effect.
    /// Outstanding envelopes from a retired or different host are stale.
    pub fn validate_effect(&self, effect: &OwnedComponentEffect) -> Result<(), String> {
        if effect.runtime != self.id
            || self
                .packages
                .get(&effect.owner)
                .is_none_or(|p| p.incarnation != effect.incarnation)
        {
            return Err("foreign or retired component effect".into());
        }
        Ok(())
    }

    pub fn unmount(&mut self, mount: &ComponentMount) -> Result<(), String> {
        self.validate_mount(mount)?;
        let children = self
            .nested_mounts
            .keys()
            .filter(|(root, _)| *root == mount.id)
            .cloned()
            .collect::<Vec<_>>();
        for key in children {
            let child = self.nested_mounts.remove(&key).unwrap();
            self.unmount(&child)?;
        }
        let state = self.mounts.remove(&mount.id).unwrap();
        self.patch_authority.remove(&mount.id);
        self.callbacks
            .retain(|_, grant| grant.receiver != mount.id && grant.source.mount != mount.id);
        self.children
            .retain(|_, grant| grant.receiver != mount.id && grant.source != mount.id);
        self.packages
            .get_mut(&state.reference.owner)
            .unwrap()
            .runtime
            .borrow_mut()
            .drop_surface(&surface(mount.id))
    }

    /// Retirement invalidates all references/events and drops the entire owner
    /// context (hooks, handlers, subscriptions and executable registrations).
    pub fn retire(&mut self, owner: &PackageIdentity) {
        let mounts = self
            .mounts
            .iter()
            .filter(|(_, mount)| &mount.reference.owner == owner)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in mounts {
            if self.mounts.contains_key(&id) {
                let _ = self.unmount(&ComponentMount {
                    runtime: self.id,
                    id,
                });
            }
        }
        self.packages.remove(owner);
        self.callbacks.retain(|_, grant| {
            &grant.source.owner != owner && self.mounts.contains_key(&grant.receiver)
        });
        self.children.retain(|_, grant| {
            self.mounts.contains_key(&grant.receiver) && self.mounts.contains_key(&grant.source)
        });
        self.nested_mounts
            .retain(|_, mount| self.mounts.contains_key(&mount.id));
        self.effects.retain(|effect| &effect.owner != owner);
        for entries in self.resolution.contributions.values_mut() {
            entries.retain(|entry| &entry.contributed_by != owner);
        }
        let _ = self.publish_contribution_catalog();
    }

    pub fn take_effects(&mut self) -> Vec<OwnedComponentEffect> {
        // Once handed to native approval, a rejected batch is discarded rather
        // than resurrected from the checkpoint as a queued request.
        if let Some(checkpoint) = &mut self.checkpoint {
            checkpoint.effects.clear();
        }
        std::mem::take(&mut self.effects)
    }

    fn validate_mount(&self, mount: &ComponentMount) -> Result<(), String> {
        if mount.runtime != self.id || !self.mounts.contains_key(&mount.id) {
            return Err("foreign or retired component mount".into());
        }
        let reference = &self.mounts[&mount.id].reference;
        if let Some(id) = reference.implementation.strip_prefix("@settings-page/")
            && self
                .packages
                .get(&reference.owner)
                .is_none_or(|package| !package.runtime.borrow().has_registered_page(id))
        {
            return Err("Settings page is no longer published by its owner".into());
        }
        Ok(())
    }

    fn render_mount(
        &mut self,
        id: u64,
        event: Option<Value>,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        let catalog = self.contribution_catalog();
        let state = &self.mounts[&id];
        let owner = state.reference.owner.clone();
        let surface_snapshot = state
            .surface
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let previous_generation = state.generation;
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or("component generation exhausted")?;
        self.next_generation = generation;
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.set_contribution_catalog(&catalog)?;
        runtime.select_surface(&surface(id))?;
        runtime.set_surface_store(&surface(id), &surface_snapshot)?;
        runtime.set_mount_data(&package.data, state.surface.as_ref(), &state.props)?;
        let result = runtime.render_native(event.as_ref(), |value| {
            bounded_json(value)?;
            let mut node = value.clone();
            let mut events = BTreeMap::new();
            rewrite_events(
                &mut node,
                self.id,
                id,
                generation,
                &owner,
                &mut events,
                &mut TreeBudget::default(),
                0,
            )?;
            validate(&node)?;
            Ok(RenderedComponent {
                node,
                events,
                generation,
            })
        });
        if event.is_some() {
            runtime.finish_event(result.is_ok())?;
        }
        let rendered = result?;
        drop(runtime);
        self.mounts.get_mut(&id).unwrap().generation = generation;
        self.drain_effects(&owner, id, previous_generation, event.is_some())?;
        Ok(rendered)
    }

    fn render_mount_scheduled(
        &mut self,
        id: u64,
        events: Value,
    ) -> Result<ScheduledComponentDispatch, String> {
        let catalog = self.contribution_catalog();
        let state = &self.mounts[&id];
        let owner = state.reference.owner.clone();
        let surface_snapshot = state
            .surface
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let previous_generation = state.generation;
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or("component generation exhausted")?;
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.set_contribution_catalog(&catalog)?;
        runtime.select_surface(&surface(id))?;
        runtime.set_surface_store(&surface(id), &surface_snapshot)?;
        runtime.set_mount_data(&package.data, state.surface.as_ref(), &state.props)?;
        let expression = format!("__nickelDispatchBatchScheduled({events})");
        let outcome = runtime.dispatch_scheduled(&expression, |value| {
            bounded_json(value)?;
            let mut node = value.clone();
            let mut owned_events = BTreeMap::new();
            rewrite_events(
                &mut node,
                self.id,
                id,
                generation,
                &owner,
                &mut owned_events,
                &mut TreeBudget::default(),
                0,
            )?;
            Ok(RenderedComponent {
                node,
                events: owned_events,
                generation,
            })
        });
        runtime.finish_event(outcome.is_ok())?;
        let outcome = outcome?;
        drop(runtime);
        let (rendered, reconciliation_requested) = match outcome {
            ScheduledRender::Unchanged => (None, false),
            ScheduledRender::Rendered {
                value,
                reconciliation_requested,
                ..
            } => {
                self.next_generation = generation;
                self.mounts.get_mut(&id).unwrap().generation = generation;
                (Some(value), reconciliation_requested)
            }
        };
        let generation_before_effects = self.next_generation;
        self.drain_effects(&owner, id, previous_generation, true)?;
        if rendered.is_some() {
            self.advance_mount_generation_grants(id, previous_generation, generation);
        }
        Ok(ScheduledComponentDispatch {
            rendered,
            reconciliation_requested,
            requires_expansion: self.next_generation != generation_before_effects,
        })
    }

    fn render_mount_patched(
        &mut self,
        id: u64,
        events: Vec<Value>,
    ) -> Result<PatchedMountDispatch, String> {
        self.render_mount_patched_with(id, events, false)
    }

    fn render_mount_slots_patched(
        &mut self,
        id: u64,
        events: Vec<Value>,
    ) -> Result<PatchedMountDispatch, String> {
        self.render_mount_patched_with(id, events, true)
    }

    fn render_mount_patched_with(
        &mut self,
        id: u64,
        events: Vec<Value>,
        slots: bool,
    ) -> Result<PatchedMountDispatch, String> {
        let catalog = self.contribution_catalog();
        let state = &self.mounts[&id];
        let owner = state.reference.owner.clone();
        let surface_snapshot = state
            .surface
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        let previous_generation = state.generation;
        let generation = self
            .next_generation
            .checked_add(1)
            .ok_or("component generation exhausted")?;
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.set_contribution_catalog(&catalog)?;
        runtime.select_surface(&surface(id))?;
        runtime.set_surface_store(&surface(id), &surface_snapshot)?;
        runtime.set_mount_data(&package.data, state.surface.as_ref(), &state.props)?;
        let outcome = if slots {
            runtime.dispatch_slots_patched(events)
        } else {
            runtime.dispatch_batch_patched(events, false)
        };
        // This accepts the package-local retained tree provisionally. The outer
        // composition checkpoint still restores it if native validation or any
        // sibling/callback render fails.
        runtime.finish_patch_render(outcome.is_ok())?;
        runtime.finish_event(outcome.is_ok())?;
        let outcome = outcome?;
        drop(runtime);
        let (patch, dirty_components, reconciliation_requested) = match outcome {
            ScheduledPatch::Unchanged => (None, Vec::new(), false),
            ScheduledPatch::Patched {
                patch,
                dirty_components,
                reconciliation_requested,
                ..
            } => {
                self.next_generation = generation;
                self.mounts.get_mut(&id).unwrap().generation = generation;
                (Some(patch), dirty_components, reconciliation_requested)
            }
        };
        self.drain_effects(&owner, id, previous_generation, true)?;
        if patch.is_some() {
            self.advance_mount_generation_grants(id, previous_generation, generation);
        }
        Ok(PatchedMountDispatch {
            patch,
            dirty_components,
            reconciliation_requested,
        })
    }

    fn drain_effects(
        &mut self,
        owner: &PackageIdentity,
        receiver: u64,
        generation: u64,
        from_event: bool,
    ) -> Result<(), String> {
        let effects = self
            .packages
            .get_mut(owner)
            .unwrap()
            .runtime
            .borrow_mut()
            .take_effects()?;
        if self.effects.len() + effects.len() > MAX_EFFECTS {
            return Err("too many composition effects".into());
        }
        let mut bytes = self
            .effects
            .iter()
            .map(|effect| effect.value.to_string().len())
            .sum::<usize>();
        for value in &effects {
            let size = value.to_string().len();
            if size > 64 * 1024 {
                return Err("composition effect exceeds size limit".into());
            }
            bytes = bytes
                .checked_add(size)
                .ok_or("composition effect size overflow")?;
            if bytes > 4 * 1024 * 1024 {
                return Err("composition effect queue exceeds size limit".into());
            }
        }
        let mut invocations: BTreeMap<u64, (ComponentEventHandle, Vec<Value>)> = BTreeMap::new();
        for value in &effects {
            if value.get("type").and_then(Value::as_str) == Some("__compositionCallback") {
                if !from_event {
                    return Err("component callback invoked outside an event".into());
                }
                let token = value
                    .get("callback")
                    .and_then(Value::as_u64)
                    .ok_or("invalid callback token")?;
                let grant = self
                    .callbacks
                    .get(&token)
                    .ok_or("stale component callback")?;
                if grant.receiver != receiver || grant.generation != generation {
                    return Err("foreign component callback".into());
                }
                let args = value
                    .get("args")
                    .filter(|args| args.is_array())
                    .ok_or("invalid callback arguments")?;
                let source = &grant.source;
                let mount = self
                    .mounts
                    .get(&source.mount)
                    .ok_or("retired callback owner")?;
                if mount.generation != source.generation || mount.reference.owner != source.owner {
                    return Err("stale callback owner".into());
                }
                let group = invocations
                    .entry(source.mount)
                    .or_insert_with(|| (source.clone(), Vec::new()));
                group.1.push(serde_json::json!([source.action, args]));
            }
        }
        if effects.iter().any(|value| {
            value.get("type").and_then(Value::as_str) == Some("__compositionCallbackBoundary")
        }) && self.callback_depth == 0
        {
            return Err("unexpected component callback boundary".into());
        }
        if self.callback_depth >= 32 {
            return Err("component callback cycle exceeds limit".into());
        }
        self.callback_depth += 1;
        let result = (|| {
            let mut produced = BTreeMap::new();
            for (mount, (handle, events)) in invocations {
                if self
                    .mounts
                    .get(&mount)
                    .is_none_or(|state| state.generation != handle.generation)
                {
                    return Err("callback owner changed during invocation".into());
                }
                let count = events.len();
                let start = self.effects.len();
                let rendered = self.render_mount(mount, Some(Value::Array(events)), |_| Ok(()))?;
                self.scheduled_renders.insert(mount, rendered);
                let generated = self.effects.split_off(start);
                let mut groups = Vec::new();
                let mut current = Vec::new();
                for effect in generated {
                    if effect.value.get("type").and_then(Value::as_str)
                        == Some("__compositionCallbackBoundary")
                    {
                        groups.push(std::mem::take(&mut current));
                    } else {
                        current.push(effect);
                    }
                }
                if groups.len() != count {
                    return Err("invalid component callback boundaries".into());
                }
                if let Some(last) = groups.last_mut() {
                    last.extend(current);
                }
                produced.insert(mount, std::collections::VecDeque::from(groups));
            }
            for value in effects {
                if value.get("type").and_then(Value::as_str) == Some("__compositionCallback") {
                    let token = value["callback"].as_u64().unwrap();
                    let source_mount = self.callbacks[&token].source.mount;
                    self.effects.extend(
                        produced
                            .get_mut(&source_mount)
                            .and_then(|groups| groups.pop_front())
                            .ok_or("missing component callback effects")?,
                    );
                } else {
                    self.effects.push(OwnedComponentEffect {
                        runtime: self.id,
                        owner: owner.clone(),
                        incarnation: self.packages[owner].incarnation,
                        value,
                    });
                }
            }
            if self.effects.len() > MAX_EFFECTS
                || self
                    .effects
                    .iter()
                    .map(|effect| effect.value.to_string().len())
                    .sum::<usize>()
                    > 4 * 1024 * 1024
            {
                return Err("composition callback effects exceed queue limit".into());
            }
            Ok(())
        })();
        self.callback_depth -= 1;
        result
    }
}

fn owned_child_events(
    node: &Value,
    source: &BTreeMap<u64, ComponentEventHandle>,
) -> Result<BTreeMap<u64, ComponentEventHandle>, String> {
    fn collect(node: &Value, tokens: &mut std::collections::BTreeSet<u64>) {
        match node {
            Value::Object(object) => {
                for (key, value) in object {
                    if is_action(key) {
                        if let Some(token) = value.as_u64() {
                            tokens.insert(token);
                        }
                    } else {
                        collect(value, tokens);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, tokens);
                }
            }
            _ => {}
        }
    }
    let mut tokens = std::collections::BTreeSet::new();
    collect(node, &mut tokens);
    tokens
        .into_iter()
        .map(|token| {
            Ok((
                token,
                source
                    .get(&token)
                    .ok_or("unknown owned child action")?
                    .clone(),
            ))
        })
        .collect()
}

fn contains_owned_transport(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("__callbackAction")
                || object.contains_key("__ownedChild")
                || object.values().any(contains_owned_transport)
        }
        Value::Array(values) => values.iter().any(contains_owned_transport),
        _ => false,
    }
}

fn contains_package_transport(value: &Value) -> bool {
    if matches!(
        value.get("kind").and_then(Value::as_str),
        Some("__packageComponent" | "__packageChild")
    ) {
        return true;
    }
    match value {
        Value::Object(object) => object.values().any(contains_package_transport),
        Value::Array(values) => values.iter().any(contains_package_transport),
        _ => false,
    }
}

fn find_unique_native_prefix<'a>(
    value: &'a Value,
    prefix: &str,
) -> Result<Option<&'a Value>, String> {
    fn visit<'a>(
        value: &'a Value,
        prefix: &str,
        found: &mut Option<&'a Value>,
    ) -> Result<(), String> {
        if value
            .get("__nativeId")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with(prefix))
        {
            if found.replace(value).is_some() {
                return Err("ambiguous duplicate retained ownership path".into());
            }
            return Ok(());
        }
        match value {
            Value::Object(object) => {
                for value in object.values() {
                    visit(value, prefix, found)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, prefix, found)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut found = None;
    visit(value, prefix, &mut found)?;
    Ok(found)
}

fn find_native_id<'a>(value: &'a Value, target: &str) -> Option<&'a Value> {
    if value.get("__nativeId").and_then(Value::as_str) == Some(target) {
        return Some(value);
    }
    match value {
        Value::Object(object) => object
            .values()
            .find_map(|value| find_native_id(value, target)),
        Value::Array(values) => values
            .iter()
            .find_map(|value| find_native_id(value, target)),
        _ => None,
    }
}

fn diff_native_subtree(
    before: &Value,
    after: &Value,
    operations: &mut Vec<NativePatchOperation>,
    visited: &mut u64,
) -> Result<(), String> {
    *visited = visited.saturating_add(1);
    if before == after {
        return Ok(());
    }
    let Some(target) = after.get("__nativeId").and_then(Value::as_str) else {
        return Err("incremental native candidate has no identity".into());
    };
    if before.get("__nativeId").and_then(Value::as_str) != Some(target)
        || before.get("kind") != after.get("kind")
    {
        operations.push(NativePatchOperation::ReplaceSubtree {
            target: before
                .get("__nativeId")
                .and_then(Value::as_str)
                .ok_or("replaced native subtree has no identity")?
                .to_owned(),
            node: after.clone(),
        });
        return Ok(());
    }
    let before_object = before.as_object().ok_or("native node is not an object")?;
    let after_object = after.as_object().ok_or("native node is not an object")?;
    for (property, value) in after_object {
        if matches!(
            property.as_str(),
            "children" | "__nativeId" | "__handlerSlots"
        ) || is_action(property)
        {
            continue;
        }
        if before_object.get(property) != Some(value) {
            if value.is_object() || value.is_array() {
                operations.push(NativePatchOperation::ReplaceSubtree {
                    target: target.to_owned(),
                    node: after.clone(),
                });
                return Ok(());
            }
            operations.push(NativePatchOperation::SetPrimitive {
                target: target.to_owned(),
                property: property.clone(),
                value: value.clone(),
            });
        }
    }
    let before_slots = before_object
        .get("__handlerSlots")
        .and_then(Value::as_object);
    let after_slots = after_object
        .get("__handlerSlots")
        .and_then(Value::as_object);
    if before_slots.map(|slots| slots.keys().collect::<Vec<_>>())
        != after_slots.map(|slots| slots.keys().collect::<Vec<_>>())
    {
        operations.push(NativePatchOperation::ReplaceSubtree {
            target: target.to_owned(),
            node: after.clone(),
        });
        return Ok(());
    }
    if let Some(slots) = after_slots {
        if slots.keys().any(|event| {
            before_object
                .get(event)
                .is_some_and(|value| !value.is_null())
                != after_object
                    .get(event)
                    .is_some_and(|value| !value.is_null())
        }) {
            operations.push(NativePatchOperation::ReplaceSubtree {
                target: target.to_owned(),
                node: after.clone(),
            });
            return Ok(());
        }
        for (event, slot) in slots {
            let action = after_object.get(event).and_then(Value::as_u64);
            if action != before_object.get(event).and_then(Value::as_u64) {
                operations.push(NativePatchOperation::ReplaceHandlerSlot {
                    slot: slot
                        .as_str()
                        .ok_or("native handler slot must be a string")?
                        .to_owned(),
                    action: usize::try_from(action.ok_or("native handler action disappeared")?)
                        .map_err(|_| "native handler action exceeds host range")?,
                });
            }
        }
    }
    let before_children = before_object.get("children").and_then(Value::as_array);
    let after_children = after_object.get("children").and_then(Value::as_array);
    match (before_children, after_children) {
        (Some(before_children), Some(after_children))
            if before_children.iter().all(|child| !child.is_object())
                && after_children.iter().all(|child| !child.is_object()) =>
        {
            if before_children != after_children {
                operations.push(NativePatchOperation::SetPrimitive {
                    target: target.to_owned(),
                    property: "children".into(),
                    value: Value::Array(after_children.clone()),
                });
            }
        }
        (Some(before_children), Some(after_children))
            if before_children.iter().chain(after_children).all(|child| {
                child.get("key").and_then(Value::as_str).is_some()
                    && child.get("__nativeId").and_then(Value::as_str).is_some()
            }) =>
        {
            let mut current = before_children.clone();
            for index in (0..current.len()).rev() {
                let id = current[index]["__nativeId"].as_str().unwrap();
                if !after_children.iter().any(|child| child["__nativeId"] == id) {
                    operations.push(NativePatchOperation::RemoveChild {
                        parent: target.to_owned(),
                        key: current[index]["key"].as_str().unwrap().to_owned(),
                        child_id: id.to_owned(),
                        index,
                    });
                    current.remove(index);
                }
            }
            for (index, desired) in after_children.iter().enumerate() {
                let desired_id = desired["__nativeId"].as_str().unwrap();
                if let Some(from) = current
                    .iter()
                    .position(|child| child["__nativeId"] == desired_id)
                {
                    if from != index {
                        operations.push(NativePatchOperation::MoveChild {
                            parent: target.to_owned(),
                            key: desired["key"].as_str().unwrap().to_owned(),
                            child_id: desired_id.to_owned(),
                            from,
                            to: index,
                        });
                        let child = current.remove(from);
                        current.insert(index, child);
                    }
                    diff_native_subtree(&current[index], desired, operations, visited)?;
                } else {
                    operations.push(NativePatchOperation::InsertChild {
                        parent: target.to_owned(),
                        key: desired["key"].as_str().unwrap().to_owned(),
                        child_id: desired_id.to_owned(),
                        index,
                        node: desired.clone(),
                    });
                    current.insert(index, desired.clone());
                }
            }
        }
        (Some(before_children), Some(after_children))
            if before_children.len() == after_children.len() =>
        {
            // Positional children have no structural operation that can change
            // their native identity in place. Replace the nearest stable
            // parent boundary instead of emitting an invalid child replacement
            // whose target names the old node while its payload names the new
            // one.
            if before_children
                .iter()
                .zip(after_children)
                .any(|(before, after)| {
                    before.get("__nativeId") != after.get("__nativeId")
                        || before.get("kind") != after.get("kind")
                })
            {
                operations.push(NativePatchOperation::ReplaceSubtree {
                    target: target.to_owned(),
                    node: after.clone(),
                });
                return Ok(());
            }
            for (before, after) in before_children.iter().zip(after_children) {
                diff_native_subtree(before, after, operations, visited)?;
            }
        }
        _ => operations.push(NativePatchOperation::ReplaceSubtree {
            target: target.to_owned(),
            node: after.clone(),
        }),
    }
    Ok(())
}

fn normalize_atomic_operations(operations: Vec<NativePatchOperation>) -> Vec<NativePatchOperation> {
    fn target(operation: &NativePatchOperation) -> &str {
        match operation {
            NativePatchOperation::SetPrimitive { target, .. }
            | NativePatchOperation::ReplaceSubtree { target, .. } => target,
            NativePatchOperation::ReplaceHandlerSlot { slot, .. } => slot,
            NativePatchOperation::InsertChild { parent, .. }
            | NativePatchOperation::RemoveChild { parent, .. }
            | NativePatchOperation::MoveChild { parent, .. } => parent,
        }
    }
    fn below(candidate: &str, ancestor: &str) -> bool {
        candidate
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with("::"))
    }
    fn at_or_below(candidate: &str, ancestor: &str) -> bool {
        candidate == ancestor || below(candidate, ancestor)
    }
    let mut normalized = Vec::with_capacity(operations.len());
    let mut covered = Vec::<String>::new();
    for operation in operations {
        let operation_target = target(&operation);
        let replaces_subtree = matches!(operation, NativePatchOperation::ReplaceSubtree { .. });
        if covered.iter().any(|ancestor| {
            below(operation_target, ancestor) || (!replaces_subtree && operation_target == ancestor)
        }) {
            continue;
        }
        if let NativePatchOperation::ReplaceSubtree {
            target: replaced_target,
            ..
        } = &operation
        {
            normalized.retain(|prior| !at_or_below(target(prior), replaced_target));
            covered.retain(|prior| !at_or_below(prior, replaced_target));
            covered.push(replaced_target.clone());
        }
        if let NativePatchOperation::RemoveChild { child_id, .. } = &operation {
            normalized.retain(|prior| !at_or_below(target(prior), child_id));
            covered.retain(|prior| !at_or_below(prior, child_id));
            covered.push(child_id.clone());
        }
        normalized.push(operation);
    }
    normalized
}

#[allow(clippy::too_many_arguments)]
fn translate_package_patch(
    patch: NativePatchEnvelope,
    namespace: Option<&str>,
    mount: u64,
    owner: &PackageIdentity,
    runtime: u64,
    generation: u64,
    authorities: &mut BTreeMap<u64, MountPatchAuthority>,
    expansion: &mut ExpansionState,
    output: &mut Vec<NativePatchOperation>,
    visited: &mut u64,
    payloads_expanded: bool,
    asset_aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    fn rewrite_asset_value(
        property: &str,
        value: &mut Value,
        asset_aliases: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        if matches!(property, "asset" | "icon")
            && let Value::String(name) = value
        {
            if name.starts_with("composition.asset.") {
                return Err("reserved native asset reference".into());
            }
            if let Some(alias) = asset_aliases.get(name) {
                *name = alias.clone();
            }
        }
        Ok(())
    }

    fn namespaced(namespace: Option<&str>, value: String) -> String {
        namespace.map_or(value.clone(), |prefix| format!("{prefix}::{value}"))
    }
    fn rewrite_payload(
        value: &mut Value,
        namespace: Option<&str>,
        mount: u64,
        owner: &PackageIdentity,
        runtime: u64,
        generation: u64,
        authority: &mut MountPatchAuthority,
        expansion: &mut ExpansionState,
        asset_aliases: &BTreeMap<String, String>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > MAX_TREE_DEPTH {
            return Err("native patch payload exceeds depth limit".into());
        }
        if matches!(
            value.get("kind").and_then(Value::as_str),
            Some("__packageComponent" | "__packageChild")
        ) {
            return Err("structural package transport requires a retained ownership splice".into());
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    rewrite_payload(
                        value,
                        namespace,
                        mount,
                        owner,
                        runtime,
                        generation,
                        authority,
                        expansion,
                        asset_aliases,
                        depth + 1,
                    )?;
                }
            }
            Value::Object(object) => {
                let slots = object
                    .get("__handlerSlots")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (property, value) in object.iter_mut() {
                    if property == "__handlerSlots" {
                        continue;
                    }
                    if is_action(property) && !value.is_null() {
                        let action = value.as_u64().ok_or("invalid patched component action")?;
                        let slot = slots
                            .get(property)
                            .and_then(Value::as_str)
                            .ok_or("patched action has no stable handler slot")?
                            .to_owned();
                        let token = expansion.next_event;
                        expansion.next_event = token
                            .checked_add(1)
                            .ok_or("expanded event identity exhausted")?;
                        expansion.events.insert(
                            token,
                            ComponentEventHandle {
                                runtime,
                                mount,
                                generation,
                                action,
                                slot: slot.clone(),
                                owner: owner.clone(),
                            },
                        );
                        authority.slots.insert(slot, token);
                        *value = Value::from(token);
                    } else {
                        rewrite_asset_value(property, value, asset_aliases)?;
                        rewrite_payload(
                            value,
                            namespace,
                            mount,
                            owner,
                            runtime,
                            generation,
                            authority,
                            expansion,
                            asset_aliases,
                            depth + 1,
                        )?;
                    }
                }
                if let Some(prefix) = namespace {
                    if let Some(id) = object
                        .get_mut("__nativeId")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                    {
                        object.insert(
                            "__nativeId".into(),
                            Value::String(format!("{prefix}::{id}")),
                        );
                    }
                    if let Some(slots) = object
                        .get_mut("__handlerSlots")
                        .and_then(Value::as_object_mut)
                    {
                        for slot in slots.values_mut() {
                            let local = slot.as_str().ok_or("invalid handler slot")?;
                            *slot = Value::String(format!("{prefix}::{local}"));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    let authority = authorities.entry(mount).or_default();
    for operation in patch.operations {
        *visited = visited.saturating_add(1);
        match operation {
            NativePatchOperation::SetPrimitive {
                target,
                property,
                mut value,
            } => {
                rewrite_asset_value(&property, &mut value, asset_aliases)?;
                output.push(NativePatchOperation::SetPrimitive {
                    target: namespaced(namespace, target),
                    property,
                    value,
                });
            }
            NativePatchOperation::ReplaceHandlerSlot { slot, action } => {
                let token = *authority
                    .slots
                    .get(&slot)
                    .ok_or("patched handler slot has no retained authority")?;
                expansion.events.insert(
                    token,
                    ComponentEventHandle {
                        runtime,
                        mount,
                        generation,
                        action: action as u64,
                        slot,
                        owner: owner.clone(),
                    },
                );
                // The renderer already addresses this stable expanded token.
                // Only its host-owned action authority changes.
            }
            NativePatchOperation::InsertChild {
                parent,
                key,
                child_id,
                index,
                mut node,
            } => {
                if !payloads_expanded {
                    let path = authority
                        .paths
                        .get(&parent)
                        .map(|path| {
                            format!(
                                "{path}/children/{}",
                                composition_child_identity(&node, index)
                            )
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "{}/splice:{}",
                                namespace.unwrap_or("root"),
                                child_id.replace('/', "%2f")
                            )
                        });
                    authority.record_paths(&node, &path, 0)?;
                    rewrite_payload(
                        &mut node,
                        namespace,
                        mount,
                        owner,
                        runtime,
                        generation,
                        authority,
                        expansion,
                        asset_aliases,
                        0,
                    )?;
                }
                output.push(NativePatchOperation::InsertChild {
                    parent: namespaced(namespace, parent),
                    key,
                    child_id: namespaced(namespace, child_id),
                    index,
                    node,
                });
            }
            NativePatchOperation::RemoveChild {
                parent,
                key,
                child_id,
                index,
            } => {
                authority.forget_paths(&child_id);
                if !payloads_expanded {
                    authority.slots.retain(|slot, token| {
                        let keep = !native_slot_in_subtree(slot, &child_id);
                        if !keep {
                            expansion.events.remove(token);
                        }
                        keep
                    });
                }
                output.push(NativePatchOperation::RemoveChild {
                    parent: namespaced(namespace, parent),
                    key,
                    child_id: namespaced(namespace, child_id),
                    index,
                });
            }
            NativePatchOperation::MoveChild {
                parent,
                key,
                child_id,
                from,
                to,
            } => output.push(NativePatchOperation::MoveChild {
                parent: namespaced(namespace, parent),
                key,
                child_id: namespaced(namespace, child_id),
                from,
                to,
            }),
            NativePatchOperation::ReplaceSubtree { target, mut node } => {
                if !payloads_expanded {
                    let path = authority.paths.get(&target).cloned().unwrap_or_else(|| {
                        format!(
                            "{}/splice:{}",
                            namespace.unwrap_or("root"),
                            target.replace('/', "%2f")
                        )
                    });
                    authority.forget_paths(&target);
                    authority.record_paths(&node, &path, 0)?;
                    authority.slots.retain(|slot, token| {
                        let keep = !native_slot_in_subtree(slot, &target);
                        if !keep {
                            expansion.events.remove(token);
                        }
                        keep
                    });
                }
                if !payloads_expanded {
                    rewrite_payload(
                        &mut node,
                        namespace,
                        mount,
                        owner,
                        runtime,
                        generation,
                        authority,
                        expansion,
                        asset_aliases,
                        0,
                    )?;
                }
                output.push(NativePatchOperation::ReplaceSubtree {
                    target: namespaced(namespace, target),
                    node,
                });
            }
        }
    }
    Ok(())
}

fn is_action(key: &str) -> bool {
    matches!(
        key,
        "action"
            | "__callbackAction"
            | "contextAction"
            | "dragAction"
            | "dropAction"
            | "focusAction"
            | "blurAction"
            | "selectAction"
            | "moveAction"
            | "fileAction"
            | "closeAction"
            | "escapeAction"
            | "submitAction"
    )
}
fn surface(id: u64) -> String {
    format!("composition-{id}")
}
fn bounded_json(value: &Value) -> Result<(), String> {
    // Count the exact encoded size without allocating a second full snapshot.
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.0 {
                return Err(std::io::Error::other("composition JSON exceeds size limit"));
            }
            self.0 -= bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(MAX_JSON_BYTES), value)
        .map_err(|_| "composition JSON exceeds size limit".into())
}

fn namespace_native_metadata(value: &mut Value, prefix: &str, depth: usize) -> Result<(), String> {
    if depth > MAX_TREE_DEPTH || prefix.len() > 512 {
        return Err("native identity namespace exceeds limit".into());
    }
    match value {
        Value::Array(values) => {
            for value in values {
                namespace_native_metadata(value, prefix, depth + 1)?;
            }
        }
        Value::Object(object) => {
            if let Some(id) = object.get_mut("__nativeId") {
                let identity = id
                    .as_str()
                    .ok_or("native node identity must be a string")?
                    .to_owned();
                if identity.is_empty() || identity.len() > 512 {
                    return Err("native node identity exceeds limit".into());
                }
                *id = Value::String(format!("{prefix}::{identity}"));
            }
            if let Some(slots) = object.get_mut("__handlerSlots") {
                let slots = slots
                    .as_object_mut()
                    .ok_or("native handler slots must be an object")?;
                for slot in slots.values_mut() {
                    let value = slot
                        .as_str()
                        .ok_or("native handler slot must be a string")?;
                    if value.is_empty() || value.len() > 640 {
                        return Err("native handler slot exceeds limit".into());
                    }
                    *slot = Value::String(format!("{prefix}::{value}"));
                }
            }
            for child in object.values_mut() {
                namespace_native_metadata(child, prefix, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_native_metadata(
    object: &serde_json::Map<String, Value>,
    expansion: &mut ExpansionState,
) -> Result<(), String> {
    let Some(id) = object.get("__nativeId") else {
        if object.contains_key("__handlerSlots") {
            return Err("native handler slots require a node identity".into());
        }
        return Ok(());
    };
    let id = id.as_str().ok_or("native node identity must be a string")?;
    if id.is_empty() || id.len() > 1024 || !expansion.native_ids.insert(id.to_owned()) {
        return Err("invalid or duplicate native node identity".into());
    }
    if let Some(slots) = object.get("__handlerSlots") {
        let slots = slots
            .as_object()
            .ok_or("native handler slots must be an object")?;
        for slot in slots.values() {
            let slot = slot
                .as_str()
                .ok_or("native handler slot must be a string")?;
            if slot.is_empty()
                || slot.len() > 1280
                || !slot.starts_with(id.split("::").next().unwrap_or(id))
                || !expansion.handler_slots.insert(slot.to_owned())
            {
                return Err("invalid or duplicate native handler slot".into());
            }
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn rewrite_events(
    value: &mut Value,
    runtime: u64,
    mount: u64,
    generation: u64,
    owner: &PackageIdentity,
    events: &mut BTreeMap<u64, ComponentEventHandle>,
    budget: &mut TreeBudget,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_TREE_DEPTH {
        return Err("component tree exceeds depth limit".into());
    }
    if value
        .as_object()
        .is_some_and(|object| object.get("kind").and_then(Value::as_str).is_some())
    {
        budget.nodes += 1;
    }
    if budget.nodes > MAX_COMPONENT_NODES {
        return Err("component tree exceeds node limit".into());
    }
    match value {
        Value::Array(values) => {
            for value in values {
                rewrite_events(
                    value,
                    runtime,
                    mount,
                    generation,
                    owner,
                    events,
                    budget,
                    depth + 1,
                )?;
            }
        }
        Value::Object(object) => {
            let slots = object
                .get("__handlerSlots")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (key, value) in object {
                if key == "__handlerSlots" {
                    continue;
                } else if is_action(key) && !value.is_null() {
                    let action = value
                        .as_u64()
                        .filter(|action| *action < MAX_HANDLERS as u64)
                        .ok_or("invalid component action")?;
                    let slot = slots
                        .get(key)
                        .and_then(Value::as_str)
                        .ok_or("component action has no stable handler slot")?
                        .to_owned();
                    if events.len() >= MAX_HANDLERS {
                        return Err("component tree exceeds handler limit".into());
                    }
                    let token = events.len() as u64;
                    events.insert(
                        token,
                        ComponentEventHandle {
                            runtime,
                            mount,
                            generation,
                            action,
                            slot,
                            owner: owner.clone(),
                        },
                    );
                    *value = Value::from(token);
                } else {
                    rewrite_events(
                        value,
                        runtime,
                        mount,
                        generation,
                        owner,
                        events,
                        budget,
                        depth + 1,
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Default)]
struct TreeBudget {
    nodes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::package_composition::{CompositionGrants, SemanticContribution};
    use nickel_core::plugins::{PluginCapability, PluginManifest};
    use serde_json::json;

    fn package(id: &str, source: &str, base: Option<&str>) -> PluginPackage {
        let mut manifest = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/nickel-default/plugin.json"
        ))
        .unwrap();
        manifest.id = id.into();
        manifest.entry = "main.js".into();
        let composition = manifest.composition.as_mut().unwrap();
        composition.id = id.into();
        composition.exports = BTreeMap::from([
            ("shell.taskbar".into(), "./main.js#Taskbar".into()),
            (
                "shell.quickSettings".into(),
                "./main.js#QuickSettings".into(),
            ),
        ]);
        if let Some(base) = base {
            composition.exports.clear();
            composition.extends = Some(base.into());
            composition.requires.insert(base.into(), "^0.2".into());
            composition.uses.insert(
                "shell.quickSettings".into(),
                nickel_core::package_composition::PublicExportReference {
                    package: base.into(),
                    export: "shell.quickSettings".into(),
                },
            );
            composition
                .replaces
                .insert("shell.taskbar".into(), "./main.js#Taskbar".into());
            composition.contributions.push(SemanticContribution {
                collection: "settings.pages".into(),
                id: "theme".into(),
                implementation: "./main.js#Taskbar".into(),
                priority: 0,
            });
        }
        PluginPackage {
            manifest,
            source: source.into(),
            stylesheet: String::new(),
            modules: Vec::new(),
            images: BTreeMap::new(),
        }
    }

    fn make_host() -> ShellCompositionRuntime {
        let base = package(
            "base",
            "globalThis.secret = 'base';\nexport function Taskbar() { return h(Button, {onClick: () => nickel.windows.activate('base-window')}, secret); }\nexport function QuickSettings() { return h(Button, {onClick: () => nickel.windows.activate('base-window')}, globalThis.secret); }\nexport default function App() { return h(Taskbar, null); }",
            None,
        );
        let child = package(
            "child",
            "globalThis.secret = 'child';\nexport function Taskbar() { const [count, setCount] = useState(0); return h(Button, {onClick: () => { setCount(count + 1); nickel.request({type:'windows.focus', id:'child-window', owner:'base'}); }}, globalThis.secret + count); }\nexport default function App() { return h(Taskbar, null); }",
            Some("base"),
        );
        ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap()
    }

    fn transactional_cross_owner_host() -> ShellCompositionRuntime {
        let mut base = package(
            "base",
            "export function Shell(){const [state]=useState({count:0});const ref=useRef({count:0});return h(Column,null,h(Text,null,'base'+state.count+':'+ref.current.count),h(nickel.component('shell.taskbar'),{onChange:()=>{state.count++;ref.current.count++;nickel.windows.activate('base');}}),h(nickel.component('shell.quickSettings')));}\nexport function Taskbar(){}\nexport function QuickSettings(){return h(Text,null,'clean-owner');}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(props){const [count,setCount]=useState(0);return h(Button,{onClick:()=>{setCount(count+1);props.onChange();nickel.windows.activate('child');}},'child'+count);}\nexport default Taskbar;",
            Some("base"),
        );
        ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn maintenance_owner_selection_skips_idle_and_rotates_pending_owners() {
        let mut host = transactional_cross_owner_host();
        let owners = host.participating_owners().cloned().collect::<Vec<_>>();
        let mut cursor = 0;
        assert_eq!(
            host.service_next_pending_platform_owner(&mut cursor)
                .unwrap()
                .unwrap()
                .0,
            owners[0]
        );
        assert_eq!(
            host.service_next_pending_platform_owner(&mut cursor)
                .unwrap()
                .unwrap()
                .0,
            owners[1]
        );
        for _ in 0..32 {
            if host
                .service_next_pending_platform_owner(&mut cursor)
                .unwrap()
                .is_none()
            {
                break;
            }
        }
        assert!(!host.platform_maintenance_pending());
        cursor = 0;
        host.shared_owner_runtime(&owners[1])
            .unwrap()
            .borrow_mut()
            .eval("void 0")
            .unwrap();
        assert_eq!(
            host.service_next_pending_platform_owner(&mut cursor)
                .unwrap()
                .unwrap()
                .0,
            owners[1]
        );
        host.begin_transaction().unwrap();
        let saved = cursor;
        assert!(
            host.service_next_pending_platform_owner(&mut cursor)
                .is_err()
        );
        assert_eq!(cursor, saved);
        host.finish_transaction(false).unwrap();
        host.retire(&owners[0]);
        cursor = usize::MAX;
        assert_eq!(
            host.service_next_pending_platform_owner(&mut cursor)
                .unwrap()
                .unwrap()
                .0,
            owners[1]
        );
    }

    #[test]
    fn platform_maintenance_rejects_composition_transactions_and_retired_owners() {
        let mut host = transactional_cross_owner_host();
        let owners = host.participating_owners().cloned().collect::<Vec<_>>();
        assert_eq!(owners.len(), 2);
        assert!(host.platform_maintenance_pending());
        for owner in &owners {
            for _ in 0..16 {
                if host.service_owner_platform_tasks(owner).unwrap() == 0 {
                    break;
                }
            }
            assert!(!host.owner_platform_maintenance_pending(owner).unwrap());
        }
        assert!(!host.platform_maintenance_pending());
        host.shared_owner_runtime(&owners[0])
            .unwrap()
            .borrow_mut()
            .eval("void 0")
            .unwrap();
        assert!(host.platform_maintenance_pending());
        assert!(host.owner_platform_maintenance_pending(&owners[0]).unwrap());
        assert!(!host.owner_platform_maintenance_pending(&owners[1]).unwrap());
        host.begin_transaction().unwrap();
        assert!(!host.platform_maintenance_pending());
        for owner in &owners {
            assert!(!host.owner_platform_maintenance_pending(owner).unwrap());
            assert!(
                host.service_owner_platform_tasks(owner)
                    .unwrap_err()
                    .contains("composition transaction")
            );
        }
        host.finish_transaction(false).unwrap();
        for owner in &owners {
            assert!(host.service_owner_platform_tasks(owner).unwrap() <= 8);
        }
        host.retire(&owners[0]);
        assert!(host.service_owner_platform_tasks(&owners[0]).is_err());
        assert!(host.owner_platform_maintenance_pending(&owners[0]).is_err());
    }

    #[test]
    fn transaction_snapshots_share_immutable_inventory_and_restore_exact_owner() {
        let mut host = transactional_cross_owner_host();
        let owner = host.packages.keys().next().unwrap().clone();
        let inventory = json!({"inventory": (0..1000)
            .map(|index| json!({"id":index,"label":format!("Row {index}")}))
            .collect::<Vec<_>>()});
        host.update_snapshot(&owner, &inventory).unwrap();
        let original = host.packages[&owner].data.clone();
        host.begin_transaction().unwrap();
        assert!(std::rc::Rc::ptr_eq(
            &original,
            &host.checkpoint.as_ref().unwrap().data[&owner]
        ));
        let replacement = json!({"inventory":[],"revision":2});
        host.update_snapshot(&owner, &replacement).unwrap();
        assert!(!std::rc::Rc::ptr_eq(&original, &host.packages[&owner].data));
        assert_eq!(original.as_ref(), &inventory);
        host.finish_transaction(false).unwrap();
        assert!(std::rc::Rc::ptr_eq(&original, &host.packages[&owner].data));
        assert_eq!(host.snapshot(&owner).unwrap(), &inventory);
        host.begin_transaction().unwrap();
        host.update_snapshot(&owner, &replacement).unwrap();
        host.finish_transaction(true).unwrap();
        assert_eq!(host.snapshot(&owner).unwrap(), &replacement);
        assert_eq!(original.as_ref(), &inventory);
    }

    #[test]
    fn catalog_reload_preserves_compatible_hooks_and_rejects_stale_events() {
        let old = package(
            "base",
            "export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'old:'+count)}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut catalog = BTreeMap::from([("base".into(), old)]);
        let mut host = ShellCompositionRuntime::new(&catalog, "base", &BTreeMap::new()).unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let rendered = host
            .render_expanded(&mount, &json!({}), |_| Ok(()))
            .unwrap();
        let stale = rendered.events[&0].clone();
        host.dispatch(&stale, &Value::Null).unwrap();

        catalog.insert("base".into(), package(
            "base",
            "export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'new:'+count)}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        ));
        let signature = BTreeMap::from([("main.js#Taskbar".into(), "state".into())]);
        let reload = host
            .reload_catalog(
                &catalog,
                &BTreeMap::from([(
                    host.resolution.active.clone(),
                    PackageReloadSignatures {
                        previous: signature.clone(),
                        replacement: signature,
                    },
                )]),
            )
            .unwrap();
        assert_eq!(reload.preserved_packages.len(), 1);
        assert_eq!(reload.patches[0].patch.operations.len(), 1);
        assert_eq!(
            reload.patches[0].patch.operations[0],
            NativePatchOperation::ReplaceSubtree {
                target: "root".into(),
                node: reload.patches[0]
                    .patch
                    .operations
                    .iter()
                    .find_map(|op| match op {
                        NativePatchOperation::ReplaceSubtree { node, .. } => Some(node.clone()),
                        _ => None,
                    })
                    .unwrap()
            }
        );
        if let NativePatchOperation::ReplaceSubtree { node, .. } =
            &reload.patches[0].patch.operations[0]
        {
            assert_eq!(node["children"][0], "new:1");
        }
        assert!(host.dispatch(&stale, &Value::Null).is_err());
    }

    #[test]
    fn rejected_catalog_reload_leaves_the_accepted_graph_live() {
        let old = package(
            "base",
            "export function Taskbar(){return h(Text,null,'accepted')}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut catalog = BTreeMap::from([("base".into(), old)]);
        let mut host = ShellCompositionRuntime::new(&catalog, "base", &BTreeMap::new()).unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        catalog.get_mut("base").unwrap().source = "export function Taskbar( {".into();
        assert!(host.reload_catalog(&catalog, &BTreeMap::new()).is_err());
        let rendered = host
            .render_expanded(&mount, &json!({}), |_| Ok(()))
            .unwrap();
        assert_eq!(rendered.node["children"][0], "accepted");
    }

    #[test]
    fn failed_checkpoint_restoration_invalidates_all_executable_participants() {
        let package = package(
            "base",
            "export function Taskbar(){const [state]=useState({count:0});return h(Button,{onClick:()=>{state.count++;Object.freeze(state);nickel.windows.activate('owned');}},String(state.count));}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), package)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let escaped_runtime = host
            .shared_owner_runtime(&host.resolution().active)
            .unwrap();
        let error = host
            .dispatch_expanded(&root, &initial.events[&0], &Value::Null, |_| {
                Err("native rejected".into())
            })
            .err()
            .unwrap();
        assert!(error.contains("checkpoint failed"));
        assert!(
            escaped_runtime
                .borrow_mut()
                .eval("globalThis.afterFailure=true")
                .is_err()
        );
        assert!(host.packages.is_empty());
        assert!(host.mounts.is_empty());
        assert!(host.take_effects().is_empty());
        assert!(host.dispatch(&initial.events[&0], &Value::Null).is_err());
    }

    #[test]
    fn rejected_expansion_restores_all_owner_hooks_handlers_and_effects() {
        let mut host = transactional_cross_owner_host();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(
            host.dispatch_expanded(&root, &initial.events[&0], &Value::Null, |_| Err(
                "native tree rejected".into()
            ))
            .is_err()
        );
        assert!(host.take_effects().is_empty());
        // A rejected attempt leaves the previously accepted handler usable.
        let accepted = host
            .dispatch_expanded(&root, &initial.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(accepted.node.to_string().contains("base1:1"));
        assert!(accepted.node.to_string().contains("child1"));
        let effects = host.take_effects();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].owner().id, "base");
        assert_eq!(effects[1].owner().id, "child");
    }

    #[test]
    fn denied_native_effect_batch_restores_cross_owner_state_and_revokes_provisional_handles() {
        let mut host = transactional_cross_owner_host();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let provisional = host
            .dispatch_expanded_pending(&root, &initial.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(provisional.node.to_string().contains("base1:1"));
        assert_eq!(host.take_effects().len(), 2);
        // Native approval rejects the complete batch, including the allowed
        // callback that ran before the later forbidden child operation.
        host.finish_transaction(false).unwrap();
        assert!(host.take_effects().is_empty());
        assert!(
            host.dispatch(&provisional.events[&0], &Value::Null)
                .is_err()
        );
        let restored = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(restored.node.to_string().contains("base0:0"));
        assert!(restored.node.to_string().contains("child0"));
        assert!(
            host.dispatch(&provisional.events[&0], &Value::Null)
                .is_err()
        );
    }

    #[test]
    fn foreign_registered_page_expands_in_published_owner_context() {
        let base = package(
            "base",
            "export function Taskbar(){ const page=readPluginSettingsPages().pages[0]; return h(page.component,null); }\nexport function QuickSettings(){return h(Text,null,'base');}\nexport default Taskbar;",
            None,
        );
        let mut child = package(
            "child",
            "registerSettingsPage({id:'details',group:'Plugins',label:'Details',component:()=>h(Button,{onClick:()=>nickel.windows.activate('provider-window')},'provider-page')});\nexport function Taskbar(){return h(Text,null,'unused');}\nexport default Taskbar;",
            Some("base"),
        );
        child
            .manifest
            .composition
            .as_mut()
            .unwrap()
            .replaces
            .clear();
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        for (owner, package) in &host.packages {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.packages.values() {
            package
                .runtime
                .borrow_mut()
                .set_settings_registry(&registry)
                .unwrap();
        }
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let tree = host
            .render_expanded(&mount, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("provider-page"));
        let event = tree.events.values().next().unwrap();
        assert_eq!(event.owner().id, "child");
        host.dispatch_expanded(&mount, event, &serde_json::json!(null), |_| Ok(()))
            .unwrap();
        let effects = host.take_effects();
        assert_eq!(effects[0].owner().id, "child");
        assert_eq!(effects[0].value()["id"], "provider-window");
    }

    fn accept_scheduled_event(
        host: &mut ShellCompositionRuntime,
        mount: &ComponentMount,
        event: ComponentEventHandle,
        accepted: &RenderedComponent,
    ) -> RenderedComponent {
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                mount,
                &[(event, Value::Null)],
                &accepted.events,
                &accepted.node,
                |_, _, _| Ok(()),
            )
            .unwrap();
        if !matches!(outcome, ScheduledExpandedBatch::Patched { .. }) {
            panic!("failure transition must emit a typed patch")
        }
        host.finish_transaction(true).unwrap();
        host.render_expanded(mount, &json!({}), |_| Ok(())).unwrap()
    }

    fn package_boundary_diagnostics(host: &mut ShellCompositionRuntime, id: &str) -> Vec<Value> {
        host.packages
            .values_mut()
            .find(|package| package.manifest.id == id)
            .unwrap()
            .runtime
            .borrow_mut()
            .boundary_diagnostics()
            .unwrap()
    }

    fn same_package_surface_failure_host() -> ShellCompositionRuntime {
        let mut base = package(
            "base",
            r#"
                globalThis.failTaskbar=false;
                function TaskbarLeaf(){const [count,setCount]=useState(0);if(failTaskbar)throw Error('taskbar render boom');return h(Column,null,h(Button,{onClick:()=>setCount(count+1)},'taskbar:'+count),h(Button,{onClick:()=>{failTaskbar=true;setCount(count+1)}},'fail taskbar'))}
                export function Taskbar(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:()=>{failTaskbar=false;reset()}},'taskbar fallback:'+error.message)},h(TaskbarLeaf))}
                function LauncherLeaf(){const [count,setCount]=useState(0);return h(Column,null,h(Button,{onClick:()=>setCount(count+1)},'launcher:'+count),h(Button,{onClick:()=>{throw Error('launcher event boom')}},'fail launcher'))}
                export function Launcher(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:reset},'launcher fallback:'+error.message)},h(LauncherLeaf))}
                export function QuickSettings(){}
                export default Taskbar;
            "#,
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell.launcher".into(), "./main.js#Launcher".into());
        ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn failure_matrix_taskbar_render_isolated_from_launcher_and_recovers() {
        let mut host = same_package_surface_failure_host();
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let launcher = host
            .mount(&host.component("shell.launcher").unwrap())
            .unwrap();
        let initial_taskbar = host
            .render_expanded(&taskbar, &json!({}), |_| Ok(()))
            .unwrap();
        let initial_launcher = host
            .render_expanded(&launcher, &json!({}), |_| Ok(()))
            .unwrap();
        let launcher_identity = initial_launcher.node["__nativeId"].clone();

        let fallback = accept_scheduled_event(
            &mut host,
            &taskbar,
            initial_taskbar.events[&1].clone(),
            &initial_taskbar,
        );
        assert!(
            fallback
                .node
                .to_string()
                .contains("taskbar fallback:taskbar render boom")
        );
        assert_eq!(
            host.packages.len(),
            1,
            "a local render failure keeps package health"
        );

        let launcher_after = accept_scheduled_event(
            &mut host,
            &launcher,
            initial_launcher.events[&0].clone(),
            &initial_launcher,
        );
        assert!(launcher_after.node.to_string().contains("launcher:1"));
        assert_eq!(launcher_after.node["__nativeId"], launcher_identity);

        let recovered = accept_scheduled_event(
            &mut host,
            &taskbar,
            fallback.events.values().next().unwrap().clone(),
            &fallback,
        );
        assert!(recovered.node.to_string().contains("taskbar:0"));
        assert!(
            package_boundary_diagnostics(&mut host, "base")
                .iter()
                .any(|entry| entry["phase"] == "render")
        );
        assert!(
            host.dispatch(&initial_taskbar.events[&0], &Value::Null)
                .is_err()
        );
    }

    #[test]
    fn failure_matrix_launcher_event_isolated_from_taskbar_and_resets() {
        let mut host = same_package_surface_failure_host();
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let launcher = host
            .mount(&host.component("shell.launcher").unwrap())
            .unwrap();
        let initial_taskbar = host
            .render_expanded(&taskbar, &json!({}), |_| Ok(()))
            .unwrap();
        let initial_launcher = host
            .render_expanded(&launcher, &json!({}), |_| Ok(()))
            .unwrap();
        let taskbar_identity = initial_taskbar.node["__nativeId"].clone();

        let fallback = accept_scheduled_event(
            &mut host,
            &launcher,
            initial_launcher.events[&1].clone(),
            &initial_launcher,
        );
        assert!(
            fallback
                .node
                .to_string()
                .contains("launcher fallback:launcher event boom")
        );
        let recovered = accept_scheduled_event(
            &mut host,
            &launcher,
            fallback.events.values().next().unwrap().clone(),
            &fallback,
        );
        assert!(recovered.node.to_string().contains("launcher:0"));

        let taskbar_after = accept_scheduled_event(
            &mut host,
            &taskbar,
            initial_taskbar.events[&0].clone(),
            &initial_taskbar,
        );
        assert!(taskbar_after.node.to_string().contains("taskbar:1"));
        assert_eq!(taskbar_after.node["__nativeId"], taskbar_identity);
        assert_eq!(host.packages.len(), 1);
        assert!(
            package_boundary_diagnostics(&mut host, "base")
                .iter()
                .any(|entry| entry["phase"] == "event")
        );
    }

    #[test]
    fn failure_matrix_settings_effect_isolated_from_taskbar_and_recovers() {
        let mut base = package(
            "base",
            r#"
                globalThis.failSettings=true;
                export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'taskbar:'+count)}
                function SettingsLeaf(){useEffect(()=>{if(failSettings){failSettings=false;throw Error('settings effect boom')}},[]);return h(Text,null,'settings ready')}
                export function Settings(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:reset},'settings fallback:'+error.message)},h(SettingsLeaf))}
                export function QuickSettings(){}
                export default Taskbar;
            "#,
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell.settings".into(), "./main.js#Settings".into());
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let settings = host
            .mount(&host.component("shell.settings").unwrap())
            .unwrap();
        let taskbar_tree = host
            .render_expanded(&taskbar, &json!({}), |_| Ok(()))
            .unwrap();
        let taskbar_identity = taskbar_tree.node["__nativeId"].clone();
        let settings_tree = host
            .render_expanded(&settings, &json!({}), |_| Ok(()))
            .unwrap();
        assert!(settings_tree.node.to_string().contains("settings ready"));

        let outcome = host
            .reconcile_expanded_pending_validated(
                &settings,
                &settings_tree.events,
                &settings_tree.node,
                |_, _, _| Ok(()),
            )
            .unwrap();
        assert!(matches!(outcome, ScheduledExpandedBatch::Patched { .. }));
        host.finish_transaction(true).unwrap();
        let fallback = host
            .render_expanded(&settings, &json!({}), |_| Ok(()))
            .unwrap();
        assert!(
            fallback
                .node
                .to_string()
                .contains("settings fallback:settings effect boom")
        );

        let taskbar_after = accept_scheduled_event(
            &mut host,
            &taskbar,
            taskbar_tree.events[&0].clone(),
            &taskbar_tree,
        );
        assert!(taskbar_after.node.to_string().contains("taskbar:1"));
        assert_eq!(taskbar_after.node["__nativeId"], taskbar_identity);
        let recovered = accept_scheduled_event(
            &mut host,
            &settings,
            fallback.events.values().next().unwrap().clone(),
            &fallback,
        );
        assert!(recovered.node.to_string().contains("settings ready"));
        assert_eq!(host.packages.len(), 1);
        assert!(
            package_boundary_diagnostics(&mut host, "base")
                .iter()
                .any(|entry| entry["phase"] == "effect")
        );
    }

    #[test]
    fn failure_matrix_contributed_settings_render_isolated_by_provider_boundary() {
        let base = package(
            "base",
            "export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'base taskbar:'+count)}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut provider = package(
            "provider",
            r#"
                globalThis.failPage=false;
                function PageLeaf(){const [count,setCount]=useState(0);if(failPage)throw Error('provider page boom');return h(Button,{onClick:()=>{failPage=true;setCount(count+1)}},'provider page:'+count)}
                function Page(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:()=>{failPage=false;reset()}},'provider fallback:'+error.message)},h(PageLeaf))}
                registerSettingsPage({id:'details',group:'Plugins',label:'Provider',component:Page});
                export function Taskbar(){return h(Text,null,'unused')}
                export default Taskbar;
            "#,
            Some("base"),
        );
        provider
            .manifest
            .composition
            .as_mut()
            .unwrap()
            .replaces
            .clear();
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("provider".into(), provider)]),
            "provider",
            &BTreeMap::new(),
        )
        .unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        for (owner, package) in &host.packages {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.packages.values() {
            package
                .runtime
                .borrow_mut()
                .set_settings_registry(&registry)
                .unwrap();
        }
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let page = host
            .mount(&host.registered_page("provider", "details").unwrap())
            .unwrap();
        let taskbar_tree = host
            .render_expanded(&taskbar, &json!({}), |_| Ok(()))
            .unwrap();
        let taskbar_identity = taskbar_tree.node["__nativeId"].clone();
        let page_tree = host.render_expanded(&page, &json!({}), |_| Ok(())).unwrap();
        let fallback =
            accept_scheduled_event(&mut host, &page, page_tree.events[&0].clone(), &page_tree);
        assert!(
            fallback
                .node
                .to_string()
                .contains("provider fallback:provider page boom")
        );
        assert_eq!(host.packages.len(), 2);

        let taskbar_after = accept_scheduled_event(
            &mut host,
            &taskbar,
            taskbar_tree.events[&0].clone(),
            &taskbar_tree,
        );
        assert!(taskbar_after.node.to_string().contains("base taskbar:1"));
        assert_eq!(taskbar_after.node["__nativeId"], taskbar_identity);
        let recovered = accept_scheduled_event(
            &mut host,
            &page,
            fallback.events.values().next().unwrap().clone(),
            &fallback,
        );
        assert!(recovered.node.to_string().contains("provider page:0"));
        assert!(
            package_boundary_diagnostics(&mut host, "provider")
                .iter()
                .any(|entry| entry["phase"] == "render")
        );
        assert!(host.dispatch(&page_tree.events[&0], &Value::Null).is_err());
    }

    #[test]
    fn rejected_settings_patch_does_not_retire_taskbar_surface_or_handlers() {
        let base = package(
            "base",
            "function SettingsLeaf(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'settings'+count)}\nregisterSettingsPage({id:'details',group:'Plugins',label:'Details',component:()=>h(ErrorBoundary,{fallback:h(Text,null,'settings fallback')},h(SettingsLeaf))});\nexport function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},'taskbar'+count)}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        for (owner, package) in &host.packages {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.packages.values() {
            package
                .runtime
                .borrow_mut()
                .set_settings_registry(&registry)
                .unwrap();
        }
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let settings = host
            .mount(&host.registered_page("base", "details").unwrap())
            .unwrap();
        let taskbar_tree = host
            .render_expanded(&taskbar, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let settings_tree = host
            .render_expanded(&settings, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let settings_event = settings_tree.events[&0].clone();
        let mut attempts = 0;
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &settings,
                &[(settings_event, Value::Null)],
                &settings_tree.events,
                &settings_tree.node,
                |patch, _, _| {
                    attempts += 1;
                    if attempts == 1 {
                        Err("settings native rejection".into())
                    } else {
                        assert!(
                            serde_json::to_string(patch)
                                .unwrap()
                                .contains("settings fallback")
                        );
                        Ok(())
                    }
                },
            )
            .unwrap();
        assert!(matches!(outcome, ScheduledExpandedBatch::Patched { .. }));
        host.finish_transaction(true).unwrap();

        let taskbar_after = host
            .dispatch_expanded(&taskbar, &taskbar_tree.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(taskbar_after.node.to_string().contains("taskbar1"));
        assert!(!taskbar_after.node.to_string().contains("settings fallback"));
    }

    #[test]
    fn admitted_mount_without_patch_authority_fails_before_running_its_event() {
        let base = package(
            "base",
            "globalThis.renders=0;\nexport function Taskbar(){renders++;const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},String(count));}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let authority = host.patch_authority.remove(&root.id).unwrap();

        let result = host.dispatch_expanded_batch_scheduled_pending_validated(
            &root,
            &[(initial.events[&0].clone(), Value::Null)],
            &initial.events,
            &initial.node,
            |_, _, _| Ok(()),
        );
        let Err(error) = result else {
            panic!("missing admitted authority must fail")
        };
        assert!(error.contains("lost native patch authority"));
        assert!(
            host.checkpoint.is_none(),
            "authority failure must roll back"
        );
        let runtime = host
            .shared_owner_runtime(&host.resolution().active)
            .unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("JSON.stringify(renders)")
                .unwrap(),
            1,
            "authority loss must not silently run a complete-tree render"
        );

        host.patch_authority.insert(root.id, authority);
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(initial.events[&0].clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.counters),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { validated, .. } = outcome else {
            panic!("restored authority must admit a typed patch")
        };
        assert_eq!(validated.local_materializations, 0);
        assert_eq!(validated.tree_bytes, 0);
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn missing_nested_patch_authority_isolated_before_any_package_runs() {
        let mut base = package(
            "base",
            "globalThis.renders=0;\nexport function Shell(){renders++;return h(Column,null,h(Text,null,'clean'),h(nickel.component('shell.taskbar')));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "globalThis.renders=0;\nexport function Taskbar(){renders++;const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},String(count));}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let event = initial.events[&0].clone();
        assert_ne!(event.mount, root.id);
        let authority = host.patch_authority.remove(&event.mount).unwrap();

        let result = host.dispatch_expanded_batch_scheduled_pending_validated(
            &root,
            &[(event.clone(), Value::Null)],
            &initial.events,
            &initial.node,
            |_, _, _| Ok(()),
        );
        let Err(error) = result else {
            panic!("missing nested authority must fail")
        };
        assert!(error.contains("lost native patch authority"));
        assert!(
            host.checkpoint.is_none(),
            "authority failure must roll back"
        );
        for (owner, package) in &host.packages {
            assert_eq!(
                package
                    .runtime
                    .borrow_mut()
                    .eval_json::<u64>("JSON.stringify(renders)")
                    .unwrap(),
                1,
                "authority loss ran package {owner:?}"
            );
        }

        host.patch_authority.insert(event.mount, authority);
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event, Value::Null)],
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.counters),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { validated, .. } = outcome else {
            panic!("restored nested authority must admit a typed patch")
        };
        assert_eq!(validated.local_materializations, 0);
        assert_eq!(validated.tree_bytes, 0);
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn passive_reconciliation_does_not_consume_an_independent_settings_root() {
        let base = package(
            "base",
            "function SettingsLeaf(){const [count,setCount]=useState(0);useEffect(()=>setCount(1),[]);return h(Text,null,'settings'+count)}\nregisterSettingsPage({id:'details',group:'Plugins',label:'Details',component:SettingsLeaf});\nexport function Taskbar(){return h(Text,null,'taskbar')}\nexport function QuickSettings(){}\nexport default Taskbar;",
            None,
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        for (owner, package) in &host.packages {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.packages.values() {
            package
                .runtime
                .borrow_mut()
                .set_settings_registry(&registry)
                .unwrap();
        }
        let taskbar = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let settings = host
            .mount(&host.registered_page("base", "details").unwrap())
            .unwrap();
        let taskbar_tree = host
            .render_expanded(&taskbar, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let settings_tree = host
            .render_expanded(&settings, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(settings_tree.node.to_string().contains("settings0"));

        let unrelated = host
            .reconcile_expanded_pending_validated(
                &taskbar,
                &taskbar_tree.events,
                &taskbar_tree.node,
                |_, _, _| -> Result<(), String> {
                    panic!("taskbar reconciliation must not consume Settings dirtiness")
                },
            )
            .unwrap();
        assert!(matches!(unrelated, ScheduledExpandedBatch::Unchanged));
        host.finish_transaction(true).unwrap();

        let settings_update = host
            .reconcile_expanded_pending_validated(
                &settings,
                &settings_tree.events,
                &settings_tree.node,
                |patch, _, _| {
                    assert!(serde_json::to_string(patch).unwrap().contains("settings1"));
                    Ok(())
                },
            )
            .unwrap();
        assert!(matches!(
            settings_update,
            ScheduledExpandedBatch::Patched { .. }
        ));
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn independent_providers_share_context_and_retire_contributions_and_stale_authority() {
        let mut base = package(
            "base",
            "export function Shell(){return h(Column,null,...nickel.contributions('taskbar.items').map(entry=>h(entry.component,{key:entry.key})),...nickel.contributions('settings.pages').map(entry=>h(entry.component,{key:entry.key})));}\nexport function Taskbar(){return h(Text,null,'base');}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let mut provider = package(
            "provider",
            "globalThis.starts=(globalThis.starts||0)+1;\nexport function Item(){const [count,setCount]=useState(0);return h(Button,{id:'item',onClick:()=>{setCount(count+1);nickel.windows.activate('provider-window');}},'provider'+count);}\nexport function Page(){return h(Text,null,'independent-page');}",
            None,
        );
        let composition = provider.manifest.composition.as_mut().unwrap();
        composition.exports.clear();
        for (collection, id, implementation) in [
            ("taskbar.items", "item", "Item"),
            ("settings.pages", "page", "Page"),
        ] {
            composition.contributions.push(SemanticContribution {
                collection: collection.into(),
                id: id.into(),
                implementation: format!("./main.js#{implementation}"),
                priority: 10,
            });
        }
        let provider_catalog = BTreeMap::from([("provider".into(), provider.clone())]);
        let provider_host =
            ShellCompositionRuntime::new(&provider_catalog, "provider", &BTreeMap::new()).unwrap();
        let owner = provider_host.resolution().active.clone();
        let context = provider_host.provider_context(&owner).unwrap();
        let catalog = BTreeMap::from([
            ("base".into(), base.clone()),
            ("provider".into(), provider.clone()),
        ]);
        let mut host = ShellCompositionRuntime::new_with_contexts(
            &catalog,
            "base",
            &BTreeMap::new(),
            &BTreeMap::from([(owner.clone(), context)]),
        )
        .unwrap();
        assert_eq!(host.resolution().inheritance_chain.len(), 1);
        assert_eq!(host.participating_owners().count(), 2);
        assert!(std::rc::Rc::ptr_eq(
            &host.shared_owner_runtime(&owner).unwrap(),
            &provider_host.shared_owner_runtime(&owner).unwrap()
        ));
        assert_eq!(
            host.shared_owner_runtime(&owner)
                .unwrap()
                .borrow_mut()
                .eval_json::<u32>("JSON.stringify(globalThis.starts)")
                .unwrap(),
            1
        );
        let stale_reference = host.contributions("taskbar.items")[0].clone();
        let stale_guest_key =
            host.contribution_key(&host.resolution.contributions["taskbar.items"][0]);
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        fn native_ids(value: &Value, found: &mut std::collections::BTreeSet<String>) {
            match value {
                Value::Array(values) => {
                    for value in values {
                        native_ids(value, found);
                    }
                }
                Value::Object(object) => {
                    if let Some(id) = object.get("__nativeId").and_then(Value::as_str) {
                        assert!(found.insert(id.to_owned()), "duplicate composed ID {id}");
                    }
                    for value in object.values() {
                        native_ids(value, found);
                    }
                }
                _ => {}
            }
        }
        let mut accepted_ids = std::collections::BTreeSet::new();
        native_ids(&tree.node, &mut accepted_ids);
        assert!(accepted_ids.iter().any(|id| id.contains("::root")));
        assert!(tree.node.to_string().contains("provider0"));
        assert!(tree.node.to_string().contains("independent-page"));
        let stale_event = tree.events[&0].clone();
        assert_eq!(stale_event.owner().id, "provider");
        let tree = host
            .dispatch_expanded(&root, &stale_event, &Value::Null, |_| Ok(()))
            .unwrap();
        let mut updated_ids = std::collections::BTreeSet::new();
        native_ids(&tree.node, &mut updated_ids);
        assert_eq!(accepted_ids, updated_ids);
        assert!(tree.node.to_string().contains("provider1"));
        let stale_effect = host.take_effects().remove(0);
        assert_eq!(stale_effect.owner().id, "provider");
        host.retire(&owner);
        assert!(host.contributions("taskbar.items").is_empty());
        assert!(host.mount(&stale_reference).is_err());
        assert!(host.dispatch(&stale_event, &Value::Null).is_err());
        assert!(host.validate_effect(&stale_effect).is_err());
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(!tree.node.to_string().contains("provider"));
        let restarted =
            ShellCompositionRuntime::new(&provider_catalog, "provider", &BTreeMap::new()).unwrap();
        let contexts =
            BTreeMap::from([(owner.clone(), restarted.provider_context(&owner).unwrap())]);
        assert!(host.sync_contributors(&catalog, &contexts).unwrap());
        assert!(host.contribution(&stale_guest_key).is_none());
        assert!(host.mount(&stale_reference).is_err());
        assert!(host.validate_effect(&stale_effect).is_err());
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("provider0"));
        let mut without_provider = catalog.clone();
        without_provider.remove("provider");
        assert!(
            host.sync_contributors(&without_provider, &BTreeMap::new())
                .unwrap()
        );
        assert!(host.contributions("taskbar.items").is_empty());
        assert!(host.shared_owner_runtime(&owner).is_ok());
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(!tree.node.to_string().contains("provider"));
        host.retire(&owner);
        assert!(host.shared_owner_runtime(&owner).is_err());
    }

    #[test]
    fn expanded_public_lookup_renders_replacement_and_routes_its_callback() {
        let mut base = package(
            "base",
            "globalThis.secret = 'base';\nexport function Shell() { return h(Column, null, h(nickel.component('shell.taskbar'), {label:'custom'})); }\nexport function Taskbar() { return h(Text,null,'base'); }\nexport function QuickSettings() { return h(Text,null,'base'); }\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "globalThis.secret = 'child';\nexport function Taskbar(props) { const [count,setCount]=useState(0); return h(Button,{onClick:()=>{setCount(count+1);nickel.windows.activate('owned');}}, secret + props.label + count); }\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let reference = host.component("shell").unwrap();
        let mount = host.mount(&reference).unwrap();
        let tree = host
            .render_expanded(&mount, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("childcustom0"));
        assert_eq!(tree.events[&0].owner().id, "child");
        let tree = host
            .dispatch_expanded(&mount, &tree.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("childcustom1"));
        assert_eq!(host.take_effects()[0].owner().id, "child");
        host.unmount(&mount).unwrap();
        assert!(host.dispatch(&tree.events[&0], &Value::Null).is_err());
    }

    #[test]
    fn scheduled_cross_package_leaf_update_emits_one_namespaced_boundary_patch() {
        let mut base = package(
            "base",
            "export function Shell(){return h(Column,null,h(Text,{key:'clean'},'clean'),h(nickel.component('shell.taskbar')));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{key:'leaf',onClick:()=>setCount(count+1)},'leaf'+count);}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let clean_id = initial.node["children"][0]["__nativeId"]
            .as_str()
            .unwrap()
            .to_owned();
        let event = initial.events[&0].clone();
        assert!(
            host.dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event.clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |_, _, _| Err::<(), _>("native rejected patch".into()),
            )
            .is_err()
        );
        // Rejection rolls every package runtime and generation back, leaving
        // the previously accepted native handler authoritative.
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event, Value::Null)],
                &initial.events,
                &initial.node,
                |patch, events, _| {
                    assert_eq!(patch.operations.len(), 1);
                    let NativePatchOperation::SetPrimitive {
                        target,
                        property,
                        value,
                    } = &patch.operations[0]
                    else {
                        panic!("package-local leaf update must stay granular")
                    };
                    assert!(target.contains("export:shell.taskbar"));
                    assert_eq!(property, "children");
                    assert_eq!(value[0], "leaf1");
                    assert_eq!(events.len(), 1);
                    assert_ne!(target, &clean_id);
                    Ok(())
                },
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { patch, events, .. } = outcome else {
            panic!("dirty nested mount must emit a patch")
        };
        assert_eq!(
            patch.counters.nodes_visited, 1,
            "a derived-shell leaf update must not visit the clean base sibling"
        );
        assert_eq!(patch.counters.nodes_mutated, 1);
        assert_eq!(patch.counters.local_materializations, 0);
        assert_eq!(patch.counters.expansion_nodes, 0);
        assert_eq!(patch.counters.tree_bytes, 0);
        assert_eq!(events.len(), 1);
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn failure_matrix_derived_shell_native_rejection_isolated_and_recovers() {
        let mut base = package(
            "base",
            "export function Shell(){return h(Column,null,h(Text,{key:'clean'},'clean sibling'),h(nickel.component('shell.taskbar')));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "function Leaf(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>{setCount(count+1);nickel.windows.activate('provisional')}},'leaf'+count)}\nexport function Taskbar(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:reset},'taskbar failed:'+error.message)},h(Leaf));}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let accepted_source = initial.node.clone();
        let accepted_events = initial.events.clone();
        let event = accepted_events[&0].clone();
        let mut attempts = 0;
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event, Value::Null)],
                &accepted_events,
                &accepted_source,
                |patch, events, _| {
                    attempts += 1;
                    if attempts == 1 {
                        assert!(patch.operations.iter().any(|operation| matches!(
                            operation,
                            NativePatchOperation::SetPrimitive { value, .. }
                                if value.to_string().contains("leaf1")
                        )));
                        return Err("native widget contract rejected leaf".into());
                    }
                    assert_eq!(events.len(), 1, "fallback owns one reset handler");
                    assert!(patch.operations.iter().any(|operation| {
                        serde_json::to_string(operation)
                            .unwrap()
                            .contains("taskbar failed:native widget contract rejected leaf")
                    }));
                    assert!(!patch.operations.iter().any(|operation| {
                        serde_json::to_string(operation)
                            .unwrap()
                            .contains("clean sibling")
                    }));
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(attempts, 2);
        let ScheduledExpandedBatch::Patched { events, .. } = outcome else {
            panic!("boundary fallback must reconcile as a typed patch")
        };
        host.finish_transaction(true).unwrap();
        assert!(
            host.take_effects().is_empty(),
            "rejected effects stay discarded"
        );
        assert!(host.dispatch(&accepted_events[&0], &Value::Null).is_err());
        let fallback = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(fallback.node.to_string().contains("taskbar failed:"));
        assert!(fallback.node.to_string().contains("clean sibling"));
        assert_eq!(fallback.events[&0].action, events[&0].action);
        let recovered =
            accept_scheduled_event(&mut host, &root, fallback.events[&0].clone(), &fallback);
        assert!(recovered.node.to_string().contains("leaf0"));
        assert!(recovered.node.to_string().contains("clean sibling"));
        assert_eq!(host.packages.len(), 2);
        let diagnostics = host
            .packages
            .values_mut()
            .find(|package| package.manifest.id == "child")
            .unwrap()
            .runtime
            .borrow_mut()
            .boundary_diagnostics()
            .unwrap();
        assert_eq!(diagnostics.last().unwrap()["phase"], "native-validation");
    }

    #[test]
    fn scheduled_root_local_update_uses_a_typed_patch_without_a_cold_render() {
        let mut host = make_host();
        let root = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(initial.events[&0].clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.counters),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch, validated, ..
        } = outcome
        else {
            panic!("root-local change must use typed patch transport")
        };
        assert_eq!(patch.operations.len(), 1);
        assert!(matches!(
            patch.operations[0],
            NativePatchOperation::SetPrimitive { .. }
        ));
        assert_eq!(validated.nodes_mutated, 1);
        assert_eq!(validated.local_materializations, 0);
        assert_eq!(validated.expansion_nodes, 0);
        assert_eq!(validated.tree_bytes, 0);
        host.finish_transaction(true).unwrap();
        let owner = host.mounts[&root.id].reference.owner.clone();
        let diagnostics = host.packages[&owner]
            .runtime
            .borrow_mut()
            .runtime_diagnostics()
            .unwrap();
        let profile = diagnostics["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["surface"] == surface(root.id))
            .unwrap();
        assert_eq!(profile["typedPatchApplyAttempts"], 1);
        assert_eq!(profile["typedPatchApplyRejections"], 0);
        assert!(profile["typedPatchApplyMicros"].is_u64());
    }

    #[test]
    fn structural_parent_replacement_preserves_surviving_component_state() {
        let mut base = package(
            "base",
            r#"
            export function Shell(){const [changed,setChanged]=useState(false);
                return h(Div,{onChange:changed?()=>{}:undefined},
                    h(Button,{id:'toggle',onClick:()=>setChanged(!changed)},'Toggle'),
                    h(nickel.component('shell.taskbar'),{key:'survivor'}));}
            export function Taskbar(){const [count,setCount]=useState(0);
                return h(Button,{id:'counter',onClick:()=>setCount(count+1)},'Count '+count);}
            export function QuickSettings(){} export default Shell;
        "#,
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        fn action(node: &Value, id: &str) -> Option<u64> {
            if node["id"] == id {
                return node["action"].as_u64();
            }
            node["children"]
                .as_array()?
                .iter()
                .find_map(|child| action(child, id))
        }
        let initial = host.render_expanded(&root, &json!({}), |_| Ok(())).unwrap();
        let counter = initial.events[&action(&initial.node, "counter").unwrap()].clone();
        let edited = host
            .dispatch_expanded(&root, &counter, &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(edited.node.to_string().contains("Count 1"));
        let old_mount = edited.events[&action(&edited.node, "counter").unwrap()].mount;
        let toggle = edited.events[&action(&edited.node, "toggle").unwrap()].clone();
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(toggle, Value::Null)],
                &edited.events,
                &edited.node,
                |patch, _, _| Ok(patch.clone()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { patch, events, .. } = outcome else {
            panic!("expected structural patch")
        };
        assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::ReplaceSubtree { .. }))
        );
        assert!(
            serde_json::to_string(&patch).unwrap().contains("Count 1"),
            "parent replacement reset surviving child state"
        );
        assert!(
            events.values().any(|event| event.mount == old_mount),
            "surviving child mount retired"
        );
        let counter_token = action(&edited.node, "counter").unwrap();
        let retained_counter = events
            .get(&counter_token)
            .expect("surviving native action lost its stable expanded token")
            .clone();
        assert_eq!(retained_counter.mount, old_mount);
        host.finish_transaction(true).unwrap();
        let next = host
            .dispatch_expanded(&root, &retained_counter, &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(next.node.to_string().contains("Count 2"));
    }

    #[test]
    fn native_path_retirement_is_bounded_and_does_not_match_key_prefix_siblings() {
        let mut authority = MountPatchAuthority::default();
        authority
            .record_paths(
                &json!({"__nativeId":"root/@row-10"}),
                "root/children/sibling",
                0,
            )
            .unwrap();
        for iteration in 0..1000 {
            let id = format!("root/@row-1/child-{iteration}");
            authority.record_paths(&json!({"__nativeId":"root/@row-1","children":[{"__nativeId":id,"key":"child"}]}),"root/children/row",0).unwrap();
            assert_eq!(authority.paths.len(), 3);
            assert!(native_slot_in_subtree("root/@row-1:action", "root/@row-1"));
            assert!(native_slot_in_subtree(
                &format!("{id}:action"),
                "root/@row-1"
            ));
            assert!(!native_slot_in_subtree(
                "root/@row-10:action",
                "root/@row-1"
            ));
            authority.forget_paths("root/@row-1");
            assert_eq!(authority.paths.len(), 1);
            assert!(authority.paths.contains_key("root/@row-10"));
        }
    }

    #[test]
    fn callback_driven_complete_owner_render_retires_removed_native_actions() {
        let mut base = package(
            "base",
            r#"
            export function Shell(){const [show,setShow]=useState(true);
                return h(Column,{},h(nickel.component('shell.taskbar'),{select:()=>setShow(false)}),
                    show?h(Button,{id:'obsolete',onClick:()=>{}},'Old'):h(Text,{},'Gone'));}
            export function Taskbar(props){return h(Button,{id:'navigate',onClick:()=>props.select()},'Navigate');}
            export function QuickSettings(){} export default Shell;
        "#,
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base)]),
            "base",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host.render_expanded(&root, &json!({}), |_| Ok(())).unwrap();
        assert_eq!(initial.events.len(), 2);
        let obsolete = initial
            .events
            .iter()
            .find(|(_, handle)| handle.mount == root.id)
            .unwrap()
            .0;
        let navigation = initial
            .events
            .iter()
            .find(|(_, handle)| handle.mount != root.id)
            .unwrap();
        let token = *navigation.0;
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(navigation.1.clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |_, _, _| Ok(()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { events, .. } = outcome else {
            panic!("missing owner update")
        };
        assert_eq!(
            events.len(),
            1,
            "removed control retained executable authority"
        );
        assert!(!events.contains_key(obsolete));
        assert!(
            events.contains_key(&token),
            "surviving navigation token changed"
        );
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn structural_package_component_insert_expands_transactionally_and_matches_cold_oracle() {
        fn structural_host() -> ShellCompositionRuntime {
            let mut base = package(
                "base",
                "export function Shell(){const [step,setStep]=useState(0);return h(Div,{onChange:step?()=>setStep(step+1):undefined},h(Button,{key:'toggle',onClick:()=>setStep(step+1)},'show'),...(step===0?[]:[step===1?h(nickel.component('shell.taskbar'),{key:'nested',onSelected:()=>setStep(2)}):h(Text,{key:'nested'},'replacement')]));}\nexport function Taskbar(){}\nexport function QuickSettings({onSelected}){return h(Button,{key:'owned',onClick:onSelected},'nested-child');}\nexport default Shell;",
                None,
            );
            base.manifest
                .composition
                .as_mut()
                .unwrap()
                .exports
                .insert("shell".into(), "./main.js#Shell".into());
            let child = package(
                "child",
                "export function Taskbar({onSelected}){return h(Column,null,h(nickel.component('shell.quickSettings'),{onSelected}));}\nexport default Taskbar;",
                Some("base"),
            );
            ShellCompositionRuntime::new(
                &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
                "child",
                &BTreeMap::new(),
            )
            .unwrap()
        }

        let mut host = structural_host();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let event = initial.events[&0].clone();
        assert!(
            host.dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event.clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |_, _, _| Err::<(), _>("reject structural splice".into()),
            )
            .is_err()
        );
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event, Value::Null)],
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.clone()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch,
            validated,
            events,
            ..
        } = outcome
        else {
            panic!()
        };
        let encoded = serde_json::to_string(&patch).unwrap();
        assert!(encoded.contains("nested-child"));
        assert!(!encoded.contains("__packageComponent"));
        assert_eq!(validated, patch);
        fn assert_live_actions(value: &Value, events: &BTreeMap<u64, ComponentEventHandle>) {
            match value {
                Value::Array(values) => {
                    for value in values {
                        assert_live_actions(value, events);
                    }
                }
                Value::Object(object) => {
                    for (key, value) in object {
                        if key == "__handlerSlots" {
                            continue;
                        }
                        if is_action(key) && !value.is_null() {
                            assert!(
                                events.contains_key(&value.as_u64().unwrap()),
                                "replacement lost its {key} callback: {value}"
                            );
                        } else {
                            assert_live_actions(value, events);
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::ReplaceSubtree { .. }))
        );
        for operation in &patch.operations {
            if let NativePatchOperation::ReplaceSubtree { node, .. }
            | NativePatchOperation::InsertChild { node, .. } = operation
            {
                assert_live_actions(node, &events);
            }
        }

        let mut oracle = structural_host();
        let oracle_root = oracle.mount(&oracle.component("shell").unwrap()).unwrap();
        let oracle_initial = oracle
            .render_expanded(&oracle_root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let oracle_next = oracle
            .dispatch_expanded(
                &oracle_root,
                &oracle_initial.events[&0],
                &Value::Null,
                |_| Ok(()),
            )
            .unwrap();
        assert!(oracle_next.node.to_string().contains("nested-child"));
        host.finish_transaction(true).unwrap();

        let accepted = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let toggle = accepted.node["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["key"] == "toggle")
            .unwrap()["action"]
            .as_u64()
            .unwrap();
        let replacement_event = accepted.events[&toggle].clone();
        let mounts_before_replace = host.mounts.len();
        assert!(
            host.dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(replacement_event.clone(), Value::Null)],
                &accepted.events,
                &accepted.node,
                |_, _, _| Err::<(), _>("reject structural replacement".into()),
            )
            .is_err()
        );
        assert_eq!(host.mounts.len(), mounts_before_replace);
        let replaced = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(replacement_event, Value::Null)],
                &accepted.events,
                &accepted.node,
                |patch, _, _| Ok(patch.clone()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched { patch, .. } = replaced else {
            panic!()
        };
        assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::ReplaceSubtree { node, .. } if node.to_string().contains("replacement")))
        );
        assert!(host.mounts.len() < mounts_before_replace);
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn native_diff_keeps_keyed_and_handler_only_updates_typed() {
        let before = serde_json::json!({
            "kind":"Column","__nativeId":"root","children":[
                {"kind":"Button","key":"a","__nativeId":"a","__handlerSlots":{"action":"a::action"},"action":0,"children":["a"]},
                {"kind":"Text","key":"b","__nativeId":"b","children":["b"]}
            ]
        });
        let after = serde_json::json!({
            "kind":"Column","__nativeId":"root","children":[
                {"kind":"Text","key":"b","__nativeId":"b","children":["b"]},
                {"kind":"Button","key":"a","__nativeId":"a","__handlerSlots":{"action":"a::action"},"action":9,"children":["a"]},
                {"kind":"Text","key":"c","__nativeId":"c","children":["c"]}
            ]
        });
        let mut operations = Vec::new();
        let mut visited = 0;
        diff_native_subtree(&before, &after, &mut operations, &mut visited).unwrap();
        assert!(operations.iter().any(|operation| matches!(operation, NativePatchOperation::MoveChild { child_id, .. } if child_id == "b")));
        assert!(operations.iter().any(|operation| matches!(operation, NativePatchOperation::InsertChild { child_id, .. } if child_id == "c")));
        assert!(operations.iter().any(|operation| matches!(operation, NativePatchOperation::ReplaceHandlerSlot { slot, action } if slot == "a::action" && *action == 9)));
        assert!(!operations.iter().any(|operation| matches!(operation, NativePatchOperation::ReplaceSubtree { target, .. } if target == "root")));
        assert!(visited >= 2);
    }

    #[test]
    fn native_diff_replaces_only_the_node_when_its_handler_slots_change_shape() {
        let before = serde_json::json!({
            "kind":"window","__nativeId":"root","__handlerSlots":{"escapeAction":"root:escapeAction"},"escapeAction":0,"children":[]
        });
        let after = serde_json::json!({
            "kind":"window","__nativeId":"root","__handlerSlots":{"escapeAction":"root:escapeAction","submitAction":"root:submitAction"},"escapeAction":0,"submitAction":1,"children":[]
        });
        let mut operations = Vec::new();
        let mut visited = 0;
        diff_native_subtree(&before, &after, &mut operations, &mut visited).unwrap();
        assert_eq!(operations.len(), 1);
        assert!(matches!(
            &operations[0],
            NativePatchOperation::ReplaceSubtree { target, node }
                if target == "root" && node["submitAction"] == 1
        ));
    }

    #[test]
    fn native_diff_replaces_stable_parent_when_positional_child_identity_changes() {
        let before = serde_json::json!({
            "kind":"Column","__nativeId":"root","children":[
                {"kind":"Text","__nativeId":"root/old","children":["old"]}
            ]
        });
        let after = serde_json::json!({
            "kind":"Column","__nativeId":"root","children":[
                {"kind":"Button","__nativeId":"root/new","children":["new"]}
            ]
        });
        let mut operations = Vec::new();
        let mut visited = 0;
        diff_native_subtree(&before, &after, &mut operations, &mut visited).unwrap();
        assert_eq!(
            operations,
            vec![NativePatchOperation::ReplaceSubtree {
                target: "root".into(),
                node: after,
            }]
        );
        assert_eq!(visited, 1);
    }

    #[test]
    fn atomic_patch_suppresses_work_below_replaced_ancestor() {
        let replacement = serde_json::json!({
            "kind":"Column","__nativeId":"root/owner","children":[
                {"kind":"Text","key":"fresh","__nativeId":"root/owner/@fresh","children":["cold"]}
            ]
        });
        let operations = normalize_atomic_operations(vec![
            NativePatchOperation::ReplaceSubtree {
                target: "root/owner".into(),
                node: replacement.clone(),
            },
            NativePatchOperation::SetPrimitive {
                target: "root/owner/@retired".into(),
                property: "children".into(),
                value: serde_json::json!(["stale"]),
            },
        ]);
        assert_eq!(
            operations,
            vec![NativePatchOperation::ReplaceSubtree {
                target: "root/owner".into(),
                node: replacement,
            }]
        );
    }

    #[test]
    fn atomic_patch_replacement_supersedes_work_on_the_same_target() {
        let replacement = serde_json::json!({
            "kind":"Column","__nativeId":"root/owner","className":"fresh","children":[]
        });
        let operations = normalize_atomic_operations(vec![
            NativePatchOperation::SetPrimitive {
                target: "root/owner".into(),
                property: "className".into(),
                value: serde_json::json!("stale"),
            },
            NativePatchOperation::ReplaceSubtree {
                target: "root/owner".into(),
                node: replacement.clone(),
            },
            NativePatchOperation::SetPrimitive {
                target: "root/owner".into(),
                property: "className".into(),
                value: serde_json::json!("also-stale"),
            },
        ]);
        assert_eq!(
            operations,
            vec![NativePatchOperation::ReplaceSubtree {
                target: "root/owner".into(),
                node: replacement,
            }]
        );
    }

    #[test]
    fn two_dirty_sibling_packages_emit_one_ordered_atomic_patch_envelope() {
        let mut base = package(
            "base",
            "export function Shell(){return h(Column,null,h(nickel.component('shell.taskbar'),{key:'a'}),h(nickel.component('shell.taskbar'),{key:'b'}));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},String(count));}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let events = initial
            .events
            .values()
            .cloned()
            .map(|event| (event, Value::Null))
            .collect::<Vec<_>>();
        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &events,
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.counters),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch, validated, ..
        } = outcome
        else {
            panic!("two dirty siblings must emit patches")
        };
        assert_eq!(patch.operations.len(), 2);
        assert_eq!(validated.nodes_mutated, 2);
        let targets = patch
            .operations
            .iter()
            .map(|operation| match operation {
                NativePatchOperation::ReplaceSubtree { target, .. }
                | NativePatchOperation::SetPrimitive { target, .. } => target,
                NativePatchOperation::ReplaceHandlerSlot { slot, .. } => slot,
                NativePatchOperation::InsertChild { parent, .. }
                | NativePatchOperation::RemoveChild { parent, .. }
                | NativePatchOperation::MoveChild { parent, .. } => parent,
            })
            .collect::<Vec<_>>();
        assert!(targets[0] < targets[1]);
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn passive_child_effect_reconciles_as_a_typed_boundary_patch() {
        let mut base = package(
            "base",
            "export function Shell(){return h(Column,null,h(Text,null,'clean'),h(nickel.component('shell.taskbar')));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(){const [count,setCount]=useState(0);useEffect(()=>setCount(1),[]);return h(Text,null,'effect'+count);}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let outcome = host
            .reconcile_expanded_pending_validated(
                &root,
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.counters),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch, validated, ..
        } = outcome
        else {
            panic!("passive child effect must patch its boundary")
        };
        assert_eq!(patch.operations.len(), 1);
        assert_eq!(validated.nodes_mutated, 1);
        assert!(format!("{:?}", patch.operations).contains("effect1"));
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn callback_parent_and_child_collapse_to_one_retained_root_patch() {
        let mut host = transactional_cross_owner_host();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let clean_owner_id = initial.node["children"][2]["__nativeId"]
            .as_str()
            .unwrap()
            .to_owned();
        let event = initial.events[&0].clone();
        assert!(
            host.dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event.clone(), Value::Null)],
                &initial.events,
                &initial.node,
                |_, _, _| Err::<(), _>("reject combined patch".into()),
            )
            .is_err()
        );

        let outcome = host
            .dispatch_expanded_batch_scheduled_pending_validated(
                &root,
                &[(event, Value::Null)],
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.clone()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch, validated, ..
        } = outcome
        else {
            panic!("callback fan-out must produce an atomic patch")
        };
        assert_eq!(patch.operations.len(), 2);
        let serialized = format!("{:?}", patch.operations);
        assert!(!serialized.contains(&clean_owner_id));
        assert!(serialized.contains("base1:1"));
        assert!(serialized.contains("child1"));
        assert_eq!(validated, patch);

        let mut oracle = transactional_cross_owner_host();
        let oracle_root = oracle.mount(&oracle.component("shell").unwrap()).unwrap();
        let oracle_initial = oracle
            .render_expanded(&oracle_root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let oracle_next = oracle
            .dispatch_expanded(
                &oracle_root,
                &oracle_initial.events[&0],
                &Value::Null,
                |_| Ok(()),
            )
            .unwrap();
        assert!(oracle_next.node.to_string().contains("base1:1"));
        assert!(oracle_next.node.to_string().contains("child1"));
        host.finish_transaction(true).unwrap();
    }

    #[test]
    fn context_providers_stop_at_cross_package_component_boundaries() {
        let mut base = package(
            "base",
            "const Context=createContext('base-default');\nexport function Shell(){return h(Context.Provider,{value:'base-provider'},h(nickel.component('shell.taskbar')));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "const Context=createContext('child-default');\nexport function Taskbar(){return h(Text,null,useContext(Context));}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let mount = host.mount(&host.component("shell").unwrap()).unwrap();
        let tree = host
            .render_expanded(&mount, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("child-default"));
        assert!(!tree.node.to_string().contains("base-provider"));
    }

    #[test]
    fn context_objects_cannot_be_transferred_as_cross_package_props() {
        let mut base = package(
            "base",
            "const Context=createContext('base-default');\nexport function Shell(){return h(nickel.component('shell.taskbar'),{context:Context});}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(){return h(Text,null,'child');}\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let mount = host.mount(&host.component("shell").unwrap()).unwrap();
        let error = match host.render_expanded(&mount, &serde_json::json!({}), |_| Ok(())) {
            Ok(_) => panic!("context transport unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error.contains("contexts cannot cross package ownership boundaries"));
    }

    #[test]
    fn callable_props_route_to_the_original_owner_and_preserve_arguments() {
        let mut base = package(
            "base",
            "export function Shell() { const [value,setValue]=useState('initial'); return h(Column,null,h(Text,null,value),h(nickel.component('shell.taskbar'), {onChange:(value)=>{setValue(value);nickel.windows.activate(value);}})); }\nexport function Taskbar() {}\nexport function QuickSettings() {}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(props) { return h(Button,{onClick:()=>{if (props.onChange) {props.onChange('first');nickel.windows.activate('middle');props.onChange('second');} else {nickel.request({type:'__compositionCallback',callback:props.token,args:['forged']});}}},'foreign control'); }\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let reference = host.component("shell").unwrap();
        let mount = host.mount(&reference).unwrap();
        let tree = host
            .render_expanded(&mount, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("initial"));
        assert_eq!(tree.events[&0].owner().id, "child");
        let tree = host
            .dispatch_expanded(&mount, &tree.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("second"));
        let effects = host.take_effects();
        assert_eq!(effects.len(), 3);
        assert_eq!(
            effects
                .iter()
                .map(|effect| effect.owner().id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "child", "base"]
        );
        assert_eq!(effects[0].value()["id"], "first");
        assert_eq!(effects[1].value()["id"], "middle");
        assert_eq!(effects[2].value()["id"], "second");
        let token = *host.callbacks.keys().next().unwrap();
        let foreign = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let foreign_tree = host
            .render(&foreign, &serde_json::json!({"token":token}))
            .unwrap();
        assert!(
            host.dispatch(&foreign_tree.events[&0], &Value::Null)
                .err()
                .unwrap()
                .contains("foreign component callback")
        );
        assert!(host.take_effects().is_empty());
        host.unmount(&mount).unwrap();
        assert!(host.callbacks.is_empty());
    }

    #[test]
    fn retained_child_callback_tracks_the_source_mount_generation() {
        let mut base = package(
            "base",
            "export function Shell() { const [tick,setTick]=useState(0); const [value,setValue]=useState('initial'); return h(Column,null,h(Text,null,value+':'+tick),h(Button,{id:'advance',onClick:()=>setTick(tick+1)},'advance'),h(nickel.component('shell.taskbar'),{onChange:value=>{setValue(value);nickel.windows.activate(value);}})); }\nexport function Taskbar() {}\nexport function QuickSettings() {}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(props) { return h(Button,{id:'child',onClick:()=>props.onChange('updated')},'child'); }\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let parent_event = tree
            .events
            .values()
            .find(|event| event.owner().id == "base")
            .unwrap()
            .clone();
        let tree = host
            .dispatch_expanded(&root, &parent_event, &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("initial:1"));

        let child_event = tree
            .events
            .values()
            .find(|event| event.owner().id == "child")
            .unwrap()
            .clone();
        let tree = host
            .dispatch_expanded(&root, &child_event, &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("updated:1"));
        let effects = host.take_effects();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].owner().id, "base");
        assert_eq!(effects[0].value()["id"], "updated");
    }

    #[test]
    fn snapshot_only_callback_update_reaches_retained_native_handler() {
        let mut base = package(
            "base",
            "export function Shell() { const windows=useWindows(); const active=windows.find(window=>window.active)?.id??'none'; return h(nickel.component('shell.taskbar'),{onActivate:()=>nickel.windows.activate(active)}); }\nexport function Taskbar() {}\nexport function QuickSettings() {}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let owner = PackageIdentity {
            id: "base".into(),
            version: base.manifest.version.as_deref().unwrap().parse().unwrap(),
        };
        let child = package(
            "child",
            "export function Taskbar(props) { return h(Button,{id:'child',onClick:props.onActivate},'child'); }\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::from([(
                owner.clone(),
                serde_json::json!({"windows":[{"id":"first","active":true}]}),
            )]),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let initial = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        let outcome = host
            .update_snapshot_and_reconcile_expanded_pending_validated(
                &owner,
                &serde_json::json!({"windows":[{"id":"second","active":true}]}),
                &root,
                &initial.events,
                &initial.node,
                |patch, _, _| Ok(patch.operations.len()),
            )
            .unwrap();
        let ScheduledExpandedBatch::Patched {
            patch,
            events,
            validated,
            ..
        } = outcome
        else {
            panic!("snapshot subscriber must reconcile")
        };
        assert_eq!(validated, 0);
        assert!(patch.operations.is_empty());
        host.finish_transaction(true).unwrap();

        let child_token = initial.node["action"].as_u64().unwrap();
        let child_event = events[&child_token].clone();
        host.dispatch_expanded_batch_scheduled_pending_validated(
            &root,
            &[(child_event, Value::Null)],
            &events,
            &initial.node,
            |_, _, _| Ok(()),
        )
        .unwrap();
        host.finish_transaction(true).unwrap();
        let effects = host.take_effects();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].owner().id, "base");
        assert_eq!(effects[0].value()["id"], "second");
    }

    #[test]
    fn native_child_props_keep_the_authors_callbacks_and_hook_state() {
        let mut base = package(
            "base",
            "export function Shell() { const [count,setCount]=useState(0); const button=id=>h(Button,{id,onClick:()=>{setCount(count+1);nickel.windows.activate(id);}},id+count); return h(nickel.component('shell.taskbar'),{header:button('header')},button('child')); }\nexport function Taskbar() {}\nexport function QuickSettings() {}\nexport default Shell;",
            None,
        );
        base.manifest
            .composition
            .as_mut()
            .unwrap()
            .exports
            .insert("shell".into(), "./main.js#Shell".into());
        let child = package(
            "child",
            "export function Taskbar(props) { return h(Column,null,props.header,...props.children); }\nexport default Taskbar;",
            Some("base"),
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("base".into(), base), ("child".into(), child)]),
            "child",
            &BTreeMap::new(),
        )
        .unwrap();
        let root = host.mount(&host.component("shell").unwrap()).unwrap();
        let tree = host
            .render_expanded(&root, &serde_json::json!({}), |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("header0"));
        assert!(tree.node.to_string().contains("child0"));
        assert_eq!(tree.events.len(), 2);
        assert!(tree.events.values().all(|event| event.owner().id == "base"));
        let tree = host
            .dispatch_expanded(&root, &tree.events[&0], &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(tree.node.to_string().contains("header1"));
        assert!(tree.node.to_string().contains("child1"));
        assert_eq!(host.take_effects()[0].owner().id, "base");
        host.unmount(&root).unwrap();
        assert!(host.children.is_empty());
    }

    #[test]
    fn replacement_reuse_and_contribution_run_in_isolated_owner_contexts() {
        let mut host = make_host();
        let replacement = host.component("shell.taskbar").unwrap();
        let inherited = host.component("shell.quickSettings").unwrap();
        let contribution = host.contributions("settings.pages").remove(0);
        assert_eq!(replacement.owner().id, "child");
        assert_eq!(inherited.owner().id, "base");
        assert_eq!(contribution.owner().id, "child");
        let child_mount = host.mount(&replacement).unwrap();
        let base_mount = host.mount(&inherited).unwrap();
        let child_tree = host.render(&child_mount, &serde_json::json!({})).unwrap();
        let base_tree = host.render(&base_mount, &serde_json::json!({})).unwrap();
        assert!(child_tree.node.to_string().contains("child"));
        assert!(!child_tree.node.to_string().contains("base"));
        assert!(base_tree.node.to_string().contains("base"));
        host.dispatch(&child_tree.events[&0], &Value::Null).unwrap();
        host.dispatch(&base_tree.events[&0], &Value::Null).unwrap();
        let effects = host.take_effects();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].owner().id, "child");
        assert_eq!(effects[0].value()["owner"], "base"); // Untrusted payload never establishes origin.
        assert_eq!(effects[1].owner().id, "base");
        let mut grants = CompositionGrants::default();
        grants.grant(
            replacement.owner().clone(),
            vec![PluginCapability::WindowsFocus],
        );
        assert!(grants.permits(effects[0].owner(), PluginCapability::WindowsFocus));
        assert!(!grants.permits(effects[1].owner(), PluginCapability::WindowsFocus));
        let contribution_mount = host.mount(&contribution).unwrap();
        let tree = host
            .render(&contribution_mount, &serde_json::json!({}))
            .unwrap();
        assert_eq!(tree.events[&0].owner().id, "child");
    }

    #[test]
    fn events_reject_stale_foreign_and_retired_handles() {
        let mut host = make_host();
        let reference = host.component("shell.taskbar").unwrap();
        let mount = host.mount(&reference).unwrap();
        let first = host.render(&mount, &serde_json::json!({})).unwrap();
        let old = first.events[&0].clone();
        let latest = host.dispatch(&old, &Value::Null).unwrap();
        assert!(host.dispatch(&old, &Value::Null).is_err());
        let mut other = make_host();
        assert!(other.mount(&reference).is_err());
        assert!(other.dispatch(&latest.events[&0], &Value::Null).is_err());
        host.retire(reference.owner());
        assert!(host.dispatch(&latest.events[&0], &Value::Null).is_err());
        assert!(host.mount(&reference).is_err());
        assert!(host.take_effects().is_empty());
    }

    #[test]
    fn rejected_native_tree_rolls_back_owner_event_and_effects() {
        let mut host = make_host();
        let reference = host.component("shell.taskbar").unwrap();
        let mount = host.mount(&reference).unwrap();
        let tree = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(
            host.dispatch_validated(&tree.events[&0], &Value::Null, |_| Err(
                "native tree rejected".into()
            ))
            .is_err()
        );
        assert!(host.take_effects().is_empty());
        let accepted = host.dispatch(&tree.events[&0], &Value::Null).unwrap();
        assert!(accepted.node.to_string().contains("child1"));
        let effect = host.take_effects().remove(0);
        assert!(host.validate_effect(&effect).is_ok());
        let other = make_host();
        assert!(other.validate_effect(&effect).is_err());
        host.retire(reference.owner());
        assert!(host.validate_effect(&effect).is_err());
    }

    #[test]
    fn loaded_component_mount_cycles_do_not_compile_call_wrappers() {
        let mut host = make_host();
        let reference = host.component("shell.taskbar").unwrap();
        let runtime = host.shared_owner_runtime(reference.owner()).unwrap();
        let compilations = runtime
            .borrow_mut()
            .engine
            .diagnostics()
            .script_compilations;
        for _ in 0..32 {
            let mount = host.mount(&reference).unwrap();
            let initial = host.render(&mount, &serde_json::json!({})).unwrap();
            assert!(initial.node.to_string().contains("child0"));
            let changed = host.dispatch(&initial.events[&0], &Value::Null).unwrap();
            assert!(changed.node.to_string().contains("child1"));
            host.consume_mount_reconciliation(&mount).unwrap();
            host.unmount(&mount).unwrap();
            assert!(host.dispatch(&changed.events[&0], &Value::Null).is_err());
            assert_eq!(
                runtime
                    .borrow_mut()
                    .engine
                    .diagnostics()
                    .script_compilations,
                compilations
            );
        }
    }

    #[test]
    fn two_mounts_share_package_globals_and_keep_separate_hooks() {
        let mut host = make_host();
        let reference = host.component("shell.taskbar").unwrap();
        let a = host.mount(&reference).unwrap();
        let b = host.mount(&reference).unwrap();
        let a_tree = host.render(&a, &serde_json::json!({})).unwrap();
        host.render(&b, &serde_json::json!({})).unwrap();
        let updated_a = host.dispatch(&a_tree.events[&0], &Value::Null).unwrap();
        assert!(updated_a.node.to_string().contains("child1"));
        let updated_b = host.render(&b, &serde_json::json!({})).unwrap();
        assert!(updated_b.node.to_string().contains("child0"));
        host.unmount(&a).unwrap();
        assert!(host.dispatch(&a_tree.events[&0], &Value::Null).is_err());
        assert!(host.dispatch(&updated_b.events[&0], &Value::Null).is_ok());
    }

    #[test]
    fn settings_sized_tree_counts_components_instead_of_every_json_value() {
        let owner = make_host().resolution().active.clone();
        let rows = (0..512)
            .map(|index| {
                serde_json::json!({
                    "kind": "button",
                    "id": format!("setting-{index}"),
                    "title": format!("Setting {index}"),
                    "description": "A production-shaped Settings row with several scalar properties",
                    "className": "settings-row",
                    "disabled": false,
                    "selected": false,
                    "width": 480,
                    "height": 40,
                    "action": index,
                    "__handlerSlots": {"action": format!("root/@setting-{index}:action")},
                    "children": [format!("Setting {index}")]
                })
            })
            .collect::<Vec<_>>();
        let mut tree = serde_json::json!({
            "kind": "column",
            "className": "settings-page",
            "children": rows
        });
        let mut events = BTreeMap::new();

        rewrite_events(
            &mut tree,
            1,
            2,
            3,
            &owner,
            &mut events,
            &mut TreeBudget::default(),
            0,
        )
        .unwrap();

        assert_eq!(events.len(), 512);
    }

    #[test]
    fn component_depth_and_handler_budgets_remain_independent() {
        let owner = make_host().resolution().active.clone();
        let mut too_many_handlers = Value::Array(
            (0..=MAX_HANDLERS)
                .enumerate()
                .map(|(index, _)| {
                    serde_json::json!({
                        "action": 0,
                        "__handlerSlots": {"action": format!("root/#{index}:action")}
                    })
                })
                .collect(),
        );
        let error = rewrite_events(
            &mut too_many_handlers,
            1,
            2,
            3,
            &owner,
            &mut BTreeMap::new(),
            &mut TreeBudget::default(),
            0,
        )
        .unwrap_err();
        assert!(error.contains("handler limit"));

        let mut too_deep = Value::Null;
        for _ in 0..=MAX_TREE_DEPTH {
            too_deep = Value::Array(vec![too_deep]);
        }
        let error = rewrite_events(
            &mut too_deep,
            1,
            2,
            3,
            &owner,
            &mut BTreeMap::new(),
            &mut TreeBudget::default(),
            0,
        )
        .unwrap_err();
        assert!(error.contains("depth limit"));
    }

    #[test]
    fn rejected_settings_surface_does_not_retire_the_shell_owner() {
        let mut host = make_host();
        let settings = host.component("shell.quickSettings").unwrap();
        let settings_mount = host.mount(&settings).unwrap();
        let error = host
            .render_expanded(&settings_mount, &serde_json::json!({}), |_| {
                Err("local Settings presentation rejected".into())
            })
            .err()
            .expect("rejected Settings presentation must return an error");
        assert!(error.contains("local Settings presentation rejected"));

        let shell = host.component("shell.taskbar").unwrap();
        let shell_mount = host.mount(&shell).unwrap();
        let (rendered, admitted_generation) = host
            .render_expanded_validated_with_generation(
                &shell_mount,
                &serde_json::json!({}),
                |_, generation| Ok(generation),
            )
            .unwrap();
        assert_eq!(rendered.generation(), admitted_generation);
        assert!(rendered.node.to_string().contains("child"));
    }

    #[test]
    fn scheduled_noop_keeps_mount_generation_and_skips_component_render() {
        let package = package(
            "scheduler",
            "globalThis.renders=0;\nexport function Taskbar(){renders++;const [value,setValue]=useState(0);return h(Button,{onClick:()=>setValue(current=>current)},String(value));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("scheduler".into(), package)]),
            "scheduler",
            &BTreeMap::new(),
        )
        .unwrap();
        let reference = host.component("shell.taskbar").unwrap();
        let mount = host.mount(&reference).unwrap();
        let tree = host.render(&mount, &serde_json::json!({})).unwrap();
        let event = tree.events[&0].clone();
        assert_eq!(tree.generation(), event.generation);

        let outcome = host.dispatch_scheduled(&event, &Value::Null).unwrap();
        assert!(outcome.rendered.is_none());
        assert!(!outcome.reconciliation_requested);
        assert_eq!(
            host.shared_owner_runtime(reference.owner())
                .unwrap()
                .borrow_mut()
                .eval_json::<u64>("JSON.stringify(renders)")
                .unwrap(),
            1
        );
        assert!(host.dispatch_scheduled(&event, &Value::Null).is_ok());
    }

    #[test]
    fn composition_mount_publishes_host_owned_versioned_surface_store() {
        let package = package(
            "surface-shell",
            "globalThis.surfaceObservation=null;\nexport function Taskbar(){const surface=useSurface();surfaceObservation=surface;return h(Text,null,surface.id+':'+surface.logicalSize.width);}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "surface-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("surface-shell".into(), package)]),
            "surface-shell",
            &BTreeMap::from([(
                owner.clone(),
                serde_json::json!({
                    "surface":{"id":"settings","kind":"window","width":720,"height":540}
                }),
            )]),
        )
        .unwrap();
        let reference = host.component("shell.taskbar").unwrap();
        let mount = host.mount(&reference).unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("settings:720"));
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert!(
            runtime
                .borrow_mut()
                .eval_json::<bool>("surfaceObservation.mountId.startsWith('composition-') && surfaceObservation.generation === 1 && surfaceObservation.scaleFactor === null && surfaceObservation.focused === null")
                .unwrap()
        );
        host.render(&mount, &serde_json::json!({})).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("JSON.stringify(surfaceObservation.generation)")
                .unwrap(),
            1
        );

        let changed = host
            .update_mount_surface(
                &mount,
                serde_json::json!({
                    "id":"settings", "kind":"window", "width":800, "height":600,
                    "output":"DP-2", "availableWidth":760, "availableHeight":560,
                    "scaleFactor":1.5, "focused":true, "visible":true
                }),
            )
            .unwrap();
        assert!(changed);
        assert!(
            !host
                .update_mount_surface(
                    &mount,
                    serde_json::json!({
                        "id":"settings", "kind":"window", "width":800, "height":600,
                        "output":"DP-2", "availableWidth":760, "availableHeight":560,
                        "scaleFactor":1.5, "focused":true, "visible":true
                    }),
                )
                .unwrap()
        );
        host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(runtime
            .borrow_mut()
            .eval_json::<bool>("surfaceObservation.output === 'DP-2' && surfaceObservation.availableSize.width === 760 && surfaceObservation.scaleFactor === 1.5 && surfaceObservation.focused === true && surfaceObservation.visible === true")
            .unwrap());
    }

    #[test]
    fn composition_host_publishes_filtered_windows_store_by_owner_snapshot() {
        let package = package(
            "windows-shell",
            "globalThis.windowObservation=null;\nexport function Taskbar(){const windows=useWindows();windowObservation={windows,generation:__windowsStore.generation};return h(Text,null,windows.map(window=>window.title).join(','));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "windows-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let first = serde_json::json!({"windows":[{"id":"1","title":"Editor","active":true}]});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("windows-shell".into(), package)]),
            "windows-shell",
            &BTreeMap::from([(owner.clone(), first.clone())]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("Editor"));
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("windowObservation.generation")
                .unwrap(),
            1
        );

        host.update_snapshot(&owner, &first).unwrap();
        host.render(&mount, &serde_json::json!({})).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__windowsStore.generation")
                .unwrap(),
            1
        );

        host.update_snapshot(
            &owner,
            &serde_json::json!({"windows":[{"id":"1","title":"Terminal","active":true}]}),
        )
        .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("Terminal"));
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("windowObservation.generation")
                .unwrap(),
            2
        );
    }

    #[test]
    fn composition_host_publishes_filtered_application_catalog_changes() {
        let package = package(
            "applications-shell",
            "globalThis.applicationObservation=null;\nexport function Taskbar(){const applications=useApplications();applicationObservation={applications,generation:__applicationsStore.generation};return h(Text,null,applications.map(value=>value.name).join(','));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "applications-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let first = serde_json::json!({"applications":[{"id":"editor","name":"Editor","icon":"application:1",
            "pinned":false,"pinOrder":null,"recentOrder":0,"kind":"application","launchClass":"graphical"}]});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("applications-shell".into(), package)]),
            "applications-shell",
            &BTreeMap::from([(owner.clone(), first.clone())]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("Editor")
        );
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__applicationsStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(&owner, &first).unwrap();
        host.render(&mount, &serde_json::json!({})).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__applicationsStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(&owner, &serde_json::json!({"applications":[
            {"id":"editor","name":"Editor","icon":"application:1","pinned":false,"pinOrder":null,"recentOrder":0,"kind":"application","launchClass":"graphical"},
            {"id":"terminal","name":"Terminal","icon":"application:2","pinned":false,"pinOrder":null,"recentOrder":1,"kind":"application","launchClass":"running","canLaunch":false,"canPin":false}
        ]})).unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("Editor,Terminal"));
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__applicationsStore.generation")
                .unwrap(),
            2
        );
    }

    #[test]
    fn composition_host_publishes_filtered_notification_lifecycle() {
        let package = package(
            "notifications-shell",
            "globalThis.notificationObservation=null;\nexport function Taskbar(){const value=useNotifications();notificationObservation={value,generation:__notificationsStore.generation};return h(Text,null,(value.notification?.summary??'none')+':'+String(value.visible));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "notifications-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let item = serde_json::json!({"id":7,"appName":"Chat","summary":"Hello","body":"Body","actions":[]});
        let first = serde_json::json!({"notifications":{"notification":item,"history":[item]}});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("notifications-shell".into(), package)]),
            "notifications-shell",
            &BTreeMap::from([(owner.clone(), first.clone())]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("Hello:true")
        );
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__notificationsStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(&owner, &first).unwrap();
        host.render(&mount, &serde_json::json!({})).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__notificationsStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(
            &owner,
            &serde_json::json!({"notifications":{"notification":null,"history":[]}}),
        )
        .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("none:false")
        );
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__notificationsStore.generation")
                .unwrap(),
            2
        );
    }

    #[test]
    fn composition_host_publishes_workspace_writability_without_revision_change() {
        let package = package(
            "workspace-shell",
            "globalThis.workspaceObservation=null;\nexport function Taskbar(){const value=useWorkspaces();workspaceObservation=value;return h(Text,null,(useWorkspace()?.id??'none')+':'+String(value.writable));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "workspace-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let first = serde_json::json!({"workspaces":{"available":true,"revision":"same","workspaces":[{"id":"1","active":true}],"activeWorkspace":"1","operations":{"switch":true}}});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("workspace-shell".into(), package)]),
            "workspace-shell",
            &BTreeMap::from([(owner.clone(), first)]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("1:true")
        );
        host.update_snapshot(&owner,&serde_json::json!({"workspaces":{"available":true,"revision":"same","workspaces":[{"id":"1","active":true}],"activeWorkspace":"1","operations":{}}})).unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("1:false")
        );
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert!(
            runtime
                .borrow_mut()
                .eval_json::<bool>(
                    "workspaceObservation.generation===2 && workspaceObservation.revision==='same'"
                )
                .unwrap()
        );
    }

    #[test]
    fn composition_host_publishes_read_only_output_availability() {
        let package = package(
            "outputs-shell",
            "globalThis.outputObservation=null;\nexport function Taskbar(){const value=useOutputs();outputObservation=value;return h(Text,null,String(value.available)+':'+String(value.outputs.length));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "outputs-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let first =
            serde_json::json!({"outputs":{"available":false,"reason":"not observed","outputs":[]}});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("outputs-shell".into(), package)]),
            "outputs-shell",
            &BTreeMap::from([(owner.clone(), first)]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("false:0")
        );
        host.update_snapshot(&owner,&serde_json::json!({"outputs":{"available":true,"revision":"read-only-1","outputs":[]}})).unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("true:0")
        );
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert!(
            runtime
                .borrow_mut()
                .eval_json::<bool>(
                    "outputObservation.generation===2 && outputObservation.revision==='read-only-1'"
                )
                .unwrap()
        );
    }

    #[test]
    fn composition_host_publishes_authoritative_locale_or_stable_fallback() {
        let package = package(
            "locale-shell",
            "globalThis.localeObservation=null;\nexport function Taskbar(){const value=useLocale();localeObservation=value;return h(Text,null,value.tag+':'+value.direction);}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "locale-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("locale-shell".into(), package)]),
            "locale-shell",
            &BTreeMap::new(),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("und:ltr")
        );
        host.update_snapshot(
            &owner,
            &serde_json::json!({"locale":{"known":true,"tag":"ar-SA","direction":"rtl"}}),
        )
        .unwrap();
        assert!(
            host.render(&mount, &serde_json::json!({}))
                .unwrap()
                .node
                .to_string()
                .contains("ar-SA:rtl")
        );
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert!(
            runtime
                .borrow_mut()
                .eval_json::<bool>("localeObservation.generation===1&&localeObservation.known")
                .unwrap()
        );
    }

    #[test]
    fn composition_host_publishes_effective_theme_independently_of_other_stores() {
        let package = package(
            "theme-shell",
            "globalThis.themeObservation=null;\nexport function Taskbar(){const theme=useTheme();themeObservation=theme;return h(Text,null,theme.mode+':'+String(useReducedMotion()));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        let owner = PackageIdentity {
            id: "theme-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let first = serde_json::json!({"appearance":{"available":true,
            "configured":{"animations":"normal","reduce_transparency":false},
            "resolved":{"theme":"dark","hue":271,"intensity":63,"accent":[10,20,30]}}});
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("theme-shell".into(), package)]),
            "theme-shell",
            &BTreeMap::from([(owner.clone(), first.clone())]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("dark:false"));
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__themeStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(&owner, &first).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("__themeStore.generation")
                .unwrap(),
            1
        );
        host.update_snapshot(
            &owner,
            &serde_json::json!({"appearance":{"available":true,
            "configured":{"animations":"off","reduce_transparency":false},
            "resolved":{"theme":"dark","hue":271,"intensity":63,"accent":[10,20,30]}}}),
        )
        .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("dark:true"));
    }

    #[test]
    fn composition_capability_store_exposes_only_owner_grants_and_availability() {
        let mut package = package(
            "capability-shell",
            "globalThis.capabilityObservation=null;\nexport function Taskbar(){const audio=useCapability('audio-read');const windows=useCapability('windows-read');capabilityObservation={audio,windows};return h(Text,null,String(audio.available)+':'+String(windows.declared));}\nexport function QuickSettings(){return h(Text,null,'settings');}\nexport default Taskbar;",
            None,
        );
        package.manifest.capabilities = vec![PluginCapability::AudioRead];
        let owner = PackageIdentity {
            id: "capability-shell".into(),
            version: package
                .manifest
                .composition
                .as_ref()
                .unwrap()
                .version
                .parse()
                .unwrap(),
        };
        let mut host = ShellCompositionRuntime::new(
            &BTreeMap::from([("capability-shell".into(), package)]),
            "capability-shell",
            &BTreeMap::from([(
                owner.clone(),
                serde_json::json!({"audio":{"available":true}}),
            )]),
        )
        .unwrap();
        let mount = host
            .mount(&host.component("shell.taskbar").unwrap())
            .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("true:false"));
        let runtime = host.shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<serde_json::Value>("JSON.stringify(capabilityObservation)")
                .unwrap(),
            serde_json::json!({"audio":{"declared":true,"available":true,"reason":null},
                "windows":{"declared":false,"available":false,"reason":null}})
        );
        host.update_snapshot(
            &owner,
            &serde_json::json!({"audio":{"available":false,"reason":"backend unavailable"}}),
        )
        .unwrap();
        let rendered = host.render(&mount, &serde_json::json!({})).unwrap();
        assert!(rendered.node.to_string().contains("false:false"));
    }
}
