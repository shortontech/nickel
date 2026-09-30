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

use crate::{JsxModuleGraph, JsxRuntime, ModuleSource};

static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);
const MAX_MOUNTS: usize = 512;
const MAX_NODES: usize = 4096;
const MAX_JSON_BYTES: usize = 1024 * 1024;
const MAX_EFFECTS: usize = 1024;

#[derive(Clone, Debug)]
pub struct ComponentReference {
    runtime: u64,
    owner: PackageIdentity,
    implementation: String,
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
}

struct ExpansionState {
    root: u64,
    events: BTreeMap<u64, ComponentEventHandle>,
    visited: std::collections::BTreeSet<String>,
}

struct PackageRuntime {
    runtime: std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
    data: Value,
    exports: BTreeMap<String, String>,
}
struct MountState {
    reference: ComponentReference,
    generation: u64,
    props: Value,
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
    effects: Vec<OwnedComponentEffect>,
    nested_mounts: BTreeMap<(u64, String), ComponentMount>,
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
        let resolution = resolve_shell_package(&manifests, active)
            .map_err(|error| format!("composition failed: {error:?}"))?;
        let mut host = Self {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            resolution,
            packages: BTreeMap::new(),
            mounts: BTreeMap::new(),
            next_mount: 0,
            effects: Vec::new(),
            nested_mounts: BTreeMap::new(),
        };
        let mut source_bytes = 0usize;
        for owner in host.resolution.inheritance_chain.clone() {
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
            .with_component_bridge();
            let data = snapshots
                .get(&owner)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            bounded_json(&data)?;
            if !data.is_object() {
                return Err("package snapshot must be an object".into());
            }
            let runtime = std::rc::Rc::new(std::cell::RefCell::new(JsxRuntime::new_modules(
                &graph,
                Some(&data.to_string()),
            )?));
            host.packages.insert(
                owner.clone(),
                PackageRuntime {
                    runtime,
                    data,
                    exports: export_keys,
                },
            );
            host.drain_effects(&owner)?;
        }
        Ok(host)
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
            })
    }

    pub fn contributions(&self, collection: &str) -> Vec<ComponentReference> {
        self.resolution
            .contributions
            .get(collection)
            .into_iter()
            .flatten()
            .map(|entry| ComponentReference {
                runtime: self.id,
                owner: entry.contributed_by.clone(),
                implementation: entry.implementation.clone(),
            })
            .collect()
    }

    pub fn mount(&mut self, reference: &ComponentReference) -> Result<ComponentMount, String> {
        if reference.runtime != self.id || !self.packages.contains_key(&reference.owner) {
            return Err("foreign or retired component reference".into());
        }
        if self.mounts.len() >= MAX_MOUNTS {
            return Err("too many component mounts".into());
        }
        let id = self
            .next_mount
            .checked_add(1)
            .ok_or("component mount identity exhausted")?;
        self.next_mount = id;
        let package = self.packages.get_mut(&reference.owner).unwrap();
        let implementation = serde_json::to_string(
            package
                .exports
                .get(&reference.implementation)
                .ok_or("component implementation is not published by its owner")?,
        )
        .unwrap();
        package.runtime.borrow_mut().register_surface_entry(&surface(id), &format!(
            "function App() {{ const {{children, ...props}} = nickel.data.__componentProps; return h(nickel.component({implementation}), props, ...(children ?? [])); }}"))?;
        self.mounts.insert(
            id,
            MountState {
                reference: reference.clone(),
                generation: 0,
                props: serde_json::json!({}),
            },
        );
        Ok(ComponentMount {
            runtime: self.id,
            id,
        })
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

    /// Accept a production renderer's tree validation before committing hooks,
    /// handlers or effects. Rejection rolls back the existing render transaction.
    pub fn render_validated(
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
        self.render_mount(handle.mount, Some((handle.action, value)), validate)
    }

    /// Expand public component mount requests through their owning contexts.
    /// Data props cross as bounded snapshots; executable props are rejected
    /// explicitly until an opaque callback prop transport is available.
    pub fn render_expanded(
        &mut self,
        mount: &ComponentMount,
        props: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        let result = self
            .render(mount, props)
            .and_then(|rendered| self.expand_rendered(mount.id, rendered, validate));
        if result.is_err() {
            // Each context has committed its local render by this point. Until
            // multi-context rollback exists, retire the transaction participants
            // so failed expansion cannot retain executable callbacks/effects.
            let owners = self.packages.keys().cloned().collect::<Vec<_>>();
            for owner in owners {
                self.retire(&owner);
            }
        }
        result
    }

    pub fn dispatch_expanded(
        &mut self,
        root: &ComponentMount,
        handle: &ComponentEventHandle,
        value: &Value,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        self.validate_mount(root)?;
        self.dispatch(handle, value)?;
        let props = self.mounts[&root.id].props.clone();
        self.render_expanded(root, &props, validate)
    }

    fn expand_rendered(
        &mut self,
        root: u64,
        rendered: RenderedComponent,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        let mut expansion = ExpansionState {
            root,
            events: BTreeMap::new(),
            visited: std::collections::BTreeSet::new(),
        };
        let node = self.expand_node("root", rendered.node, &rendered.events, &mut expansion, 0)?;
        bounded_json(&node)?;
        validate(&node)?;
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
        Ok(RenderedComponent {
            node,
            events: expansion.events,
        })
    }

    fn expand_node(
        &mut self,
        path: &str,
        mut node: Value,
        source_events: &BTreeMap<u64, ComponentEventHandle>,
        expansion: &mut ExpansionState,
        depth: usize,
    ) -> Result<Value, String> {
        if depth > 64 || expansion.events.len() > MAX_NODES {
            return Err("composition expansion exceeds limits".into());
        }
        if node.get("kind").and_then(Value::as_str) == Some("__packageComponent") {
            let contract = node
                .get("contract")
                .and_then(Value::as_str)
                .ok_or("missing public component contract")?;
            let reference = self
                .component(contract)
                .ok_or("unknown public component contract")?;
            let key = format!("{path}/{contract}");
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
            let rendered = self.render(&mount, props)?;
            return self.expand_node(&key, rendered.node, &rendered.events, expansion, depth + 1);
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
                        expansion,
                        depth + 1,
                    )?;
                }
            }
            Value::Object(object) => {
                for (key, value) in object {
                    if is_action(key) && !value.is_null() {
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
        self.packages
            .get_mut(owner)
            .ok_or("retired snapshot owner")?
            .data = data.clone();
        Ok(())
    }

    /// Check this immediately before the host validates and executes the effect.
    /// Outstanding envelopes from a retired or different host are stale.
    pub fn validate_effect(&self, effect: &OwnedComponentEffect) -> Result<(), String> {
        if effect.runtime != self.id || !self.packages.contains_key(&effect.owner) {
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
        self.mounts
            .retain(|_, mount| &mount.reference.owner != owner);
        self.packages.remove(owner);
        self.nested_mounts
            .retain(|_, mount| self.mounts.contains_key(&mount.id));
        self.effects.retain(|effect| &effect.owner != owner);
    }

    pub fn take_effects(&mut self) -> Vec<OwnedComponentEffect> {
        std::mem::take(&mut self.effects)
    }

    fn validate_mount(&self, mount: &ComponentMount) -> Result<(), String> {
        if mount.runtime != self.id || !self.mounts.contains_key(&mount.id) {
            return Err("foreign or retired component mount".into());
        }
        Ok(())
    }

    fn render_mount(
        &mut self,
        id: u64,
        event: Option<(u64, &Value)>,
        validate: impl FnOnce(&Value) -> Result<(), String>,
    ) -> Result<RenderedComponent, String> {
        let state = &self.mounts[&id];
        let owner = state.reference.owner.clone();
        let generation = state
            .generation
            .checked_add(1)
            .ok_or("component generation exhausted")?;
        let package = self
            .packages
            .get_mut(&owner)
            .ok_or("retired component owner")?;
        let mut runtime = package.runtime.borrow_mut();
        runtime.select_surface(&surface(id))?;
        let mut data = package.data.clone();
        let object = data
            .as_object_mut()
            .ok_or("package snapshot must be an object")?;
        object.insert("__componentProps".into(), state.props.clone());
        runtime.set_data(&data.to_string())?;
        let expression = event.map_or_else(
            || "__nickelRender()".into(),
            |(action, value)| format!("__nickelDispatch({action},{value})"),
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
                &mut 0,
            )?;
            validate(&node)?;
            Ok(RenderedComponent { node, events })
        });
        if event.is_some() {
            runtime.finish_event(result.is_ok())?;
        }
        let rendered = result?;
        drop(runtime);
        self.mounts.get_mut(&id).unwrap().generation = generation;
        if let Err(error) = self.drain_effects(&owner) {
            self.retire(&owner);
            return Err(error);
        }
        Ok(rendered)
    }

    fn drain_effects(&mut self, owner: &PackageIdentity) -> Result<(), String> {
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
        for value in effects {
            self.effects.push(OwnedComponentEffect {
                runtime: self.id,
                owner: owner.clone(),
                value,
            });
        }
        Ok(())
    }
}

fn is_action(key: &str) -> bool {
    matches!(
        key,
        "action"
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
    if value.to_string().len() > MAX_JSON_BYTES {
        return Err("composition JSON exceeds size limit".into());
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
    count: &mut usize,
) -> Result<(), String> {
    *count += 1;
    if *count > MAX_NODES {
        return Err("component tree exceeds node limit".into());
    }
    match value {
        Value::Array(values) => {
            for value in values {
                rewrite_events(value, runtime, mount, generation, owner, events, count)?;
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                if is_action(key) && !value.is_null() {
                    let action = value
                        .as_u64()
                        .filter(|action| *action < MAX_NODES as u64)
                        .ok_or("invalid component action")?;
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
                    rewrite_events(value, runtime, mount, generation, owner, events, count)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
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
    fn executable_props_are_rejected_without_lossy_serialization() {
        let mut base = package(
            "base",
            "export function Shell() { return h(nickel.component('shell.taskbar'), {onChange:()=>nickel.windows.activate('base')}); }\nexport function Taskbar() {}\nexport function QuickSettings() {}\nexport default Shell;",
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
            "export function Taskbar() { return h(Text,null,'child'); }\nexport default Taskbar;",
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
        let error = host
            .render_expanded(&mount, &serde_json::json!({}), |_| Ok(()))
            .err()
            .unwrap();
        assert!(error.contains("opaque callback transport"));
        assert!(host.take_effects().is_empty());
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
}
