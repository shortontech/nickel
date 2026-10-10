//! Nickel catalog and capability policy over Twinkle composition execution.
use crate::composition_admission::{NickelCompositionHost, admitted_link_plan};
use crate::{CapabilityRuntimeExt, JsxModuleGraph, ModuleSource, NickelModuleBindings};
use nickel_core::package_composition::{
    PackageIdentity, ResolvedShellPackage, resolve_shell_package,
};
use nickel_core::plugins::PluginPackage;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
pub use twinkle_jsx_runtime::composition::{ComponentMount, PackageReloadSignatures};
use twinkle_jsx_runtime::composition::{CompositionRuntime, OwnerRuntime};
use twinkle_protocol::native_tree::bounded_json;
pub type ComponentReference =
    twinkle_jsx_runtime::composition::ComponentReference<NickelCompositionHost>;
pub type ComponentEventHandle =
    twinkle_jsx_runtime::composition::ComponentEventHandle<NickelCompositionHost>;
pub type OwnedComponentEffect =
    twinkle_jsx_runtime::composition::OwnedComponentEffect<NickelCompositionHost>;
pub type RenderedComponent =
    twinkle_jsx_runtime::composition::RenderedComponent<NickelCompositionHost>;
pub type ScheduledComponentDispatch =
    twinkle_jsx_runtime::composition::ScheduledComponentDispatch<NickelCompositionHost>;
pub type ReloadedMountPatch =
    twinkle_jsx_runtime::composition::ReloadedMountPatch<NickelCompositionHost>;
pub type CompositionReload =
    twinkle_jsx_runtime::composition::CompositionReload<NickelCompositionHost>;
pub type ProviderContext = twinkle_jsx_runtime::composition::ProviderContext<NickelCompositionHost>;
pub type ScheduledExpandedBatch<T> =
    twinkle_jsx_runtime::composition::ScheduledExpandedBatch<NickelCompositionHost, T>;
pub struct ShellCompositionRuntime {
    resolution: ResolvedShellPackage,
    engine: CompositionRuntime<NickelCompositionHost>,
}
impl Deref for ShellCompositionRuntime {
    type Target = CompositionRuntime<NickelCompositionHost>;
    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}
impl DerefMut for ShellCompositionRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.engine
    }
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
    .and_then(|graph| {
        graph.with_nickel_component_bridge(
            Value::Object(contributions),
            Value::Object(serde_json::Map::new()),
        )
    })
}

impl ShellCompositionRuntime {
    pub fn mount(&mut self, reference: &ComponentReference) -> Result<ComponentMount, String> {
        self.engine.mount(reference)
    }
    #[cfg(test)]
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
            self.context(&entry.contributed_by).incarnation
        )
    }
    pub fn resolution(&self) -> &ResolvedShellPackage {
        &self.resolution
    }
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
            .filter(|(_, p)| p.manifest.composition.is_some())
            .map(|(id, p)| {
                let composition = p.manifest.composition.clone().unwrap();
                if p.manifest.id != composition.id
                    || p.manifest.version.as_deref() != Some(&composition.version)
                {
                    return Err("installed composition identity mismatch".into());
                }
                Ok((id.clone(), composition))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let mut resolution = resolve_shell_package(&manifests, active)
            .map_err(|e| format!("composition failed: {e:?}"))?;
        extend_installed_contributions(&mut resolution, &manifests)?;
        let mut owners = resolution.inheritance_chain.clone();
        for (id, manifest) in &manifests {
            if !owners.iter().any(|owner| owner.id == *id) {
                owners.push(
                    resolve_shell_package(&manifests, &manifest.id)
                        .map_err(|e| format!("contributor composition failed: {e:?}"))?
                        .active,
                );
            }
        }
        let mut packages = BTreeMap::new();
        let mut new_owners = Vec::new();
        let mut source_bytes = 0usize;
        for owner in owners {
            let package = &catalog[&owner.id];
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
                let mut retained = context.retained_context();
                if context.owner() != &owner
                    || retained.source_digest != package.source_digest()
                    || retained.host_state != package.manifest
                {
                    return Err("provider context identity mismatch".into());
                }
                if let Some(data) = snapshots.get(&owner) {
                    bounded_json(data)?;
                    if !data.is_object() {
                        return Err("package snapshot must be an object".into());
                    }
                    retained.data = std::rc::Rc::new(data.clone());
                }
                packages.insert(owner, retained);
                continue;
            }
            let graph = package_graph(package, &resolution)?;
            let data = snapshots
                .get(&owner)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            bounded_json(&data)?;
            if !data.is_object() {
                return Err("package snapshot must be an object".into());
            }
            let mut runtime = crate::create_module_runtime(&graph, Some(&data.to_string()))?;
            runtime.set_diagnostic_owner(&owner.id)?;
            runtime.set_capability_store(&package.manifest.capabilities, &data)?;
            let composition = package.manifest.composition.as_ref().unwrap();
            let mut exports = BTreeMap::new();
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
                if !exports.contains_key(implementation) {
                    exports.insert(
                        implementation.clone(),
                        format!("owned.component.{}", exports.len()),
                    );
                }
            }
            let mut context = OwnerRuntime::admit(
                std::rc::Rc::new(std::cell::RefCell::new(runtime)),
                package.manifest.clone(),
                std::rc::Rc::new(data),
                exports,
                BTreeMap::new(),
                package.source_digest(),
            )?;
            if package.images.len() > 1024
                || package
                    .images
                    .keys()
                    .any(|name| name.is_empty() || name.len() > 512)
            {
                return Err("owner context contains invalid asset bindings".into());
            }
            context.assets = package
                .images
                .keys()
                .enumerate()
                .map(|(index, name)| {
                    (
                        name.clone(),
                        format!("composition.asset.{}.{index}", context.incarnation),
                    )
                })
                .collect();
            new_owners.push(owner.clone());
            packages.insert(owner, context);
        }
        let mut engine = CompositionRuntime::from_admitted_plan(
            admitted_link_plan(&resolution),
            NickelCompositionHost,
            packages,
        )?;
        for owner in new_owners {
            engine.collect_initial_owner_effects(&owner)?;
        }
        engine.publish_contribution_catalog()?;
        Ok(Self { resolution, engine })
    }
    pub fn reload_catalog(
        &mut self,
        catalog: &BTreeMap<String, PluginPackage>,
        signatures: &BTreeMap<PackageIdentity, PackageReloadSignatures>,
    ) -> Result<CompositionReload, String> {
        let snapshots = self
            .engine
            .owner_contexts()
            .map(|(owner, context)| (owner.clone(), context.data.as_ref().clone()))
            .collect();
        let candidate = Self::new(catalog, &self.resolution.active.id, &snapshots)?;
        let graphs = catalog
            .values()
            .filter(|p| p.manifest.composition.is_some())
            .map(|p| {
                let owner = candidate
                    .engine
                    .participating_owners()
                    .find(|owner| owner.id == p.manifest.id)
                    .ok_or("replacement owner is unavailable")?
                    .clone();
                Ok((owner, package_graph(p, &candidate.resolution)?))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let next = candidate.resolution;
        let result = self
            .engine
            .reload_admitted(candidate.engine, &graphs, signatures)?;
        self.resolution = next;
        Ok(result)
    }
    pub fn sync_contributors(
        &mut self,
        catalog: &BTreeMap<String, PluginPackage>,
        contexts: &BTreeMap<PackageIdentity, ProviderContext>,
    ) -> Result<bool, String> {
        if self.transaction_pending() {
            return Err("cannot reconcile contributors during a composition transaction".into());
        }
        let manifests = catalog
            .iter()
            .filter_map(|(id, p)| p.manifest.composition.clone().map(|c| (id.clone(), c)))
            .collect();
        let next = resolve_shell_package(&manifests, &self.resolution.active.id)
            .map_err(|e| format!("composition failed: {e:?}"))?;
        if next.exports != self.resolution.exports
            || next.inheritance_chain != self.resolution.inheritance_chain
        {
            return Err("shell ancestry changed during contributor reconciliation".into());
        }
        let mut admitted = contexts.clone();
        for owner in self.engine.participating_owners() {
            if catalog.contains_key(&owner.id) {
                admitted
                    .entry(owner.clone())
                    .or_insert(self.engine.provider_context(owner)?);
            }
        }
        let required = manifests
            .values()
            .map(|manifest| {
                resolve_shell_package(&manifests, &manifest.id)
                    .map(|resolution| resolution.active)
                    .map_err(|e| format!("contributor composition failed: {e:?}"))
            })
            .collect::<Result<std::collections::BTreeSet<_>, String>>()?;
        if required.iter().any(|owner| !admitted.contains_key(owner)) {
            return Err("admitted contributor context is unavailable".into());
        }
        admitted.retain(|owner, _| required.contains(owner));
        let candidate = Self::new_with_contexts(
            catalog,
            &self.resolution.active.id,
            &BTreeMap::new(),
            &admitted,
        )?;
        let owners_changed =
            candidate.engine.owner_contexts().count() != self.engine.owner_contexts().count();
        let changed = self
            .engine
            .sync_admitted_contributors(admitted_link_plan(&candidate.resolution), &admitted)?;
        self.resolution = candidate.resolution;
        self.engine.publish_contribution_catalog()?;
        Ok(changed || owners_changed)
    }
    #[cfg(test)]
    fn registered_page(&self, provider: &str, id: &str) -> Option<ComponentReference> {
        self.engine
            .registered_component(&serde_json::json!({"settingPage":{"provider":provider,"id":id}}))
            .ok()
            .flatten()
    }
    #[cfg(test)]
    fn context(&self, owner: &PackageIdentity) -> &OwnerRuntime<NickelCompositionHost> {
        self.engine
            .owner_contexts()
            .find(|(id, _)| *id == owner)
            .unwrap()
            .1
    }
    pub fn retire(&mut self, owner: &PackageIdentity) {
        self.engine.retire(owner);
        for entries in self.resolution.contributions.values_mut() {
            entries.retain(|entry| &entry.contributed_by != owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativePatchOperation, SettingsRuntimeExt};
    use twinkle_protocol::native_tree::is_action;
    fn surface(id: u64) -> String {
        format!("composition-{id}")
    }
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
        let owner = host.participating_owners().next().unwrap().clone();
        let inventory = json!({"inventory": (0..1000)
            .map(|index| json!({"id":index,"label":format!("Row {index}")}))
            .collect::<Vec<_>>()});
        host.update_snapshot(&owner, &inventory).unwrap();
        let original = host.context(&owner).data.clone();
        host.begin_transaction().unwrap();
        assert!(host.transaction_pending());
        let replacement = json!({"inventory":[],"revision":2});
        host.update_snapshot(&owner, &replacement).unwrap();
        assert!(!std::rc::Rc::ptr_eq(&original, &host.context(&owner).data));
        assert_eq!(original.as_ref(), &inventory);
        host.finish_transaction(false).unwrap();
        assert!(std::rc::Rc::ptr_eq(&original, &host.context(&owner).data));
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
        assert!(host.owner_contexts().next().is_none());
        assert!(host.mount_count() == 0);
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
        for (owner, package) in host.owner_contexts() {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.owner_contexts().map(|(_, package)| package) {
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
        host.owner_contexts()
            .map(|(_, package)| package)
            .find(|package| package.host_state.id == id)
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
            host.owner_contexts().count(),
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
        assert_eq!(host.owner_contexts().count(), 1);
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
        assert_eq!(host.owner_contexts().count(), 1);
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
        for (owner, package) in host.owner_contexts() {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.owner_contexts().map(|(_, package)| package) {
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
        assert_eq!(host.owner_contexts().count(), 2);

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
        for (owner, package) in host.owner_contexts() {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.owner_contexts().map(|(_, package)| package) {
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
        for (owner, package) in host.owner_contexts() {
            package
                .runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)
                .unwrap();
        }
        for package in host.owner_contexts().map(|(_, package)| package) {
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
        assert_eq!(fallback.events[&0].action_id(), events[&0].action_id());
        let recovered =
            accept_scheduled_event(&mut host, &root, fallback.events[&0].clone(), &fallback);
        assert!(recovered.node.to_string().contains("leaf0"));
        assert!(recovered.node.to_string().contains("clean sibling"));
        assert_eq!(host.owner_contexts().count(), 2);
        let diagnostics = host
            .owner_contexts()
            .map(|(_, package)| package)
            .find(|package| package.host_state.id == "child")
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
        let owner = host.mount_owner(&root).unwrap().clone();
        let diagnostics = host
            .context(&owner)
            .runtime
            .borrow_mut()
            .runtime_diagnostics()
            .unwrap();
        let profile = diagnostics["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["surface"] == surface(root.id()))
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
        let old_mount = edited.events[&action(&edited.node, "counter").unwrap()].mount_id();
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
            events.values().any(|event| event.mount_id() == old_mount),
            "surviving child mount retired"
        );
        let counter_token = action(&edited.node, "counter").unwrap();
        let retained_counter = events
            .get(&counter_token)
            .expect("surviving native action lost its stable expanded token")
            .clone();
        assert_eq!(retained_counter.mount_id(), old_mount);
        host.finish_transaction(true).unwrap();
        let next = host
            .dispatch_expanded(&root, &retained_counter, &Value::Null, |_| Ok(()))
            .unwrap();
        assert!(next.node.to_string().contains("Count 2"));
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
            .find(|(_, handle)| handle.mount_id() == root.id())
            .unwrap()
            .0;
        let navigation = initial
            .events
            .iter()
            .find(|(_, handle)| handle.mount_id() != root.id())
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
        let mounts_before_replace = host.mount_count();
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
        assert_eq!(host.mount_count(), mounts_before_replace);
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
        assert!(host.mount_count() < mounts_before_replace);
        host.finish_transaction(true).unwrap();
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
        let token = host.callback_transport_tokens().next().unwrap();
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
        assert!(host.callback_transport_tokens().next().is_none());
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
        assert!(host.owned_child_grant_count() == 0);
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
        let compilations = runtime.borrow_mut().script_compilation_count();
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
                runtime.borrow_mut().script_compilation_count(),
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
        assert_eq!(tree.generation(), event.generation());

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
