//! Package-lifecycle composition with isolated JavaScript authority.
//!
//! Packages never exchange JavaScript functions, globals or handler indices.
//! The generic host invokes public components and contributions through opaque
//! Rust references, and validates native effects using their Rust-owned origin.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use nickel_core::package_composition::{
    PackageIdentity, ResolvedShellPackage, resolve_shell_package,
};
use nickel_core::plugins::PluginPackage;
use serde_json::Value;

use crate::{JsxModuleGraph, JsxRuntime, ModuleSource, ScheduledRender};

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

pub enum ScheduledExpandedBatch<T> {
    Unchanged,
    Rendered {
        rendered: RenderedComponent,
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
    data: Value,
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

struct CompositionCheckpoint {
    mounts: BTreeMap<u64, MountState>,
    effects: Vec<OwnedComponentEffect>,
    nested_mounts: BTreeMap<(u64, String), ComponentMount>,
    callbacks: BTreeMap<u64, CallbackGrant>,
    children: BTreeMap<u64, OwnedChildGrant>,
    data: BTreeMap<PackageIdentity, Value>,
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

impl ShellCompositionRuntime {
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
                    package.data = data.clone();
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
                    data,
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
        for owner in self
            .packages
            .keys()
            .filter(|owner| !owners.contains(*owner))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.retire(&owner);
        }
        for owner in owners {
            if !self.packages.contains_key(&owner) {
                let context = &contexts[&owner];
                if context.owner != owner
                    || context.package.source_digest != catalog[&owner.id].source_digest()
                    || context.package.manifest != catalog[&owner.id].manifest
                {
                    return Err("provider context identity mismatch".into());
                }
                self.packages.insert(owner, context.package.clone());
            }
        }
        self.resolution = next;
        self.publish_contribution_catalog()?;
        Ok(changed)
    }

    fn contribution_catalog_script(&self) -> String {
        let catalog=self.resolution.contributions.iter().map(|(collection,entries)|(collection.clone(),Value::Array(entries.iter().filter(|e|self.packages.contains_key(&e.contributed_by)).map(|entry|serde_json::json!({"id":entry.id,"provider":entry.contributed_by.id,"version":entry.contributed_by.version.to_string(),"key":self.contribution_key(entry)})).collect()))).collect::<serde_json::Map<_,_>>();
        let script = format!(
            "Object.keys(__nickelContributionCatalog).forEach(key=>delete __nickelContributionCatalog[key]); Object.assign(__nickelContributionCatalog, {});",
            Value::Object(catalog)
        );
        script
    }

    fn publish_contribution_catalog(&self) -> Result<(), String> {
        let script = self.contribution_catalog_script();
        for package in self.packages.values() {
            package.runtime.borrow_mut().eval(&script)?;
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

    pub fn resolution(&self) -> &ResolvedShellPackage {
        &self.resolution
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
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| "component mount identity exhausted")?;
        self.next_mount = id;
        let package = self.packages.get_mut(&reference.owner).unwrap();
        let component = if let Some(id) = reference.implementation.strip_prefix("@settings-page/") {
            if !package.runtime.borrow().has_registered_page(id) {
                return Err("Settings page is no longer published by its owner".into());
            }
            format!(
                "__nickelRegisteredPageComponent({})",
                serde_json::to_string(id).unwrap()
            )
        } else {
            let implementation = serde_json::to_string(
                package
                    .exports
                    .get(&reference.implementation)
                    .ok_or("component implementation is not published by its owner")?,
            )
            .unwrap();
            format!("nickel.component({implementation})")
        };
        package.runtime.borrow_mut().register_surface_entry(&surface(id), &format!(
            "function App() {{ const {{children, ...props}} = __nickelHydrateComponentProps(nickel.data.__componentProps); return h({component}, props, ...(children ?? [])); }}"))?;
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

    /// Dispatch through the hook scheduler without forcing a root render when
    /// every state/reducer update is referentially unchanged. This is the
    /// retained bridge prerequisite; rendered outcomes remain complete trees.
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
                    package
                        .data
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
    /// Changed batches use the full expansion compatibility path for now.
    pub fn dispatch_expanded_batch_scheduled_pending_validated<T>(
        &mut self,
        root: &ComponentMount,
        events: &[(ComponentEventHandle, Value)],
        validate: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<ScheduledExpandedBatch<T>, String> {
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

            let mut changed = false;
            let mut reconciliation_requested = false;
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
                let outcome = self.render_mount_scheduled(mount, Value::Array(batch))?;
                changed |= outcome.rendered.is_some() || outcome.requires_expansion;
                reconciliation_requested |= outcome.reconciliation_requested;
                offset = end;
            }

            if !changed {
                return Ok(ScheduledExpandedBatch::Unchanged);
            }
            let props = self.mounts[&root.id].props.clone();
            let (rendered, validated) = self.render_expanded_validated(root, &props, validate)?;
            Ok(ScheduledExpandedBatch::Rendered {
                rendered,
                validated,
                reconciliation_requested,
            })
        })();
        if result.is_err() {
            self.finish_transaction(false)?;
        }
        result
    }

    fn expand_rendered<T>(
        &mut self,
        root: u64,
        rendered: RenderedComponent,
        validate: impl FnOnce(&Value, u64) -> Result<T, String>,
    ) -> Result<(RenderedComponent, T), String> {
        let generation = rendered.generation;
        let mut expansion = ExpansionState {
            root,
            events: BTreeMap::new(),
            visited: std::collections::BTreeSet::new(),
            native_ids: std::collections::BTreeSet::new(),
            handler_slots: std::collections::BTreeSet::new(),
        };
        let node = self.expand_node(
            "root",
            rendered.node,
            &rendered.events,
            root,
            &mut expansion,
            0,
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
        Ok((
            RenderedComponent {
                node,
                events: expansion.events,
                generation,
            },
            validated,
        ))
    }

    fn expand_node(
        &mut self,
        path: &str,
        mut node: Value,
        source_events: &BTreeMap<u64, ComponentEventHandle>,
        source_mount: u64,
        expansion: &mut ExpansionState,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 64 || expansion.events.len() > MAX_HANDLERS {
            return Err("composition expansion exceeds limits".into());
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
                self.nested_mounts.insert(identity, mount.clone());
                mount
            };
            let props = node.get("props").ok_or("missing public component props")?;
            self.callbacks.retain(|_, grant| grant.receiver != mount.id);
            self.children.retain(|_, grant| grant.receiver != mount.id);
            let props =
                self.transport_callback_props(props, source_events, &mount, source_mount, depth)?;
            let rendered = self.render(&mount, &props)?;
            let mut embedded = rendered.node;
            namespace_native_metadata(&mut embedded, &key, 0)?;
            return self.expand_node(
                &key,
                embedded,
                &rendered.events,
                mount.id,
                expansion,
                depth + 1,
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
                    let identity = if let Some(key) = value.get("key") {
                        format!(
                            "@{}",
                            key.to_string()
                                .bytes()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>()
                        )
                    } else {
                        format!("#{index}")
                    };
                    *value = self.expand_node(
                        &format!("{path}/{identity}"),
                        std::mem::take(value),
                        source_events,
                        source_mount,
                        expansion,
                        depth + 1,
                    )?;
                }
            }
            Value::Object(object) => {
                for (key, value) in object {
                    if key == "__handlerSlots" {
                        continue;
                    } else if is_action(key) && !value.is_null() {
                        let handle = source_events
                            .get(&value.as_u64().ok_or("invalid host event token")?)
                            .ok_or("unknown host event token")?
                            .clone();
                        let token = expansion.events.len() as u64;
                        expansion.events.insert(token, handle);
                        *value = Value::from(token);
                    } else {
                        *value = self.expand_node(
                            &format!("{path}/{key}"),
                            std::mem::take(value),
                            source_events,
                            source_mount,
                            expansion,
                            depth + 1,
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
                if object.len() != 1 {
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
            .map(|package| &package.data)
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
        package.data = data.clone();
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
        if let Some(id) = reference.implementation.strip_prefix("@settings-page/") {
            if self
                .packages
                .get(&reference.owner)
                .is_none_or(|package| !package.runtime.borrow().has_registered_page(id))
            {
                return Err("Settings page is no longer published by its owner".into());
            }
        }
        Ok(())
    }

    fn render_mount(
        &mut self,
        id: u64,
        event: Option<Value>,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        let catalog_script = self.contribution_catalog_script();
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
        runtime.eval(&catalog_script)?;
        runtime.select_surface(&surface(id))?;
        runtime.set_surface_store(&surface(id), &surface_snapshot)?;
        let mut data = package.data.clone();
        let object = data
            .as_object_mut()
            .ok_or("package snapshot must be an object")?;
        if let Some(surface) = &state.surface {
            object.insert("surface".into(), surface.clone());
        }
        object.insert("__componentProps".into(), state.props.clone());
        runtime.set_data_value(data)?;
        let expression = event.as_ref().map_or_else(
            || "__nickelRender()".into(),
            |events| format!("__nickelDispatchBatch({events})"),
        );
        let result = runtime.render(&expression, |value| {
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
        let catalog_script = self.contribution_catalog_script();
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
        runtime.eval(&catalog_script)?;
        runtime.select_surface(&surface(id))?;
        runtime.set_surface_store(&surface(id), &surface_snapshot)?;
        let mut data = package.data.clone();
        let object = data
            .as_object_mut()
            .ok_or("package snapshot must be an object")?;
        if let Some(surface) = &state.surface {
            object.insert("surface".into(), surface.clone());
        }
        object.insert("__componentProps".into(), state.props.clone());
        runtime.set_data_value(data)?;
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
        Ok(ScheduledComponentDispatch {
            rendered,
            reconciliation_requested,
            requires_expansion: self.next_generation != generation_before_effects,
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
                self.render_mount(mount, Some(Value::Array(events)), |_| Ok(()))?;
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
            for (key, value) in object {
                if key == "__handlerSlots" {
                    continue;
                } else if is_action(key) && !value.is_null() {
                    let action = value
                        .as_u64()
                        .filter(|action| *action < MAX_HANDLERS as u64)
                        .ok_or("invalid component action")?;
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
            "export function Shell(){const [state]=useState({count:0});const ref=useRef({count:0});return h(Column,null,h(Text,null,'base'+state.count+':'+ref.current.count),h(nickel.component('shell.taskbar'),{onChange:()=>{state.count++;ref.current.count++;nickel.windows.activate('base');}}));}\nexport function Taskbar(){}\nexport function QuickSettings(){}\nexport default Shell;",
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
                .map(|_| serde_json::json!({"action": 0}))
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
