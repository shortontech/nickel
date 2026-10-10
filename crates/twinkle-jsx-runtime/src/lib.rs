//! JavaScript evaluator and transaction machinery for native Twinkle presentation.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the direct V8 context and the bootstrap's render and event transactions.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Instant;

mod bindings;
pub use bindings::{DataPublisher, HostBindings};
mod engine;
pub mod modules;

pub use modules::{JsxModuleGraph, ModuleSource};

/// Self-contained source fixtures for optional native preview tooling.
pub mod examples {
    pub const STANDALONE_TSX: &str = include_str!("../examples/standalone.tsx");
    pub const STANDALONE_CSS: &str = include_str!("../examples/standalone.css");
}

const BOOTSTRAP: &str = include_str!("bootstrap.js");

fn encoded_json_len(value: &Value) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("encoded JSON length overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|error| error.to_string())?;
    Ok(counter.0)
}

pub struct JsxRuntime {
    engine: engine::JavascriptEngine,
    registration_owner: Option<String>,
    registered_exports: std::collections::BTreeSet<String>,
    capability_vocabulary: Option<Vec<String>>,
    settings_revision: u64,
    settings_data: Option<std::rc::Rc<Value>>,
    mount_settings_identity: Option<(std::rc::Rc<Value>, Value)>,
    mount_transport_identity: Option<std::rc::Rc<Value>>,
    data_publisher: Option<DataPublisher>,
    projection_names: Vec<String>,
    projection_identities: std::collections::BTreeMap<String, std::rc::Rc<Value>>,
    checkpoint: Option<(u64, Option<std::rc::Rc<Value>>)>,
    invalidated: bool,
    maintenance_checkpoint: u64,
    maintenance_drain_pending: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotReloadIdentity<'a> {
    pub owner: &'a str,
    pub module: &'a str,
    pub export: &'a str,
    pub signature: &'a str,
}

/// Deterministic low-level driver for runtime conformance tests. It deliberately
/// bypasses native presentation while retaining the normal render/event/effect
/// transaction boundaries.
#[doc(hidden)]
pub struct JsxTestHarness {
    runtime: JsxRuntime,
    source: String,
    data: Option<String>,
    accepted: Option<Value>,
    errors: Vec<String>,
    pending_events: Vec<(usize, Value)>,
}

/// Native event slots exposed by a materialized JSX node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsxTestEvent {
    Click,
    ContextMenu,
    Drag,
    Drop,
    Focus,
    Blur,
    Select,
    Move,
    FileAction,
    Close,
    Escape,
    Submit,
}

impl JsxTestEvent {
    fn property(self) -> &'static str {
        match self {
            Self::Click => "action",
            Self::ContextMenu => "contextAction",
            Self::Drag => "dragAction",
            Self::Drop => "dropAction",
            Self::Focus => "focusAction",
            Self::Blur => "blurAction",
            Self::Select => "selectAction",
            Self::Move => "moveAction",
            Self::FileAction => "fileAction",
            Self::Close => "closeAction",
            Self::Escape => "escapeAction",
            Self::Submit => "submitAction",
        }
    }
}

impl JsxTestHarness {
    pub fn new(source: &str, data: Option<&str>) -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(source, data)?,
            source: source.into(),
            data: data.map(str::to_owned),
            accepted: None,
            errors: Vec::new(),
            pending_events: Vec::new(),
        })
    }
    pub fn render(&mut self) -> Result<&Value, String> {
        let value = self
            .runtime
            .render("__nickelRender()", |node| Ok(node.clone()));
        match value {
            Ok(value) => {
                self.accepted = Some(value);
                Ok(self.accepted.as_ref().unwrap())
            }
            Err(error) => {
                self.errors.push(error.clone());
                Err(error)
            }
        }
    }
    pub fn rerender(&mut self) -> Result<&Value, String> {
        self.render()
    }
    /// Return all materialized native nodes whose `id` exactly matches `id`.
    pub fn query_all_by_id(&self, id: &str) -> Vec<Value> {
        self.query_nodes(|node| node.get("id").and_then(Value::as_str) == Some(id))
    }
    /// Return the unique materialized native node whose `id` matches `id`.
    pub fn get_by_id(&self, id: &str) -> Result<Value, String> {
        self.unique_query(format!("id {id:?}"), self.query_all_by_id(id))
    }
    /// Return all nodes containing `text` as a direct textual child.
    pub fn query_all_by_text(&self, text: &str) -> Vec<Value> {
        self.query_nodes(|node| {
            node.get("children")
                .and_then(Value::as_array)
                .is_some_and(|children| children.iter().any(|child| child.as_str() == Some(text)))
        })
    }
    pub fn get_by_text(&self, text: &str) -> Result<Value, String> {
        self.unique_query(format!("text {text:?}"), self.query_all_by_text(text))
    }
    /// Return all nodes with the explicit accessibility `role`.
    pub fn query_all_by_role(&self, role: &str) -> Vec<Value> {
        self.query_nodes(|node| node.get("role").and_then(Value::as_str) == Some(role))
    }
    pub fn get_by_role(&self, role: &str) -> Result<Value, String> {
        self.unique_query(format!("role {role:?}"), self.query_all_by_role(role))
    }
    /// Resolve a typed event slot from a node returned by a semantic query.
    pub fn event_action(node: &Value, event: JsxTestEvent) -> Result<usize, String> {
        let property = event.property();
        node.get(property)
            .and_then(Value::as_u64)
            .and_then(|action| usize::try_from(action).ok())
            .ok_or_else(|| format!("node has no {property} event"))
    }
    pub fn inject_store(&mut self, name: &str, value: &Value) -> Result<(), String> {
        let setter = match name {
            "theme" => "__nickelSetThemeStore",
            "locale" => "__nickelSetLocaleStore",
            _ => return Err("unknown test store".into()),
        };
        self.runtime.eval(&format!("{setter}({value})"))
    }
    pub fn inject_locale(&mut self, value: &Value) -> Result<bool, String> {
        self.runtime.set_locale_store(value)
    }
    pub fn inject_surface(&mut self, mount_id: &str, value: &Value) -> Result<bool, String> {
        self.runtime.set_surface_store(mount_id, value)
    }
    pub fn inject_capabilities(&mut self, snapshot: &Value) -> Result<bool, String> {
        self.runtime.set_capability_snapshot(snapshot)
    }
    pub fn event(&mut self, action: usize, value: &Value) -> Result<&Value, String> {
        self.dispatch_events(&[(action, value.clone())])
    }
    /// Queue an event without rendering. `flush_updates` admits every queued
    /// event in one runtime event turn and performs exactly one render.
    pub fn queue_event(&mut self, action: usize, value: Value) {
        self.pending_events.push((action, value));
    }
    pub fn queue_node_event(
        &mut self,
        node: &Value,
        event: JsxTestEvent,
        value: Value,
    ) -> Result<(), String> {
        self.queue_event(Self::event_action(node, event)?, value);
        Ok(())
    }
    pub fn flush_updates(&mut self) -> Result<&Value, String> {
        let events = std::mem::take(&mut self.pending_events);
        self.dispatch_events(&events)
    }
    pub fn dispatch_events(&mut self, events: &[(usize, Value)]) -> Result<&Value, String> {
        let expression = format!(
            "__nickelDispatchBatch({})",
            serde_json::to_string(events).map_err(|error| error.to_string())?
        );
        let result = self.runtime.render(&expression, |node| Ok(node.clone()));
        match result {
            Ok(value) => {
                self.runtime.finish_event(true)?;
                self.accepted = Some(value);
                Ok(self.accepted.as_ref().unwrap())
            }
            Err(error) => {
                self.errors.push(error.clone());
                Err(error)
            }
        }
    }
    pub fn flush_effects(&mut self) -> Result<Vec<Value>, String> {
        self.runtime.take_effects()
    }
    pub fn unmount(&mut self) -> Result<(), String> {
        self.runtime.drop_surface("default")
    }
    pub fn errors(&self) -> &[String] {
        &self.errors
    }
    pub fn cold_equivalent(&self) -> Result<bool, String> {
        let Some(accepted) = &self.accepted else {
            return Err("test harness has no accepted render".into());
        };
        let mut cold = JsxRuntime::new(&self.source, self.data.as_deref())?;
        let rendered = cold.render("__nickelRender()", |node| Ok(node.clone()))?;
        Ok(&rendered == accepted)
    }
    pub fn runtime_mut(&mut self) -> &mut JsxRuntime {
        &mut self.runtime
    }

    fn accepted(&self) -> Result<&Value, String> {
        self.accepted
            .as_ref()
            .ok_or_else(|| "test harness has no accepted render".into())
    }

    fn query_nodes(&self, predicate: impl Fn(&Value) -> bool) -> Vec<Value> {
        fn visit(value: &Value, predicate: &impl Fn(&Value) -> bool, matches: &mut Vec<Value>) {
            match value {
                Value::Object(_) => {
                    if predicate(value) {
                        matches.push(value.clone());
                    }
                    if let Some(children) = value.get("children").and_then(Value::as_array) {
                        for child in children {
                            visit(child, predicate, matches);
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        visit(value, predicate, matches);
                    }
                }
                _ => {}
            }
        }
        let mut matches = Vec::new();
        if let Ok(root) = self.accepted() {
            visit(root, &predicate, &mut matches);
        }
        matches
    }

    fn unique_query(&self, description: String, matches: Vec<Value>) -> Result<Value, String> {
        match matches.len() {
            1 => Ok(matches.into_iter().next().unwrap()),
            count => Err(format!(
                "expected one node matching {description}, found {count}"
            )),
        }
    }
}

pub use twinkle_protocol::{
    NativePatchCounters, NativePatchEnvelope, NativePatchOperation, ScheduledPatch, ScheduledRender,
};

#[derive(Deserialize)]
struct ScheduledRenderWire {
    rendered: bool,
    #[serde(default)]
    dirty: Vec<String>,
    node: Option<Value>,
}

#[derive(Deserialize)]
struct ScheduledPatchWire {
    rendered: bool,
    #[serde(default)]
    dirty: Vec<String>,
    patch: Option<NativePatchEnvelope>,
}

#[derive(Deserialize)]
struct ReconciliationRequest {
    requested: bool,
}

impl JsxRuntime {
    pub fn new(source: &str, data: Option<&str>) -> Result<Self, String> {
        Self::new_with_bindings(source, data, &HostBindings::default())
    }
    pub fn new_with_bindings(
        source: &str,
        data: Option<&str>,
        bindings: &HostBindings,
    ) -> Result<Self, String> {
        let mut runtime = Self {
            engine: engine::JavascriptEngine::new(),
            registration_owner: None,
            registered_exports: Default::default(),
            capability_vocabulary: None,
            settings_revision: 0,
            settings_data: None,
            mount_settings_identity: None,
            mount_transport_identity: None,
            data_publisher: bindings.publisher,
            projection_names: bindings.projections.clone(),
            projection_identities: Default::default(),
            checkpoint: None,
            invalidated: false,
            maintenance_checkpoint: 0,
            maintenance_drain_pending: false,
        };
        runtime.eval(BOOTSTRAP)?;
        if !bindings.bootstrap.is_empty() {
            runtime.eval(&bindings.bootstrap)?;
        }
        runtime.eval("Object.freeze(__hostCheckpointParticipants)")?;
        if let Some(data) = data {
            runtime.set_data(data)?;
        }
        runtime.eval(source)?;
        Ok(runtime)
    }

    /// Starts one package in one JavaScript context. Every module is evaluated
    /// at most once and shares the bootstrap, hooks, effects, and surface state.
    pub fn new_modules(graph: &JsxModuleGraph, data: Option<&str>) -> Result<Self, String> {
        Self::new(&graph.compile()?, data)
    }

    pub fn hot_install_modules(
        &mut self,
        graph: &JsxModuleGraph,
        signatures: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), String> {
        let compiled = graph.compile()?;
        let signatures = serde_json::to_string(signatures).map_err(|e| e.to_string())?;
        self.eval(&format!(
            "__nickelInstallHotModules(function(){{ (function(){{{compiled}}})(); }}, {signatures})"
        ))
    }

    /// Host-owned registration identity; never derived from executable package data.
    pub fn registration_owner(&self) -> Option<&str> {
        self.registration_owner.as_deref()
    }
    pub fn set_registration_owner(&mut self, owner: Option<String>) {
        self.registration_owner = owner;
    }
    pub fn set_registered_exports(&mut self, exports: std::collections::BTreeSet<String>) {
        self.registered_exports = exports;
    }
    pub fn has_registered_export(&self, id: &str) -> bool {
        self.registered_exports.contains(id)
    }
    pub fn observation_revision(&self) -> u64 {
        self.settings_revision
    }
    pub fn advance_observation_revision(&mut self) {
        self.settings_revision = self.settings_revision.wrapping_add(1);
    }

    pub fn eval(&mut self, source: &str) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.eval(source)
    }

    pub fn set_diagnostic_owner(&mut self, owner: &str) -> Result<(), String> {
        self.call_native_void("__nickelSetDiagnosticOwner", &[Value::from(owner)])
    }

    pub fn set_contribution_catalog(&mut self, catalog: &Value) -> Result<(), String> {
        self.call_native_void(
            "__nickelSetContributionCatalog",
            std::slice::from_ref(catalog),
        )
    }

    pub fn consume_reconciliation(&mut self) -> Result<(), String> {
        self.call_native_void("__nickelConsumeReconciliation", &[])
    }

    /// Invoke already loaded host bookkeeping without compiling an expression
    /// whose source changes with IDs, timings or payload byte counts.
    fn call_native_void(&mut self, name: &str, arguments: &[Value]) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.call_global_void(name, arguments)
    }

    fn call_native_json<T: DeserializeOwned>(
        &mut self,
        name: &str,
        arguments: &[Value],
    ) -> Result<T, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.call_global_json(name, arguments)
    }

    pub fn eval_json<T: DeserializeOwned>(&mut self, source: &str) -> Result<T, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.eval_json(source)
    }

    pub fn set_data(&mut self, serialized_json: &str) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let data: Value =
            serde_json::from_str(serialized_json).map_err(|error| error.to_string())?;
        self.set_data_value(data)
    }

    /// Pass an already parsed host snapshot without serializing and reparsing it.
    pub fn set_data_value(&mut self, mut data: Value) -> Result<(), String> {
        self.mount_settings_identity = None;
        self.mount_transport_identity = None;
        self.projection_identities.clear();
        self.publish_data_stores(&data)?;
        self.engine
            .call_global_void("__nickelSetData", std::slice::from_ref(&data))?;
        // Surface geometry does not change package setting values.
        if let Some(object) = data.as_object_mut() {
            object.remove("surface");
        }
        if self.settings_data.as_deref() != Some(&data) {
            self.settings_revision = self.settings_revision.wrapping_add(1);
            self.settings_data = Some(std::rc::Rc::new(data));
        }
        Ok(())
    }

    /// Retain immutable host inventory while overlaying mount-local fields.
    /// Compatibility reads lazily copy exposed fields; their mutable objects
    /// are never reused across publications or surfaces.
    pub fn set_mount_data(
        &mut self,
        data: &std::rc::Rc<Value>,
        surface: Option<&Value>,
        props: &Value,
    ) -> Result<(), String> {
        let object = data
            .as_object()
            .ok_or("package snapshot must be an object")?;
        let reused: Vec<String> = self
            .projection_names
            .iter()
            .filter(|name| {
                self.projection_identities
                    .get(*name)
                    .is_some_and(|previous| std::rc::Rc::ptr_eq(previous, data))
            })
            .cloned()
            .collect();
        self.publish_data_stores_with_reuse(
            data,
            &reused.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        for name in &self.projection_names {
            self.projection_identities
                .insert(name.clone(), data.clone());
        }
        // Invalidate before calling: a bridge exception can occur after V8 has
        // replaced its retained base, so the next attempt must resend it.
        let previous_transport = self.mount_transport_identity.take();
        let reuse_base = previous_transport
            .as_ref()
            .is_some_and(|previous| std::rc::Rc::ptr_eq(previous, data));
        let absent = Value::Null;
        let has_surface = Value::Bool(surface.is_some());
        self.engine.call_global_object_fields(
            "__nickelSetMountData",
            &[
                ("base", if reuse_base { &absent } else { data.as_ref() }),
                ("surface", surface.unwrap_or(&absent)),
                ("hasSurface", &has_surface),
                ("props", props),
            ],
        )?;
        self.mount_transport_identity = Some(data.clone());
        // This identity caches only native settings comparison, never guest
        // objects or store publication. Holding the immutable Rc also makes
        // host mutation use copy-on-write instead of changing it in place.
        if self
            .mount_settings_identity
            .as_ref()
            .is_some_and(|(previous, previous_props)| {
                std::rc::Rc::ptr_eq(previous, data) && previous_props == props
            })
        {
            return Ok(());
        }
        // Only changed native settings need a comparison projection. Building
        // and sorting it before the identity check allocates on every reused
        // mount even though the V8 surface/props publication above is bounded.
        let mut fields = object
            .iter()
            .filter(|(key, _)| {
                key.as_str() != "__componentProps"
                    && (surface.is_none() || key.as_str() != "surface")
            })
            .map(|(key, value)| (key.as_str(), value))
            .collect::<Vec<_>>();
        if let Some(surface) = surface {
            fields.push(("surface", surface));
        }
        fields.push(("__componentProps", props));
        // Match serde_json::Map's ordering in the former owned projection.
        fields.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let setting_fields = || fields.iter().filter(|(key, _)| *key != "surface");
        let unchanged = self
            .settings_data
            .as_deref()
            .and_then(Value::as_object)
            .is_some_and(|previous| {
                previous.len() == setting_fields().count()
                    && setting_fields().all(|(key, value)| previous.get(*key) == Some(*value))
            });
        if !unchanged {
            self.settings_revision = self.settings_revision.wrapping_add(1);
            self.settings_data = Some(std::rc::Rc::new(Value::Object(
                setting_fields()
                    .map(|(key, value)| ((*key).to_owned(), (*value).clone()))
                    .collect(),
            )));
        }
        self.mount_settings_identity = Some((data.clone(), props.clone()));
        Ok(())
    }

    fn publish_data_stores(&mut self, data: &Value) -> Result<(), String> {
        self.publish_data_stores_with_reuse(data, &[])
    }
    fn publish_data_stores_with_reuse(
        &mut self,
        data: &Value,
        reused: &[&str],
    ) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        match self.data_publisher {
            Some(publish) => publish(self, data, reused),
            None => Ok(()),
        }
    }
    /// Trusted host observation bridges validate schemas in their registered implementation.
    pub fn call_host_observation(
        &mut self,
        function: &str,
        args: &[Value],
    ) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.call_global_bool(function, args)
    }
    pub fn validate_host_observation(&mut self, function: &str) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.call_global_void(function, &[])
    }
    pub fn invalidate_projection(&mut self, name: &str) {
        self.projection_identities.remove(name);
    }
    pub fn bridge_value_count(&mut self) -> u64 {
        self.engine.diagnostics().bridge_values
    }
    pub fn microtask_checkpoint_count(&mut self) -> u64 {
        self.engine.diagnostics().microtask_checkpoints
    }

    pub fn set_locale_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine
            .call_global_bool("__nickelSetLocaleStore", std::slice::from_ref(snapshot))
    }

    /// Publish host-owned effective presentation state. This is observation,
    /// not the capability-gated appearance configuration authority.
    pub fn set_theme_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine
            .call_global_bool("__nickelSetThemeStore", std::slice::from_ref(snapshot))
    }

    /// Publishes host-defined capability observations. This grants no action authority.
    pub fn set_capability_snapshot(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let names: Vec<String> = serde_json::from_value(
            snapshot
                .get("known")
                .cloned()
                .ok_or("missing capability vocabulary")?,
        )
        .map_err(|_| "invalid capability vocabulary")?;
        if self
            .capability_vocabulary
            .as_ref()
            .is_some_and(|registered| registered != &names)
        {
            return Err("capability vocabulary is frozen for this runtime".into());
        }
        let changed = self
            .engine
            .call_global_bool("__nickelSetCapabilityStore", std::slice::from_ref(snapshot))?;
        self.capability_vocabulary = Some(names);
        Ok(changed)
    }

    pub fn select_surface(&mut self, id: &str) -> Result<(), String> {
        self.call_native_void("__nickelSelectSurface", &[Value::from(id)])
    }

    /// Publish one host-owned, mount-scoped surface observation. JavaScript
    /// retains the previous immutable snapshot when these public fields are
    /// unchanged and advances its monotonic generation otherwise.
    pub fn set_surface_store(&mut self, mount_id: &str, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.engine.call_global_bool(
            "__nickelSetSurfaceStore",
            &[Value::String(mount_id.to_owned()), snapshot.clone()],
        )
    }

    pub fn register_surface_entry(&mut self, id: &str, source: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!(
            "__nickelRegisterSurfaceApp({id}, (function() {{\n{source}\nreturn App;\n}})())"
        ))
    }

    pub fn register_component_surface(
        &mut self,
        id: &str,
        selection: &str,
        registered_page: bool,
    ) -> Result<(), String> {
        self.call_native_void(
            "__nickelRegisterComponentSurface",
            &[
                Value::from(id),
                Value::from(selection),
                Value::Bool(registered_page),
            ],
        )
    }

    pub fn register_surface_entry_with_identity(
        &mut self,
        id: &str,
        source: &str,
        identity: &HotReloadIdentity<'_>,
    ) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        let identity = serde_json::to_string(identity).map_err(|error| error.to_string())?;
        self.eval(&format!(
            "__nickelRegisterSurfaceApp({id}, (function() {{\n{source}\nreturn App;\n}})(), {identity})"
        ))
    }

    /// Atomically admits a development replacement after it evaluates. State
    /// is retained only for the exact same function and full source identity;
    /// incompatible replacements clean up the old surface before admission.
    pub fn hot_replace_surface_entry(
        &mut self,
        id: &str,
        source: &str,
        identity: &HotReloadIdentity<'_>,
    ) -> Result<bool, String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        let identity = serde_json::to_string(identity).map_err(|error| error.to_string())?;
        self.eval_json(&format!(
            "JSON.stringify(__nickelReplaceSurfaceApp({id}, (function() {{\n{source}\nreturn App;\n}})(), {identity}))"
        ))
    }

    pub fn drop_surface(&mut self, id: &str) -> Result<(), String> {
        self.call_native_void("__nickelDropSurface", &[Value::from(id)])
    }

    pub fn render<T>(
        &mut self,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let value = self.eval_json::<Value>(expression);
        self.finish_render(value, parse)
    }

    /// Render an already loaded surface, optionally after an admitted native
    /// event batch. This retains the complete-tree validation/commit boundary
    /// used for cold mounts without compiling a call expression.
    pub fn render_native<T>(
        &mut self,
        events: Option<&Value>,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let value = match events {
            Some(events) => {
                self.call_native_json("__nickelDispatchBatch", std::slice::from_ref(events))
            }
            None => self.call_native_json("__nickelRender", &[]),
        };
        self.finish_render(value, parse)
    }

    pub fn render_current<T>(
        &mut self,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        self.render_native(None, parse)
    }

    fn finish_render<T>(
        &mut self,
        value: Result<Value, String>,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let transport_bytes = value
            .as_ref()
            .ok()
            .map(encoded_json_len)
            .transpose()?
            .unwrap_or(0);
        let validation_started = Instant::now();
        let parsed = value.and_then(|value| parse(&value));
        let validation_micros = validation_started
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX));
        let finalizer = if parsed.is_ok() {
            "__nickelCommitRender"
        } else {
            "__nickelRollbackRender"
        };
        self.call_native_void(finalizer, &[])
            .map_err(|error| format!("could not finalize plugin render: {error}"))?;
        self.report_host_profile("cold-tree", validation_micros as u64, transport_bytes)?;
        parsed
    }

    /// Legacy/cold dispatch through the retained hook scheduler. Production
    /// mounts with admitted patch authority use [`Self::dispatch_patched`];
    /// this complete-tree result remains for initial admission, cold-oracle
    /// comparison, and mounts that have not acquired patch authority. A no-op
    /// executes no component and needs no native validation.
    pub fn dispatch_scheduled<T>(
        &mut self,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<ScheduledRender<T>, String> {
        let outcome = self.eval_json::<ScheduledRenderWire>(expression)?;
        if !outcome.rendered {
            if outcome.node.is_some() || !outcome.dirty.is_empty() {
                return Err("invalid unchanged scheduler outcome".into());
            }
            return Ok(ScheduledRender::Unchanged);
        }
        let node = outcome.node.ok_or("scheduled render omitted its tree")?;
        let transport_bytes = encoded_json_len(&node)?;
        let validation_started = Instant::now();
        let parsed = parse(&node);
        let validation_micros = validation_started
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX));
        self.call_native_void(
            if parsed.is_ok() {
                "__nickelCommitRender"
            } else {
                "__nickelRollbackRender"
            },
            &[],
        )
        .map_err(|error| format!("could not finalize scheduled plugin render: {error}"))?;
        self.report_host_profile("cold-tree", validation_micros as u64, transport_bytes)?;
        let value = parsed?;
        let reconciliation_requested = self
            .call_native_json::<ReconciliationRequest>("__nickelReconciliationRequest", &[])?
            .requested;
        Ok(ScheduledRender::Rendered {
            value,
            dirty_components: outcome.dirty,
            reconciliation_requested,
        })
    }

    /// Dispatch a same-package event and transport only dirty component-owned
    /// native subtrees. The JavaScript runtime does not materialize or compare
    /// the complete accepted root on this path.
    pub fn dispatch_patched(&mut self, expression: &str) -> Result<ScheduledPatch, String> {
        let wire_value = self.eval_json::<Value>(expression)?;
        self.decode_scheduled_patch(wire_value)
    }

    /// Production dispatch to the already loaded scheduler. Events remain
    /// data, not newly compiled JavaScript source. Admission and native patch
    /// validation are identical to the expression-based diagnostic adapter.
    pub fn dispatch_batch_patched(
        &mut self,
        events: Vec<Value>,
        previous: bool,
    ) -> Result<ScheduledPatch, String> {
        let wire_value = self.call_native_json(
            "__nickelDispatchBatchPatched",
            &[Value::Array(events), Value::Bool(previous)],
        )?;
        self.decode_scheduled_patch(wire_value)
    }

    pub fn dispatch_slots_patched(&mut self, events: Vec<Value>) -> Result<ScheduledPatch, String> {
        let wire_value =
            self.call_native_json("__nickelDispatchSlotsPatched", &[Value::Array(events)])?;
        self.decode_scheduled_patch(wire_value)
    }

    fn decode_scheduled_patch(&mut self, wire_value: Value) -> Result<ScheduledPatch, String> {
        let transport_bytes = encoded_json_len(&wire_value)?;
        self.report_host_profile("typed-patch", 0, transport_bytes)?;
        let outcome: ScheduledPatchWire =
            serde_json::from_value(wire_value).map_err(|error| error.to_string())?;
        if !outcome.rendered {
            if outcome.patch.is_some() || !outcome.dirty.is_empty() {
                return Err("invalid unchanged patch scheduler outcome".into());
            }
            return Ok(ScheduledPatch::Unchanged);
        }
        let patch = outcome.patch.ok_or("scheduled render omitted its patch")?;
        if patch.version != 1 || patch.operations.len() > 256 {
            return Err("unsupported or oversized native patch envelope".into());
        }
        let reconciliation_requested = self
            .call_native_json::<ReconciliationRequest>("__nickelReconciliationRequest", &[])?
            .requested;
        Ok(ScheduledPatch::Patched {
            patch,
            dirty_components: outcome.dirty,
            transport_bytes,
            reconciliation_requested,
        })
    }

    pub fn finish_patch_render(&mut self, accepted: bool) -> Result<(), String> {
        self.call_native_void(
            if accepted {
                "__nickelCommitRender"
            } else {
                "__nickelRollbackRender"
            },
            &[],
        )
        .map_err(|error| format!("could not finalize native patch render: {error}"))
    }

    /// Checkpoint bootstrap-owned hooks, surface handlers and queued effects.
    /// Plain object/array hook values preserve identity on rollback. Unsupported
    /// or irreversible restoration invalidates execution. Package globals and
    /// arbitrary closure state remain outside this contract.
    pub fn begin_transaction(&mut self) -> Result<(), String> {
        if self.checkpoint.is_some() {
            return Err("runtime transaction already pending".into());
        }
        self.call_native_void("__nickelBeginCheckpoint", &[])?;
        self.checkpoint = Some((self.settings_revision, self.settings_data.clone()));
        Ok(())
    }

    pub fn finish_transaction(&mut self, accepted: bool) -> Result<(), String> {
        let (revision, data) = self
            .checkpoint
            .take()
            .ok_or("runtime transaction is unavailable")?;
        // A rejected transaction restores a different settings-data owner.
        // Discard the comparison shortcut before attempting that restoration.
        if !accepted {
            self.mount_settings_identity = None;
            self.mount_transport_identity = None;
            self.projection_identities.clear();
        }
        if let Err(error) =
            self.call_native_void("__nickelFinishCheckpoint", &[Value::from(accepted)])
        {
            self.invalidated = true;
            return Err(error);
        }
        if !accepted {
            self.settings_revision = revision;
            self.settings_data = data;
        }
        Ok(())
    }

    pub fn take_effects(&mut self) -> Result<Vec<Value>, String> {
        self.call_native_json("__nickelTakeEffects", &[])
    }

    pub fn reconciliation_requested(&mut self) -> Result<bool, String> {
        Ok(self
            .call_native_json::<ReconciliationRequest>("__nickelReconciliationRequest", &[])?
            .requested)
    }

    /// Returns the bounded, coalesced component-failure evidence retained by
    /// this package context. Reading diagnostics does not clear them.
    pub fn boundary_diagnostics(&mut self) -> Result<Vec<Value>, String> {
        self.call_native_json("__nickelBoundaryDiagnostics", &[])
    }

    /// Translate a rejected native candidate into the nearest JSX boundary.
    /// Callers must roll back the candidate transaction first. The returned
    /// paths are dirtied for a fresh, commit-gated fallback reconciliation.
    pub fn capture_native_failure(
        &mut self,
        owners: &[String],
        message: &str,
    ) -> Result<Vec<String>, String> {
        if owners.len() > 256 {
            return Err("too many native failure owners".into());
        }
        let expression = format!(
            "__nickelCaptureNativeFailure({}, {})",
            serde_json::to_string(owners).map_err(|error| error.to_string())?,
            serde_json::to_string(&message.chars().take(512).collect::<String>())
                .map_err(|error| error.to_string())?,
        );
        let value: Value = self.eval_json(&expression)?;
        serde_json::from_value(
            value
                .get("captured")
                .cloned()
                .ok_or("native failure capture omitted boundary paths")?,
        )
        .map_err(|error| error.to_string())
    }

    /// Bounded render diagnostics plus per-mount execution and transport profiles.
    pub fn runtime_diagnostics(&mut self) -> Result<Value, String> {
        let mut diagnostics: Value = self.call_native_json("__nickelRuntimeDiagnostics", &[])?;
        let object = diagnostics
            .as_object_mut()
            .ok_or("runtime diagnostics did not return an object")?;
        object.insert(
            "engine".into(),
            serde_json::to_value(self.engine.diagnostics()).map_err(|error| error.to_string())?,
        );
        Ok(diagnostics)
    }

    /// Native diagnostic only: may collect garbage and contains runtime data.
    /// Never exposed to package JavaScript. Snapshot runs are not representative
    /// normal-memory or timing evidence; the serialized output is capped at 128 MiB.
    /// Number of compiled scripts, for host-side reuse and lifecycle diagnostics.
    pub fn script_compilation_count(&mut self) -> u64 {
        self.engine.diagnostics().script_compilations
    }

    pub fn diagnostic_heap_snapshot(&mut self) -> Result<Vec<u8>, String> {
        self.engine.diagnostic_heap_snapshot()
    }

    /// Native diagnostic only. Services up to eight already queued platform
    /// tasks, without waiting or explicitly requesting GC. The two-millisecond
    /// admission budget cannot preempt an individual native task. Not exposed
    /// to package JavaScript and never permitted during a pending transaction.
    pub fn diagnostic_pump_platform_tasks(&mut self) -> Result<usize, String> {
        self.service_platform_tasks()
    }

    /// Service a bounded foreground batch at a native host-owned safe point.
    /// The caller must subsequently reconcile dirty surfaces and validate
    /// effects through ordinary admission. This never waits or requests GC.
    /// Pending bootstrap renders/events are checked before every task.
    pub fn service_platform_tasks(&mut self) -> Result<usize, String> {
        if self.invalidated || self.checkpoint.is_some() {
            return Err("platform task servicing requires an idle valid runtime".into());
        }
        let result = self.engine.diagnostic_pump_platform_tasks();
        self.maintenance_checkpoint = self.engine.microtask_checkpoint_count();
        self.maintenance_drain_pending = result.as_ref().is_ok_and(|count| *count > 0);
        result
    }

    /// Native transaction state for hosts selecting an idle maintenance owner.
    /// This read does not enter JavaScript or advance microtasks.
    pub fn transaction_pending(&self) -> bool {
        self.checkpoint.is_some()
    }

    /// Native-only observation; does not enter V8, allocate, or collect garbage.
    pub fn platform_tasks_serviced(&self) -> u64 {
        self.engine.platform_tasks_serviced()
    }

    /// Native-only, allocation-free scheduling hint shared by every surface of
    /// this runtime. Ordinary bridge activity coalesces until a service attempt;
    /// a nonempty batch requests another drain, an empty batch goes idle.
    /// This does not inspect V8's queue or replace the servicing readiness checks.
    pub fn platform_maintenance_pending(&self) -> bool {
        !self.invalidated
            && self.checkpoint.is_none()
            && (self.maintenance_drain_pending
                || self.engine.microtask_checkpoint_count() != self.maintenance_checkpoint)
    }

    /// Inspect one surface without selecting it or rebuilding its declaration.
    pub fn surface_work_pending(&mut self, surface: &str) -> Result<bool, String> {
        if self.invalidated || self.checkpoint.is_some() {
            return Err("surface work query requires an idle valid runtime".into());
        }
        self.engine.call_global_bool_without_microtasks(
            "__nickelSurfaceWorkPending",
            &[Value::String(surface.to_owned())],
        )
    }

    fn report_host_profile(
        &mut self,
        transport_kind: &str,
        native_validation_micros: u64,
        transport_bytes: usize,
    ) -> Result<(), String> {
        self.call_native_void(
            "__nickelReportHostProfile",
            &[
                Value::from(transport_kind),
                Value::from(native_validation_micros),
                Value::from(transport_bytes),
            ],
        )
    }

    /// Record one host-owned typed patch application without retaining the
    /// patch, candidate tree, or validation error in JavaScript diagnostics.
    pub fn report_typed_patch_apply(
        &mut self,
        application_micros: u64,
        accepted: bool,
    ) -> Result<(), String> {
        self.call_native_void(
            "__nickelReportTypedPatchApply",
            &[Value::from(application_micros), Value::from(accepted)],
        )
    }

    pub fn finish_event(&mut self, accepted: bool) -> Result<(), String> {
        if accepted {
            self.settings_revision = self.settings_revision.wrapping_add(1);
        }
        self.call_native_void(
            if accepted {
                "__nickelAcceptEvent"
            } else {
                "__nickelRollbackEvent"
            },
            &[],
        )
        .map_err(|error| format!("could not finalize plugin event: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_declarations_match_public_runtime_globals() {
        let declarations = include_str!("../types/twinkle.d.ts");
        let declared: std::collections::BTreeSet<String> = declarations
            .lines()
            .filter_map(|line| {
                let declaration = line
                    .strip_prefix("declare function ")
                    .or_else(|| line.strip_prefix("declare const "))?;
                Some(
                    declaration
                        .split(['(', '<', ':'])
                        .next()
                        .unwrap()
                        .trim()
                        .to_owned(),
                )
            })
            .collect();
        let mut runtime = JsxRuntime::new("", None).unwrap();
        let observed: std::collections::BTreeSet<String> = runtime
            .eval_json("JSON.stringify(__nickelPublicRuntimeGlobals)")
            .unwrap();
        assert_eq!(declared, observed);
        assert!(!declarations.contains("Nickel"));
    }
    #[test]
    fn standalone_tsx_fixture_prepares_and_renders_without_shell_bindings() {
        let graph = JsxModuleGraph::new(
            "standalone.tsx",
            [
                ModuleSource {
                    path: "standalone.tsx",
                    source: include_str!("../examples/standalone.tsx"),
                },
                ModuleSource {
                    path: "standalone.css",
                    source: include_str!("../examples/standalone.css"),
                },
            ],
        )
        .unwrap();
        let mut runtime = JsxRuntime::new_modules(&graph, None).unwrap();
        let node = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(node["id"], "main");
        assert!(
            runtime
                .eval_json::<bool>("typeof nickel === 'undefined'")
                .unwrap()
        );
    }

    #[test]
    fn standalone_bootstrap_omits_nickel_services_and_settings_registration() {
        let mut runtime = super::JsxRuntime::new("", None).unwrap();
        assert!(runtime.eval_json::<bool>("typeof nickel === 'undefined' && typeof registerSetting === 'undefined' && typeof registerSettingsPage === 'undefined' && typeof readPluginSettings === 'undefined' && typeof useWindows === 'undefined' && typeof useApplications === 'undefined' && typeof NickelStores === 'undefined' && typeof useCapability === 'undefined'").unwrap());
        let bindings = HostBindings::new(HostBindings::ABI_VERSION,"globalThis.hostState={value:1};__twinkleRegisterCheckpointParticipant({capture:copy=>copy(hostState),restore:(state,original)=>{hostState=original(state)}})",|_,_,_|Ok(()),&[]).unwrap();
        let mut runtime = super::JsxRuntime::new_with_bindings(
            "twinkle.request({type:'example'})",
            None,
            &bindings,
        )
        .unwrap();
        runtime.begin_transaction().unwrap();
        runtime
            .eval("hostState.value=2;twinkle.request({type:'provisional'})")
            .unwrap();
        runtime.finish_transaction(false).unwrap();
        assert_eq!(runtime.eval_json::<u64>("hostState.value").unwrap(), 1);
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"example"})]
        );
    }

    #[test]
    fn standalone_capabilities_require_explicit_host_vocabulary() {
        let mut runtime = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useHostCapability('example-read').available))}",
            None,
        )
        .unwrap();
        assert!(runtime.render("__nickelRender()", |_| Ok(())).is_err());
        let snapshot = serde_json::json!({"known":["example-read"],"entries":{"example-read":{"declared":true,"available":true}}});
        assert!(runtime.set_capability_snapshot(&snapshot).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_capability_snapshot(&snapshot).unwrap());
        assert!(
            runtime
                .set_capability_snapshot(
                    &serde_json::json!({"known":["replacement-read"],"entries":{}})
                )
                .is_err()
        );
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(__capabilityStore.known)")
                .unwrap(),
            serde_json::json!(["example-read"])
        );

        assert!(
            runtime
                .set_capability_snapshot(
                    &serde_json::json!({"known":["duplicate","duplicate"],"entries":{}})
                )
                .is_err()
        );
        assert!(runtime.eval("function Unknown(){useHostCapability('windows-read')} __nickelSetApp(Unknown); __nickelRender()").is_err());
    }

    #[test]
    fn jsx_test_harness_queries_events_batches_and_injects_domain_stores() {
        use super::{JsxTestEvent, JsxTestHarness};

        let source = r#"
            function App() {
                const [count,setCount]=useState(0);
                const locale=useLocale(), surface=useSurface(), capability=useHostCapability('windows-read');
                return h(Column,{id:'root'},
                    h(Button,{id:'increment',role:'button',onClick:()=>setCount(value=>value+1)},'Increment'),
                    h(Text,{id:'summary'},count+':'+locale.tag+':'+surface.id+':'+capability.available));
            }
        "#;
        let mut harness = JsxTestHarness::new(source, None).unwrap();
        harness
            .inject_locale(&serde_json::json!({"known":true,"tag":"fr-FR","direction":"ltr"}))
            .unwrap();
        harness
            .inject_surface(
                "mount",
                &serde_json::json!({"id":"settings","kind":"window"}),
            )
            .unwrap();
        harness
            .inject_capabilities(
                &serde_json::json!({"known":["windows-read"],"entries":{"windows-read":{"declared":true,"available":true}}}),
            )
            .unwrap();
        harness.render().unwrap();
        let button = harness.get_by_role("button").unwrap();
        assert_eq!(button["id"], "increment");
        assert_eq!(harness.get_by_text("Increment").unwrap()["id"], "increment");
        assert_eq!(
            harness.get_by_id("summary").unwrap()["children"][0],
            "0:fr-FR:settings:true"
        );

        harness
            .queue_node_event(&button, JsxTestEvent::Click, Value::Null)
            .unwrap();
        harness
            .queue_node_event(&button, JsxTestEvent::Click, Value::Null)
            .unwrap();
        harness.flush_updates().unwrap();
        assert_eq!(
            harness.get_by_id("summary").unwrap()["children"][0],
            "2:fr-FR:settings:true"
        );
        assert!(harness.flush_effects().unwrap().is_empty());

        harness
            .inject_locale(&serde_json::json!({"known":true,"tag":"de-DE","direction":"ltr"}))
            .unwrap();
        let diagnostics = harness.runtime_mut().runtime_diagnostics().unwrap();
        let locale_change = diagnostics["storeChanges"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        assert_eq!(locale_change["store"], "locale");
        assert_eq!(locale_change["newlyDirty"], 1);
        harness.rerender().unwrap();
        assert_eq!(
            harness.get_by_id("summary").unwrap()["children"][0],
            "2:de-DE:settings:true"
        );
    }

    #[test]
    fn error_boundary_contains_render_memo_and_reducer_failures() {
        let source = r#"
            globalThis.phase = 'ok';
            function Leaf({value}) {
                const [state,dispatch]=useReducer((old,action)=>{if(action==='fail')throw Error('reducer boom');return old+1},0);
                if (phase === 'render') throw Error('render boom');
                return h(Button,{onClick:()=>dispatch(phase==='reducer'?'fail':'ok')},value+':'+state);
            }
            const MemoLeaf=memo(Leaf,()=>{if(phase==='memo')throw Error('memo boom');return false});
            function App(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:()=>{phase='ok';reset()}},'failed:'+error.message)},h(MemoLeaf,{value:phase}))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(initial["children"][0], "ok:0");

        runtime.eval("phase='render'").unwrap();
        let failed = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(failed["children"][0], "failed:render boom");
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        let reset = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(reset["children"][0], "ok:0");

        runtime.eval("phase='memo'").unwrap();
        let failed = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(failed["children"][0], "failed:memo boom");
        runtime.eval("phase='ok'").unwrap();
        runtime.eval("Array.from(__componentRecords.values()).find(record=>record.boundary).boundaryError=undefined; for(const path of __componentRecords.keys())__dirtyComponents.add(path)").unwrap();
        let healthy = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(healthy["children"][0], "ok:0");

        runtime.eval("phase='reducer'").unwrap();
        let reduced = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            reduced["children"][0],
            "failed:reducer boom",
            "diagnostics: {:?}",
            runtime.boundary_diagnostics().unwrap()
        );
    }

    #[test]
    fn boundary_effect_failure_is_deferred_and_diagnostics_are_bounded() {
        let source = r#"
            function Child(){useEffect(()=>{throw Error('effect boom')},[]);return h(Text,null,'valid')}
            function App(){return h(ErrorBoundary,{fallback:h(Text,null,'fallback')},h(Child))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let committed = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(committed["children"][0], "valid");
        let fallback = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(fallback["children"][0], "fallback");
        assert_eq!(
            runtime.boundary_diagnostics().unwrap()[0]["phase"],
            "effect"
        );

        runtime.eval("for(let i=0;i<80;i++)__nickelRecordBoundaryFailure('root/'+i,'render',Error('failure '+i))").unwrap();
        assert_eq!(runtime.boundary_diagnostics().unwrap().len(), 32);
    }

    #[test]
    fn failing_surface_does_not_replace_an_independent_surface_or_its_handlers() {
        let source = r#"
            function App(){return h(ErrorBoundary,{fallback:h(Text,null,'settings failed')},
                twinkle.data.fail ? h((()=>{throw Error('settings')})) : h(Button,{onClick:()=>twinkle.request('show-launcher')},twinkle.data.name))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .set_data(r#"{"name":"taskbar","fail":false}"#)
            .unwrap();
        let taskbar = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        runtime.register_surface_entry("settings", source).unwrap();
        runtime.select_surface("settings").unwrap();
        runtime
            .set_data(r#"{"name":"settings","fail":true}"#)
            .unwrap();
        let settings = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(settings["children"][0], "settings failed");

        runtime.select_surface("default").unwrap();
        let unchanged = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(unchanged["__nativeId"], taskbar["__nativeId"]);
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(runtime.take_effects().unwrap().len(), 1);
    }

    #[test]
    fn hot_replacement_preserves_only_compatible_state_and_rejects_atomically() {
        let source = "function App(){const [value,setValue]=useState(0);useEffect(()=>()=>log.push('cleanup'),[]);return h(Button,{onClick:()=>setValue(value+1)},'old:'+value)}";
        let identity = super::HotReloadIdentity {
            owner: "shell",
            module: "src/App.jsx",
            export: "App",
            signature: "state,effect",
        };
        let mut runtime =
            super::JsxRuntime::new("globalThis.log=[];function App(){return null}", None).unwrap();
        runtime
            .register_surface_entry_with_identity("hot", source, &identity)
            .unwrap();
        runtime.select_surface("hot").unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();

        let compatible = "function App(){const [value,setValue]=useState(0);useEffect(()=>()=>log.push('cleanup'),[]);return h(Button,{onClick:()=>setValue(value+1)},'new:'+value)}";
        assert!(
            runtime
                .hot_replace_surface_entry("hot", compatible, &identity)
                .unwrap()
        );
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "new:1");

        assert!(
            runtime
                .hot_replace_surface_entry("hot", "function App( {", &identity)
                .is_err()
        );
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "new:1");

        let incompatible = super::HotReloadIdentity {
            signature: "state",
            ..identity
        };
        assert!(
            !runtime
                .hot_replace_surface_entry(
                    "hot",
                    "function App(){const [value]=useState(9);return h(Text,null,'reset:'+value)}",
                    &incompatible
                )
                .unwrap()
        );
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "reset:9");
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            ["cleanup"]
        );
    }

    #[test]
    fn allocation_triggered_gc_runs_without_foreground_task_servicing() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        let before = runtime.engine.diagnostics().gc_collections;
        // Ordinary short-lived allocation pressure, without forced collection,
        // snapshots or pumping platform tasks.
        for _ in 0..16 {
            runtime.eval("globalThis.gcFixture=Array.from({length:16384},(_,index)=>({index,value:'row'}))").unwrap();
        }
        assert!(runtime.engine.diagnostics().gc_collections > before);
        assert_eq!(
            runtime.eval_json::<u64>("gcFixture[16383].index").unwrap(),
            16383
        );
        runtime.eval("gcFixture=null").unwrap();
        assert_eq!(runtime.eval_json::<u64>("6*7").unwrap(), 42);
    }

    #[test]
    fn diagnostic_platform_tasks_are_bounded_and_reject_pending_transactions() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
        runtime.begin_transaction().unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        runtime.finish_transaction(false).unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
        assert_eq!(runtime.eval_json::<u64>("6*7").unwrap(), 42);
        runtime.invalidated = true;
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
    }

    #[test]
    fn diagnostic_platform_tasks_recheck_readiness_before_task_admission() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime.eval("globalThis.readinessProbes=0; const originalReadiness=__nickelPlatformMaintenanceReady; __nickelPlatformMaintenanceReady=()=>{ if (++readinessProbes === 2) __nickelRender(); return originalReadiness(); }").unwrap();
        // Simulate readiness changing between the initial batch check and
        // admission. This must reject even when the native task queue is empty.
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        assert_eq!(runtime.eval_json::<u64>("readinessProbes").unwrap(), 2);
        runtime.finish_patch_render(false).unwrap();
        runtime
            .eval("__nickelPlatformMaintenanceReady=originalReadiness")
            .unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
    }

    #[test]
    fn maintenance_activity_coalesces_and_empty_service_returns_to_idle() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        assert!(runtime.platform_maintenance_pending());
        for _ in 0..16 {
            if runtime.service_platform_tasks().unwrap() == 0 {
                break;
            }
        }
        assert!(!runtime.platform_maintenance_pending());
        let before = runtime.engine.diagnostics().invocations;
        for _ in 0..512 {
            assert!(!runtime.platform_maintenance_pending());
        }
        assert_eq!(runtime.engine.diagnostics().invocations, before);
        runtime.eval("void 0").unwrap();
        runtime.eval("void 0").unwrap();
        assert!(runtime.platform_maintenance_pending());
        runtime.begin_transaction().unwrap();
        assert!(!runtime.platform_maintenance_pending());
        runtime.finish_transaction(false).unwrap();
        assert!(runtime.platform_maintenance_pending());
    }

    #[test]
    fn platform_maintenance_errors_and_deadlines_do_not_checkpoint_or_poison_isolate() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime
            .eval("globalThis.savedFailureReadiness=__nickelPlatformMaintenanceReady")
            .unwrap();
        for (body, expected) in [
            ("throw Error('readiness failed')", "exception"),
            ("while(true){}", "deadline exceeded"),
        ] {
            runtime
                .eval(&format!("__nickelPlatformMaintenanceReady=()=>{{{body}}}"))
                .unwrap();
            let before = runtime.engine.diagnostics();
            let error = runtime.service_platform_tasks().unwrap_err();
            assert!(error.contains(expected), "{error}");
            let after = runtime.engine.diagnostics();
            assert_eq!(after.microtask_checkpoints, before.microtask_checkpoints);
            assert_eq!(
                after.platform_tasks_serviced,
                before.platform_tasks_serviced
            );
            if expected == "deadline exceeded" {
                assert_eq!(
                    after.deadline_terminations,
                    before.deadline_terminations + 1
                );
            }
            runtime
                .eval("__nickelPlatformMaintenanceReady=savedFailureReadiness")
                .unwrap();
            assert!(runtime.service_platform_tasks().unwrap() <= 8);
            assert_eq!(runtime.eval_json::<u64>("6 * 7").unwrap(), 42);
        }
    }

    #[test]
    fn serviced_platform_tasks_checkpoint_microtasks_only_after_admitted_work() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime.eval("globalThis.savedTaskReadiness=__nickelPlatformMaintenanceReady; globalThis.taskProbeQueued=true; __nickelPlatformMaintenanceReady=()=>{if(!taskProbeQueued){taskProbeQueued=true;Promise.resolve().then(()=>{globalThis.taskMicrotaskRan=true})}return savedTaskReadiness()}; globalThis.taskMicrotaskObserved=()=>taskMicrotaskRan").unwrap();
        let mut serviced = 0;
        for _ in 0..128 {
            runtime.eval("globalThis.taskMicrotaskRan=false;globalThis.taskProbeQueued=false;globalThis.taskPressure=Array.from({length:16384},(_,index)=>({index,label:'task-pressure-'+index}));globalThis.taskPressure=null").unwrap();
            let before = runtime.engine.diagnostics().microtask_checkpoints;
            let count = runtime.service_platform_tasks().unwrap();
            serviced += count;
            assert_eq!(
                runtime.engine.diagnostics().microtask_checkpoints - before,
                u64::from(count > 0)
            );
            assert_eq!(
                runtime
                    .engine
                    .call_global_bool_without_microtasks("taskMicrotaskObserved", &[])
                    .unwrap(),
                count > 0
            );
            // Drain any intentionally untouched empty-batch microtask before
            // resetting the next sample's marker.
            runtime.eval("void 0").unwrap();
        }
        assert!(
            serviced > 0,
            "ordinary allocation pressure must exercise native foreground work"
        );
    }

    #[test]
    fn diagnostic_platform_tasks_validate_readiness_at_batch_exit() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime.eval("globalThis.exitReadinessProbes=0; const savedExitReadiness=__nickelPlatformMaintenanceReady; __nickelPlatformMaintenanceReady=()=>{if(++exitReadinessProbes===3)__nickelRender();return savedExitReadiness();}").unwrap();
        let checkpoints = runtime.engine.diagnostics().microtask_checkpoints;
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        assert_eq!(
            runtime.engine.diagnostics().microtask_checkpoints,
            checkpoints
        );
        assert_eq!(runtime.eval_json::<u64>("exitReadinessProbes").unwrap(), 3);
        runtime.finish_patch_render(false).unwrap();
        runtime
            .eval("__nickelPlatformMaintenanceReady=savedExitReadiness")
            .unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
    }

    #[test]
    fn diagnostic_platform_tasks_reject_uncheckpointed_render_and_event() {
        let mut runtime = super::JsxRuntime::new(
            "function App(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(count+1)},String(count))}",
            None,
        ).unwrap();
        let initial: Value = runtime.eval_json("__nickelRender()").unwrap();
        assert!(runtime.checkpoint.is_none());
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        runtime.finish_patch_render(true).unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);

        let candidate = runtime
            .dispatch_patched("__nickelDispatchBatchPatched([[0,null]])")
            .unwrap();
        assert!(matches!(candidate, super::ScheduledPatch::Patched { .. }));
        assert!(runtime.checkpoint.is_none());
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        runtime.finish_patch_render(false).unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
        // An unchanged event has no render candidate, but still awaits its
        // owner's acceptance or rollback.
        assert!(matches!(
            runtime
                .dispatch_patched("__nickelDispatchBatchPatched([])")
                .unwrap(),
            super::ScheduledPatch::Unchanged
        ));
        assert!(runtime.diagnostic_pump_platform_tasks().is_err());
        runtime.finish_event(false).unwrap();
        assert!(runtime.diagnostic_pump_platform_tasks().unwrap() <= 8);
        let restored = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(restored, initial);
    }

    #[test]
    fn diagnostic_heap_snapshot_is_complete_and_runtime_remains_callable() {
        let mut runtime = super::JsxRuntime::new(
            "const sentinel={label:'nickel diagnostic heap sentinel'}; function App(){return h(Text,null,sentinel.label)}",
            None,
        ).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let snapshot = runtime.diagnostic_heap_snapshot().unwrap();
        let snapshot: Value = serde_json::from_slice(&snapshot).unwrap();
        assert!(snapshot["snapshot"]["meta"]["node_fields"].is_array());
        assert!(!snapshot["nodes"].as_array().unwrap().is_empty());
        assert!(
            snapshot["strings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "nickel diagnostic heap sentinel")
        );
        let rendered = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(rendered["children"][0], "nickel diagnostic heap sentinel");
    }

    #[test]
    fn virtual_rows_scope_unkeyed_descendant_state_by_logical_key() {
        let source = r#"
            const items = ['a','b','c'];
            function StatefulRow({item}) {
                const [initial] = useState(item);
                return h(Text,null,item+':'+initial);
            }
            function App() { return h(VirtualColumn,{
                items,itemKey:String,itemHeight:24,
                renderItem:item=>h(StatefulRow,{item})
            }); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let mut tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        for (start, end, expected) in [(0, 2, vec!["a:a", "b:b"]), (1, 3, vec!["b:b", "c:c"])] {
            let action = tree["action"].as_u64().unwrap();
            tree = runtime.render(&format!(
                "__nickelDispatch({action},JSON.stringify({{source:1,start:{start},end:{end}}}))"
            ), |node| Ok(node.clone())).unwrap();
            let labels: Vec<_> = tree["children"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["children"][0]["children"][0].as_str().unwrap())
                .collect();
            assert_eq!(labels, expected);
        }
    }

    #[test]
    fn retired_memoized_native_rows_receive_fresh_handlers_when_reinserted() {
        let source = "function App(){const [show,setShow]=useState(true);const row=useMemo(()=>h(Button,{key:'row',onClick:()=>{}},'Row'),[]);return h(Column,null,h(Button,{key:'toggle',onClick:()=>setShow(v=>!v)},'Toggle'),show?row:null);}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let toggle = initial["children"][0]["action"].as_u64().unwrap();
        let retired = initial["children"][1]["action"].as_u64().unwrap();
        let slot = initial["children"][1]["__handlerSlots"]["action"].to_string();
        for _ in 0..2 {
            runtime
                .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{toggle},null]])"))
                .unwrap();
            runtime.finish_patch_render(true).unwrap();
            runtime.finish_event(true).unwrap();
        }
        let replacement = runtime
            .eval_json::<Option<u64>>(&format!("__handlerSlots.get({slot})??null"))
            .unwrap();
        assert!(
            replacement.is_some_and(|action| action > retired),
            "memoized native output must not resurrect a retired callback"
        );
    }

    #[test]
    fn direct_json_calls_preserve_data_microtasks_and_error_recovery() {
        let mut runtime = super::JsxRuntime::new(r#"
            function App(){ return h(Text,null,'ready'); }
            let completed = false;
            let setterCalls = 0;
            Object.defineProperty(Object.prototype, 'payload', {set(){setterCalls++}, configurable:true});
            function echo(value) {
                Promise.resolve().then(()=>completed=true);
                return JSON.stringify(value);
            }
            function observed(){return JSON.stringify({completed,setterCalls});}
            function broken(){return 'not-json';}
            function throws(){throw Error('direct failure');}
            function spins(){while(true){}}
            function coercionSpins(){return {toString(){while(true){}}};}
        "#, None).unwrap();
        let value = serde_json::json!({"payload":[null,true,42,"quoted\"\\日本語"],"__proto__":{"data":true}});
        let before = runtime.engine.diagnostics().script_compilations;
        let echoed: serde_json::Value = runtime
            .call_native_json("echo", std::slice::from_ref(&value))
            .unwrap();
        assert_eq!(echoed, value);
        let observed: serde_json::Value = runtime.call_native_json("observed", &[]).unwrap();
        assert_eq!(
            observed,
            serde_json::json!({"completed":true,"setterCalls":0})
        );
        assert!(
            runtime
                .call_native_json::<serde_json::Value>("broken", &[])
                .is_err()
        );
        assert!(
            runtime
                .call_native_json::<serde_json::Value>("throws", &[])
                .unwrap_err()
                .contains("direct failure")
        );
        let deadline_error = runtime
            .call_native_json::<serde_json::Value>("spins", &[])
            .unwrap_err();
        assert!(deadline_error.contains("deadline"), "{deadline_error}");
        let coercion_error = runtime
            .call_native_json::<serde_json::Value>("coercionSpins", &[])
            .unwrap_err();
        assert!(coercion_error.contains("deadline"), "{coercion_error}");
        assert_eq!(
            runtime
                .call_native_json::<serde_json::Value>("echo", std::slice::from_ref(&value))
                .unwrap(),
            value
        );
        assert_eq!(runtime.engine.diagnostics().script_compilations, before);
        assert_eq!(runtime.engine.diagnostics().deadline_terminations, 2);
        runtime.invalidated = true;
        assert!(
            runtime
                .call_native_json::<serde_json::Value>("echo", &[])
                .is_err()
        );
    }

    #[test]
    fn direct_patched_dispatch_matches_expression_oracle_without_compilation() {
        let source = r#"
            function App(){const [value,setValue]=useState('initial');
                return h(Column,null,h(Button,{onClick:setValue},'Set'),h(Text,null,value));}
        "#;
        let mut direct = super::JsxRuntime::new(source, None).unwrap();
        let mut oracle = super::JsxRuntime::new(source, None).unwrap();
        let first = direct
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(
            oracle
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap(),
            first
        );
        let action = first["children"][0]["action"].as_u64().unwrap();
        let before = direct.engine.diagnostics().script_compilations;
        for (value, accepted) in [
            ("quoted\"\\日本語", true),
            ("reject", false),
            ("recovered", true),
        ] {
            direct.begin_transaction().unwrap();
            oracle.begin_transaction().unwrap();
            let events = vec![serde_json::json!([action, value])];
            let actual = direct
                .dispatch_batch_patched(events.clone(), false)
                .unwrap();
            let expected = oracle
                .dispatch_patched(&format!(
                    "__nickelDispatchBatchPatched({},false)",
                    serde_json::Value::Array(events)
                ))
                .unwrap();
            assert_eq!(actual, expected);
            for runtime in [&mut direct, &mut oracle] {
                runtime.finish_patch_render(accepted).unwrap();
                runtime.finish_event(accepted).unwrap();
                runtime.finish_transaction(accepted).unwrap();
                assert!(runtime.take_effects().unwrap().is_empty());
                runtime.runtime_diagnostics().unwrap();
                runtime.boundary_diagnostics().unwrap();
            }
        }
        assert_eq!(direct.engine.diagnostics().script_compilations, before);
        let final_tree = direct
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(final_tree["children"][1]["children"][0], "recovered");
        assert_eq!(
            final_tree,
            oracle
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap()
        );
    }

    #[test]
    fn native_bookkeeping_calls_do_not_compile_javascript() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime
            .eval(
                r#"
            globalThis.nativeCalls = [];
            __nickelSelectSurface = (...args) => nativeCalls.push(['surface', ...args]);
            __nickelDropSurface = (...args) => nativeCalls.push(['drop', ...args]);
            __nickelSetDiagnosticOwner = (...args) => nativeCalls.push(['owner', ...args]);
            __nickelReportHostProfile = (...args) => nativeCalls.push(['profile', ...args]);
            __nickelReportTypedPatchApply = (...args) => nativeCalls.push(['patch', ...args]);
        "#,
            )
            .unwrap();
        let before = runtime.engine.diagnostics().script_compilations;
        for sample in 0..32 {
            runtime.select_surface("surface\"\\日本語").unwrap();
            runtime.set_diagnostic_owner("owner").unwrap();
            runtime
                .report_host_profile("typed-patch", sample, sample as usize * 31)
                .unwrap();
            runtime
                .report_typed_patch_apply(sample, sample % 2 == 0)
                .unwrap();
            runtime.drop_surface("surface\"\\日本語").unwrap();
        }
        assert_eq!(runtime.engine.diagnostics().script_compilations, before);
        let calls: Vec<serde_json::Value> =
            runtime.eval_json("JSON.stringify(nativeCalls)").unwrap();
        assert_eq!(calls.len(), 160);
        for (sample, calls) in calls.as_chunks::<5>().0.iter().enumerate() {
            assert_eq!(
                calls[0],
                serde_json::json!(["surface", "surface\"\\日本語"])
            );
            assert_eq!(calls[1], serde_json::json!(["owner", "owner"]));
            assert_eq!(
                calls[2],
                serde_json::json!(["profile", "typed-patch", sample, sample * 31])
            );
            assert_eq!(
                calls[3],
                serde_json::json!(["patch", sample, sample % 2 == 0])
            );
            assert_eq!(calls[4], serde_json::json!(["drop", "surface\"\\日本語"]));
        }
        runtime.invalidated = true;
        assert!(runtime.select_surface("stale").is_err());
        assert!(runtime.drop_surface("stale").is_err());
        assert!(runtime.set_diagnostic_owner("stale").is_err());
        assert!(runtime.report_host_profile("stale", 0, 0).is_err());
        assert!(runtime.report_typed_patch_apply(0, true).is_err());
    }

    #[test]
    fn incremental_row_retirement_releases_javascript_handler_slots() {
        let source = "function App(){const [start,setStart]=useState(0);return h(Column,null,h(Button,{key:'next',onClick:()=>setStart(n=>n+1)},'Next'),...Array.from({length:5},(_,n)=>h(Button,{key:String(start+n),onClick:()=>{}},String(start+n))));}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let next = initial["children"][0]["action"].as_u64().unwrap();
        let retired = initial["children"][1]["action"].as_u64().unwrap();
        runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{next},null]])"))
            .unwrap();
        runtime.finish_patch_render(false).unwrap();
        runtime.finish_event(false).unwrap();
        assert!(
            runtime
                .eval_json::<bool>(&format!("__handlers.has({retired})"))
                .unwrap(),
            "rejected retirement must restore callbacks"
        );
        for _ in 0..128 {
            runtime
                .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{next},null]])"))
                .unwrap();
            runtime.finish_patch_render(true).unwrap();
            runtime.finish_event(true).unwrap();
        }
        let counts = runtime.runtime_diagnostics().unwrap()["retained"].clone();
        assert_eq!(
            counts["handlerSlots"], 6,
            "retired rows must not remain in the JavaScript slot map"
        );
        assert_eq!(
            counts["handlerEntries"], 6,
            "incremental snapshots must copy live handlers, not all historically visited rows"
        );
        assert_eq!(counts["previousHandlerEntries"], 6);
        assert!(
            !runtime
                .eval_json::<bool>(&format!("__handlers.has({retired})"))
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>(&format!("__handlers.has({next})"))
                .unwrap(),
            "surviving callbacks retain their identities"
        );
    }

    #[test]
    fn retained_diagnostics_count_selected_surfaces_once_and_retire_them() {
        let mut runtime = super::JsxRuntime::new(
            "function App(){const [n,setN]=useState(0);return h(Button,{onClick:()=>setN(n+1)},String(n));}",
            None,
        ).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        for expected in [1, 2, 2] {
            if expected == 2 {
                runtime.select_surface("other").unwrap();
                runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            }
            let counts = runtime.runtime_diagnostics().unwrap()["retained"].clone();
            for field in [
                "surfaces",
                "hookComponents",
                "hookSlots",
                "componentRecords",
                "handlerEntries",
                "handlerSlots",
            ] {
                assert_eq!(counts[field], expected, "{field}");
            }
        }
        runtime.drop_surface("other").unwrap();
        assert_eq!(
            runtime.runtime_diagnostics().unwrap()["retained"]["surfaces"],
            1
        );
        runtime.select_surface("default").unwrap();
        assert_eq!(
            runtime.runtime_diagnostics().unwrap()["retained"]["handlerSlots"],
            1
        );
        runtime.drop_surface("default").unwrap();
        let counts = runtime.runtime_diagnostics().unwrap()["retained"].clone();
        assert!(counts.as_object().unwrap().values().all(|count| count == 0));
    }

    #[test]
    fn runtime_diagnostics_are_bounded_and_include_owner_surface_and_stack() {
        let source = "function Child(){throw Error('boom')} function App(){return h(ErrorBoundary,{fallback:h(Text,null,'fallback')},h(Child))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.set_diagnostic_owner("diagnostic.package").unwrap();
        runtime
            .set_surface_store("mount-7", &serde_json::json!({"id":"settings"}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let failures = runtime.boundary_diagnostics().unwrap();
        assert_eq!(failures[0]["package"], "diagnostic.package");
        assert_eq!(failures[0]["mount"], "mount-7");
        assert!(failures[0]["stack"].as_array().unwrap().len() >= 2);
        for _ in 0..40 {
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        }
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert!(diagnostics["counters"]["renders"].as_u64().unwrap() >= 41);
        assert_eq!(diagnostics["reasons"].as_array().unwrap().len(), 32);
        assert!(diagnostics["storeChanges"].as_array().unwrap().len() <= 32);
        assert!(diagnostics["nativeMutations"].as_array().unwrap().len() <= 32);
    }

    #[test]
    fn runtime_store_and_native_mutation_diagnostics_are_content_free_and_bounded() {
        let source = "function App(){const locale=useLocale();const [count,setCount]=useState(0);return h(Button,{onClick:()=>setCount(value=>value+1)},String(count)+locale.tag)}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = initial["action"].as_u64().unwrap();
        for index in 0..40 {
            let tag = if index % 2 == 0 { "en-US" } else { "de-DE" };
            runtime
                .set_locale_store(&serde_json::json!({"known":true,"tag":tag,"direction":"ltr"}))
                .unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();

            let outcome = runtime
                .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
                .unwrap();
            assert!(matches!(outcome, super::ScheduledPatch::Patched { .. }));
            runtime.finish_patch_render(true).unwrap();
            runtime.finish_event(true).unwrap();
        }
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(diagnostics["storeChanges"].as_array().unwrap().len(), 32);
        assert_eq!(diagnostics["nativeMutations"].as_array().unwrap().len(), 32);
        assert!(diagnostics["counters"]["storeChanges"].as_u64().unwrap() >= 40);
        assert_eq!(diagnostics["counters"]["nativeMutations"], 40);
        assert!(diagnostics["storeChanges"].to_string().len() < 4096);
        assert!(diagnostics["nativeMutations"].to_string().len() < 8192);
    }

    #[test]
    fn per_mount_profile_counts_execution_patches_commits_and_transport() {
        let source = "function App(){const [value,setValue]=useState(0);return h(Button,{onClick:()=>setValue(value+1)},String(value))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = initial["action"].as_u64().unwrap();
        let outcome = runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        assert!(matches!(outcome, super::ScheduledPatch::Patched { .. }));
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
        runtime.report_typed_patch_apply(37, true).unwrap();
        runtime.report_typed_patch_apply(11, false).unwrap();
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        let profile = &diagnostics["profiles"][0];
        assert_eq!(profile["surface"], "default");
        assert_eq!(profile["renders"], 2);
        assert_eq!(profile["componentExecutions"], 2);
        assert_eq!(profile["patches"], 1);
        assert_eq!(profile["patchOperations"], 1);
        assert!(profile["patchGenerationMillis"].is_u64());
        assert_eq!(profile["lifecycleCommits"], 2);
        assert!(profile["coldTreeTransportBytes"].as_u64().unwrap() > 0);
        assert!(profile["patchEnvelopeTransportBytes"].as_u64().unwrap() > 0);
        assert!(profile["nativeValidationMicros"].is_u64());
        assert_eq!(profile["typedPatchApplyAttempts"], 2);
        assert_eq!(profile["typedPatchApplyRejections"], 1);
        assert_eq!(profile["typedPatchApplyMicros"], 48);
        assert_eq!(profile["timingPrecision"], "wall-clock-milliseconds");
        assert_eq!(profile["hostTimingPrecision"], "wall-clock-microseconds");
        let engine = &diagnostics["engine"];
        assert!(engine["invocations"].as_u64().unwrap() > 0);
        assert_eq!(engine["failedInvocations"], 0);
        assert_eq!(engine["deadlineTerminations"], 0);
        assert_eq!(
            engine["microtaskCheckpoints"], engine["invocations"],
            "every successful synchronous V8 turn owns one explicit checkpoint"
        );
        assert!(engine["javascriptMicros"].is_u64());
        assert!(engine["bridgeMicros"].is_u64());
        assert!(engine["microtaskMicros"].is_u64());
        assert!(engine["gcCollections"].is_u64());
        assert!(engine["gcMicros"].is_u64());
        assert!(engine["externalMemoryBytes"].is_u64());
        assert!(engine["mallocedMemoryBytes"].is_u64());
        assert!(engine["usedGlobalHandlesBytes"].as_u64().unwrap() > 0);
        assert_eq!(engine["nativeContexts"], 1);
        assert_eq!(engine["detachedContexts"], 0);
        assert!(engine["usedHeapBytes"].as_u64().unwrap() > 0);
        assert!(
            engine["peakUsedHeapBytes"].as_u64().unwrap()
                >= engine["usedHeapBytes"].as_u64().unwrap()
        );
        assert!(
            engine["heapLimitBytes"].as_u64().unwrap()
                >= engine["totalHeapBytes"].as_u64().unwrap()
        );

        runtime.select_surface("settings").unwrap();
        runtime
            .set_surface_store("mount-settings", &serde_json::json!({"id":"settings"}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(diagnostics["profiles"].as_array().unwrap().len(), 2);
        assert_eq!(diagnostics["engine"]["nativeContexts"], 1);
        assert_eq!(diagnostics["engine"]["detachedContexts"], 0);
        assert!(diagnostics["profiles"].as_array().unwrap().iter().any(
            |profile| profile["surface"] == "settings" && profile["mount"] == "mount-settings"
        ));
    }

    #[test]
    fn keyed_insertion_keeps_trailing_sibling_out_of_transport() {
        let source = r#"
            function Item({name}) { return h(Text,{key:name},name); }
            function App() {
                const [extra,setExtra]=useState(false);
                return h(Window,{},h(Button,{key:'toggle',onClick:()=>setExtra(true)},'toggle'),
                    ...(extra?[h(Item,{key:'left',name:'left'})]:[]),
                    h(Item,{key:'right',name:'right'}));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let right_id = initial["children"][1]["__nativeId"]
            .as_str()
            .unwrap()
            .to_owned();
        let outcome = runtime
            .dispatch_patched("__nickelDispatchBatchPatched([[0,null]])")
            .unwrap();
        let super::ScheduledPatch::Patched { patch, .. } = outcome else {
            panic!()
        };
        assert!(patch.operations.iter().any(|op| matches!(
            op,
            super::NativePatchOperation::InsertChild { index: 1, .. }
        )));
        assert!(!patch.operations.iter().any(|op| matches!(op,
            super::NativePatchOperation::ReplaceSubtree { target, .. } if target == &right_id)));
        assert_eq!(
            patch.operations.len(),
            1,
            "unchanged trailing child needs no operation"
        );
        assert_eq!(patch.counters.nodes_mutated, 1);
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(diagnostics["counters"]["nativeMutations"], 1);
        assert_eq!(diagnostics["nativeMutations"][0]["count"], 1);
        assert_eq!(diagnostics["nativeMutations"][0]["kinds"]["insertChild"], 1);
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
    }

    #[test]
    fn leaf_patch_keeps_ancestor_native_boundary_current_for_later_unmount() {
        let source = r#"
            function Counter({name}) {
                const [count,setCount]=useState(0);
                return h(Button,{key:name,onClick:()=>setCount(count+1)},name+':'+count);
            }
            function App() {
                const [showFirst,setShowFirst]=useState(true);
                return h(Window,{},
                    h(Button,{key:'toggle',onClick:()=>setShowFirst(value=>!value)},'toggle'),
                    ...(showFirst?[h(Counter,{key:'first',name:'first'})]:[]),
                    h(Counter,{key:'second',name:'second'}));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();

        let mut patches = Vec::new();
        for action in [2, 0, 0] {
            let super::ScheduledPatch::Patched { patch, .. } = runtime
                .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
                .unwrap()
            else {
                panic!("state update must produce a typed patch");
            };
            patches.push(serde_json::to_string(&patch).unwrap());
            runtime.finish_patch_render(true).unwrap();
            runtime.finish_event(true).unwrap();
        }

        assert!(!patches[1].contains("first:0"));
        assert!(patches[2].contains("first:0"));
        assert_eq!(
            runtime
                .eval_json::<String>(
                    "JSON.stringify(Array.from(__componentRecords.values()).find(record=>record.kind.name==='Counter'&&record.declaration.key==='second').output.children[0])",
                )
                .unwrap(),
            "second:1"
        );
    }

    #[test]
    fn large_keyed_insert_reuses_admitted_siblings_without_quadratic_visits() {
        let source = r#"
            globalThis.itemRuns=0;
            function Item({item}) { itemRuns++; return h(Text,{key:'native-'+item},String(item)); }
            function App() {
                const [items,setItems]=useState(Array.from({length:200},(_,index)=>index));
                return h(Window,{},
                    h(Button,{key:'insert',onClick:()=>setItems(current=>[
                        ...current.slice(0,100),200,...current.slice(100)
                    ])},'insert'),
                    h(Column,{key:'items'},...items.map(item=>h(Item,{key:'component-'+item,item}))));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let materialized_before =
            runtime.runtime_diagnostics().unwrap()["counters"]["nativeNodesMaterialized"]
                .as_u64()
                .unwrap();
        let action = initial["children"][0]["action"].as_u64().unwrap();
        let outcome = runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        let super::ScheduledPatch::Patched { patch, .. } = outcome else {
            panic!("keyed insertion must patch");
        };
        assert_eq!(patch.operations.len(), 1);
        assert!(matches!(
            patch.operations[0],
            super::NativePatchOperation::InsertChild { index: 100, .. }
        ));
        assert_eq!(patch.counters.nodes_visited, 2);
        assert_eq!(patch.counters.nodes_mutated, 1);
        assert_eq!(patch.counters.local_materializations, 0);
        assert_eq!(patch.counters.expansion_nodes, 0);
        assert_eq!(patch.counters.tree_bytes, 0);
        assert_eq!(
            runtime
                .eval_json::<u64>("JSON.stringify(globalThis.itemRuns)")
                .unwrap(),
            201,
            "only the inserted component executes during reconciliation"
        );
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(
            diagnostics["counters"]["nativeNodesMaterialized"]
                .as_u64()
                .unwrap()
                - materialized_before,
            4,
            "only the new root wrappers and inserted leaf are materialized"
        );
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
    }

    #[test]
    fn patched_dispatch_transports_only_the_changed_leaf() {
        let source = r#"
            globalThis.runs={leaf:0,sibling:0};
            function Sibling() { runs.sibling++; return h(Column,{},...Array.from({length:64},(_,index)=>h(Text,{key:String(index)},'stable'.repeat(20)))); }
            function Leaf() { runs.leaf++; const [value,setValue]=useState(0); return h(Button,{onClick:()=>setValue(value+1)},String(value)); }
            function App() { return h(Window,{},h(Sibling),h(Leaf)); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = initial["children"][1]["action"].as_u64().unwrap();
        let materialized_before =
            runtime.runtime_diagnostics().unwrap()["counters"]["nativeNodesMaterialized"]
                .as_u64()
                .unwrap();
        let outcome = runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        let super::ScheduledPatch::Patched {
            patch,
            transport_bytes,
            ..
        } = outcome
        else {
            panic!("leaf update must patch");
        };
        assert_eq!(patch.operations.len(), 1);
        let super::NativePatchOperation::SetPrimitive {
            target,
            property,
            value,
        } = &patch.operations[0]
        else {
            panic!("text-only leaf update must use setPrimitive");
        };
        assert_eq!(
            target,
            initial["children"][1]["__nativeId"].as_str().unwrap()
        );
        assert_eq!(property, "children");
        assert_eq!(value[0], "1");
        assert_eq!(patch.counters.nodes_visited, 1);
        assert_eq!(patch.counters.local_materializations, 0);
        assert_eq!(patch.counters.expansion_nodes, 0);
        assert_eq!(patch.counters.tree_bytes, 0);
        assert!(transport_bytes < serde_json::to_vec(&initial).unwrap().len());
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(
            diagnostics["counters"]["nativeNodesMaterialized"]
                .as_u64()
                .unwrap()
                - materialized_before,
            1,
            "the dirty leaf is the only native node materialized"
        );
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"leaf":2,"sibling":1}),
            "the clean sibling is neither executed nor traversed for native output"
        );
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
    }

    #[test]
    fn patched_dispatch_replaces_node_when_a_conditional_handler_appears() {
        let source = r#"
            function App() {
                const [enabled,setEnabled]=useState(false);
                return h(Window,{onSubmit:enabled?()=>{}:undefined},
                    h(Button,{onClick:()=>setEnabled(value=>!value)},'enable'));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = initial["children"][0]["action"].as_u64().unwrap();
        let outcome = runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        let super::ScheduledPatch::Patched { patch, .. } = outcome else {
            panic!("conditional handler update must stay typed");
        };
        assert_eq!(patch.operations.len(), 1);
        assert!(matches!(
            &patch.operations[0],
            super::NativePatchOperation::ReplaceSubtree { target, node }
                if target == "root" && node["submitAction"].is_number()
        ));
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
        let retired = runtime
            .eval_json::<u64>("__handlerSlots.get('root:submitAction')")
            .unwrap();
        runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
        assert!(
            !runtime
                .eval_json::<bool>(&format!("__handlers.has({retired})"))
                .unwrap()
        );
        assert_eq!(
            runtime.runtime_diagnostics().unwrap()["retained"]["handlerSlots"],
            1
        );
        runtime
            .dispatch_patched(&format!("__nickelDispatchBatchPatched([[{action},null]])"))
            .unwrap();
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
        assert!(
            runtime
                .eval_json::<u64>("__handlerSlots.get('root:submitAction')")
                .unwrap()
                > retired,
            "reintroduced root callbacks must not reuse retired action identities"
        );
    }

    #[test]
    fn native_and_handler_slot_ids_survive_keyed_insertion() {
        let source = r#"
            function Item({name}) { return h(Button,{key:name,onClick:()=>{}},name); }
            function App() {
                const [extra,setExtra]=useState(false);
                return h(Window,{},h(Button,{key:'toggle',onClick:()=>setExtra(true)},'toggle'),
                    ...(extra?[h(Item,{key:'left',name:'left'})]:[]),h(Item,{key:'right',name:'right'}));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let first = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let updated = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        runtime.finish_event(true).unwrap();
        let super::ScheduledRender::Rendered { value: updated, .. } = updated else {
            panic!("insertion must render");
        };
        let first_right = &first["children"][1];
        let updated_right = &updated["children"][2];
        assert_eq!(first_right["__nativeId"], updated_right["__nativeId"]);
        assert_eq!(
            first_right["__handlerSlots"]["action"],
            updated_right["__handlerSlots"]["action"]
        );
        assert_ne!(first_right["action"], updated_right["action"]);
    }

    #[test]
    fn local_leaf_update_executes_only_the_dirty_component() {
        let source = r#"
            globalThis.runs = {app:0, branch:0, leaf:0, sibling:0};
            function Leaf() { runs.leaf++; const [value,setValue]=useState(0); return h(Button,{onClick:()=>setValue(value+1)},String(value)); }
            function Sibling() { runs.sibling++; return h(Text,{},'stable'); }
            function Branch() { runs.branch++; return h(Column,{},h(Leaf),h(Sibling)); }
            function App() { runs.app++; return h(Window,{},h(Branch)); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let outcome = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert!(matches!(outcome, super::ScheduledRender::Rendered { .. }));
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"branch":1,"leaf":2,"sibling":1})
        );
    }

    #[test]
    fn keyed_siblings_retain_outputs_when_one_owner_changes() {
        let source = r#"
            globalThis.runs = {left:0, right:0};
            function Item({name}) {
                runs[name]++;
                const [value,setValue]=useState(0);
                return h(Button,{onClick:()=>setValue(value+1)},`${name}:${value}`);
            }
            function App() { return h(Window,{},h(Item,{key:'left',name:'left'}),h(Item,{key:'right',name:'right'})); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |_| Ok(()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"left":2,"right":1})
        );
    }

    #[test]
    fn reducer_and_surface_store_updates_execute_only_their_owners() {
        let source = r#"
            globalThis.runs = {app:0, reducer:0, store:0, sibling:0};
            function ReducerLeaf() {
                runs.reducer++;
                const [value,dispatch]=useReducer(value=>value+1,0);
                return h(Button,{onClick:()=>dispatch()},String(value));
            }
            function StoreLeaf() { runs.store++; return h(Text,{},String(useSurface().logicalSize?.width ?? 0)); }
            function Sibling() { runs.sibling++; return h(Text,{},'stable'); }
            function App() { runs.app++; return h(Window,{},h(ReducerLeaf),h(StoreLeaf),h(Sibling)); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .set_surface_store(
                "mount",
                &serde_json::json!({"id":"main","kind":"window","width":640,"height":480}),
            )
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |_| Ok(()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        runtime
            .set_surface_store(
                "mount",
                &serde_json::json!({"id":"main","kind":"window","width":800,"height":480}),
            )
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"reducer":2,"store":2,"sibling":1})
        );
    }

    #[test]
    fn surface_selector_ignores_focus_only_updates_and_retains_size_identity() {
        let source = r#"
            globalThis.runs=0;globalThis.sizes=[];
            function App(){runs++;const size=useSurface(surface=>surface.availableSize);sizes.push(size);return h(Text,null,String(size?.width??0));}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .set_surface_store(
                "mount",
                &serde_json::json!({"availableWidth":640,"availableHeight":480,"focused":false}),
            )
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_surface_store(
                "mount",
                &serde_json::json!({"availableWidth":640,"availableHeight":480,"focused":true}),
            )
            .unwrap();
        assert!(!runtime.reconciliation_requested().unwrap());
        assert_eq!(runtime.eval_json::<u64>("JSON.stringify(runs)").unwrap(), 1);
        assert!(
            runtime
                .eval_json::<bool>(
                    "JSON.stringify(sizes[0] === __surfaceStore.snapshot.availableSize)"
                )
                .unwrap()
        );
    }

    #[test]
    fn retained_mount_transport_restores_surfaces_and_recovers_after_bridge_failure() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        let initial = std::rc::Rc::new(serde_json::json!({"inventory":["original"]}));
        let rejected = std::rc::Rc::new(serde_json::json!({"inventory":["rejected"]}));
        let left = serde_json::json!({"side":"left"});
        let right = serde_json::json!({"side":"right"});
        runtime.select_surface("left").unwrap();
        runtime.set_mount_data(&initial, None, &left).unwrap();
        runtime
            .eval("twinkle.data.inventory[0]='left-local'")
            .unwrap();
        runtime.select_surface("right").unwrap();
        runtime.set_mount_data(&initial, None, &right).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            serde_json::json!({"inventory":["original"],"__componentProps":right})
        );
        runtime.begin_transaction().unwrap();
        runtime.set_mount_data(&rejected, None, &right).unwrap();
        runtime.select_surface("left").unwrap();
        runtime.set_mount_data(&rejected, None, &left).unwrap();
        runtime.finish_transaction(false).unwrap();
        assert!(runtime.mount_transport_identity.is_none());
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            serde_json::json!({"inventory":["original"],"__componentProps":right})
        );
        runtime.select_surface("left").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data.inventory)")
                .unwrap(),
            serde_json::json!(["left-local"])
        );
        runtime.set_mount_data(&initial, None, &left).unwrap();
        runtime.eval("globalThis.savedPublish=__nickelPublishData;__nickelPublishData=()=>{throw Error('publication rejected')}").unwrap();
        assert!(runtime.set_mount_data(&rejected, None, &left).is_err());
        assert!(runtime.mount_transport_identity.is_none());
        runtime.eval("__nickelPublishData=savedPublish").unwrap();
        runtime.set_mount_data(&initial, None, &left).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data.inventory)")
                .unwrap(),
            serde_json::json!(["original"])
        );
    }

    #[test]
    fn surface_work_query_does_not_run_microtasks_and_reports_effect_only_work() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.eval("globalThis.originalWorkQuery=__nickelSurfaceWorkPending;__nickelSurfaceWorkPending=id=>{Promise.resolve().then(()=>twinkle.request('show-launcher'));return originalWorkQuery(id)}").unwrap();
        let before = runtime.engine.diagnostics().microtask_checkpoints;
        assert!(!runtime.surface_work_pending("default").unwrap());
        assert_eq!(runtime.engine.diagnostics().microtask_checkpoints, before);
        // An ordinary call, not the metadata query, advances queued work.
        runtime
            .eval("__nickelSurfaceWorkPending=originalWorkQuery")
            .unwrap();
        assert!(runtime.surface_work_pending("default").unwrap());
        assert!(!runtime.reconciliation_requested().unwrap());
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![Value::String("show-launcher".into())]
        );
        assert!(!runtime.surface_work_pending("default").unwrap());
    }

    #[test]
    fn inactive_surface_reducer_failure_targets_its_own_boundary() {
        let mut runtime = super::JsxRuntime::new(
            "globalThis.saved={};function Child(){const [value,dispatch]=useReducer(()=>{throw Error('reducer failure')},0);saved[twinkle.data.name]=dispatch;return h(Text,null,'ready')}function App(){return h(ErrorBoundary,{fallback:h(Text,null,'fallback')},h(Child))}",
            None,
        ).unwrap();
        for name in ["left", "right"] {
            runtime.select_surface(name).unwrap();
            runtime
                .set_data_value(serde_json::json!({"name":name}))
                .unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        }
        runtime.eval("saved.left(null)").unwrap();
        assert!(!runtime.reconciliation_requested().unwrap());
        let diagnostics = runtime.boundary_diagnostics().unwrap();
        assert_eq!(diagnostics.last().unwrap()["surface"], "left");
        assert_eq!(diagnostics.last().unwrap()["phase"], "reducer");
        runtime.select_surface("left").unwrap();
        assert!(runtime.reconciliation_requested().unwrap());
        let fallback = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(fallback.to_string().contains("fallback"));
        runtime.select_surface("right").unwrap();
        let ready = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(ready.to_string().contains("ready"));
    }

    #[test]
    fn retired_component_dispatch_cannot_update_a_reused_path_on_the_same_surface() {
        for hook in ["useState(0)", "useReducer((previous,action)=>action,0)"] {
            let source = format!(
                "function Child(){{const [value,dispatch]={hook};globalThis.latestDispatch=dispatch;return h(Text,null,'value:'+value)}}function App(){{return twinkle.data.show?h(Child):h(Text,null,'hidden')}}"
            );
            let mut runtime = super::JsxRuntime::new(&source, None).unwrap();
            runtime
                .set_data_value(serde_json::json!({"show":true}))
                .unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            runtime
                .eval("globalThis.retiredDispatch=latestDispatch")
                .unwrap();
            runtime
                .set_data_value(serde_json::json!({"show":false}))
                .unwrap();
            let hidden = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert!(hidden.to_string().contains("hidden"));
            runtime.eval("retiredDispatch(88)").unwrap();
            assert!(!runtime.reconciliation_requested().unwrap());
            runtime
                .set_data_value(serde_json::json!({"show":true}))
                .unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            runtime.eval("retiredDispatch(99)").unwrap();
            assert!(!runtime.reconciliation_requested().unwrap());
            runtime.eval("latestDispatch(7)").unwrap();
            assert!(runtime.reconciliation_requested().unwrap());
            let fresh = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert!(fresh.to_string().contains("value:7"));
        }
    }

    #[test]
    fn state_and_reducer_dispatch_target_their_surface_and_reject_retired_lifetimes() {
        for hook in ["useState(0)", "useReducer((previous,action)=>action,0)"] {
            let source = format!(
                "globalThis.saved={{}};function App(){{const [value,setValue]={hook};saved[twinkle.data.name]=setValue;return h(Text,null,twinkle.data.name+':'+value)}}"
            );
            let mut runtime = super::JsxRuntime::new(&source, None).unwrap();
            for name in ["left", "right"] {
                runtime.select_surface(name).unwrap();
                runtime
                    .set_data_value(serde_json::json!({"name":name}))
                    .unwrap();
                runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            }
            runtime.eval("saved.left(7)").unwrap();
            assert!(runtime.surface_work_pending("left").unwrap());
            assert!(!runtime.surface_work_pending("right").unwrap());
            assert!(!runtime.reconciliation_requested().unwrap());
            runtime.select_surface("left").unwrap();
            assert!(runtime.reconciliation_requested().unwrap());
            let rendered = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert!(rendered.to_string().contains("left:7"));
            runtime.begin_transaction().unwrap();
            assert!(runtime.surface_work_pending("left").is_err());
            runtime.eval("saved.left(9)").unwrap();
            runtime.finish_transaction(false).unwrap();
            let restored = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert_eq!(restored, rendered);
            runtime.eval("globalThis.retiredSetter=saved.left").unwrap();
            runtime.drop_surface("left").unwrap();
            assert!(!runtime.surface_work_pending("left").unwrap());
            runtime.select_surface("left").unwrap();
            runtime
                .set_data_value(serde_json::json!({"name":"left"}))
                .unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            runtime.eval("retiredSetter(99)").unwrap();
            assert!(!runtime.reconciliation_requested().unwrap());
            let fresh = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert!(fresh.to_string().contains("left:0"));
        }
    }

    #[test]
    fn unchanged_mount_inventory_transport_is_independent_of_logical_row_count() {
        let mut warmed_counts = Vec::new();
        for count in [100, 1000, 10000] {
            let mut runtime = super::JsxRuntime::new(
                "function App(){return h(Text,null,String(__nickelResource('inventory',{}).images.length))}",
                None,
            )
            .unwrap();
            let base = std::rc::Rc::new(serde_json::json!({"inventory":{"available":true,
                "images":(0..count).map(|id| serde_json::json!({"id":id.to_string(),"label":"original"})).collect::<Vec<_>>()}}));
            let props = serde_json::json!({});
            runtime.set_mount_data(&base, None, &props).unwrap();
            runtime.render("__nickelRender()", |_| Ok(())).unwrap();
            runtime.eval("globalThis.stringifies=0;globalThis.savedStringify=JSON.stringify;JSON.stringify=(...args)=>{stringifies++;return savedStringify(...args)}").unwrap();
            let before = runtime.engine.diagnostics().bridge_values;
            runtime.set_mount_data(&base, None, &props).unwrap();
            let converted = runtime.engine.diagnostics().bridge_values - before;
            warmed_counts.push(converted);
            assert_eq!(converted, 5, "{count} rows converted {converted} values");
            assert_eq!(runtime.eval_json::<u64>("stringifies").unwrap(), 0);
            assert!(!runtime.reconciliation_requested().unwrap());
            // A compatibility read may mutate its own copy, never the retained
            // native source. The next publication still restores it and dirties
            // consumers whose exposed previous value was locally changed.
            runtime
                .eval("twinkle.data.inventory.images[0].label='guest-local'")
                .unwrap();
            runtime.set_mount_data(&base, None, &props).unwrap();
            assert!(runtime.reconciliation_requested().unwrap());
            assert_eq!(
                runtime
                    .eval_json::<String>("JSON.stringify(twinkle.data.inventory.images[0].label)")
                    .unwrap(),
                "original"
            );
        }
        assert!(warmed_counts.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn mount_settings_identity_is_revoked_by_rollback_and_owned_publication() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        let initial = std::rc::Rc::new(serde_json::json!({"inventory":[0]}));
        let mut replacement = std::rc::Rc::new(serde_json::json!({"inventory":[1]}));
        let props = serde_json::json!({"label":"same"});
        runtime.set_mount_data(&initial, None, &props).unwrap();
        let revision = runtime.settings_revision;
        runtime.begin_transaction().unwrap();
        runtime.set_mount_data(&replacement, None, &props).unwrap();
        runtime.finish_transaction(false).unwrap();
        assert!(runtime.mount_settings_identity.is_none());
        runtime.set_mount_data(&replacement, None, &props).unwrap();
        assert_eq!(runtime.settings_revision, revision + 1);
        assert_eq!(
            runtime.settings_data.as_ref().unwrap()["inventory"],
            serde_json::json!([1])
        );

        runtime
            .set_data_value(serde_json::json!({"inventory":[2]}))
            .unwrap();
        assert!(runtime.mount_settings_identity.is_none());
        runtime.set_mount_data(&replacement, None, &props).unwrap();
        assert_eq!(
            runtime.settings_data.as_ref().unwrap()["inventory"],
            serde_json::json!([1])
        );
        let retained = runtime.mount_settings_identity.as_ref().unwrap().0.clone();
        std::rc::Rc::make_mut(&mut replacement)["inventory"] = serde_json::json!([3]);
        assert!(!std::rc::Rc::ptr_eq(&retained, &replacement));
        let revision = runtime.settings_revision;
        runtime.set_mount_data(&replacement, None, &props).unwrap();
        assert_eq!(runtime.settings_revision, revision + 1);
        assert_eq!(
            runtime.settings_data.as_ref().unwrap()["inventory"],
            serde_json::json!([3])
        );
    }

    #[test]
    fn borrowed_mount_projection_matches_owned_data_and_reuses_setting_snapshot() {
        let source = "function App(){return h(Text,null,'ready')}";
        let mut borrowed = super::JsxRuntime::new(source, None).unwrap();
        let mut owned = super::JsxRuntime::new(source, None).unwrap();
        let base = std::rc::Rc::new(serde_json::json!({
            "surface":{"id":"base"}, "__componentProps":{"stale":true},
            "__proto__":{"transported":true},
            "inventory":(0..1000).map(|id| serde_json::json!({"id":id,"label":"row"})).collect::<Vec<_>>()
        }));
        let props = serde_json::json!({"label":"mount"});
        let overlay = serde_json::json!({"id":"override"});
        for surface in [None, Some(&overlay)] {
            let mut expected = base.as_ref().clone();
            expected["__componentProps"] = props.clone();
            if let Some(surface) = surface {
                expected["surface"] = surface.clone();
            }
            owned.set_data_value(expected).unwrap();
            borrowed.set_mount_data(&base, surface, &props).unwrap();
            assert_eq!(
                borrowed
                    .eval_json::<String>("JSON.stringify(JSON.stringify(twinkle.data))")
                    .unwrap(),
                owned
                    .eval_json::<String>("JSON.stringify(JSON.stringify(twinkle.data))")
                    .unwrap()
            );
            assert_eq!(borrowed.settings_revision, owned.settings_revision);
            assert_eq!(borrowed.settings_data, owned.settings_data);
        }
        let retained = borrowed.settings_data.clone().unwrap();
        let revision = borrowed.settings_revision;
        borrowed
            .eval("twinkle.data.inventory[0].label='guest-local'")
            .unwrap();
        borrowed.set_mount_data(&base, None, &props).unwrap();
        assert!(std::rc::Rc::ptr_eq(
            &retained,
            borrowed.settings_data.as_ref().unwrap()
        ));
        assert_eq!(borrowed.settings_revision, revision);
        assert_eq!(
            borrowed
                .eval_json::<String>("JSON.stringify(twinkle.data.inventory[0].label)")
                .unwrap(),
            "row"
        );
        assert_eq!(base["surface"]["id"], "base");
        assert_eq!(base["__componentProps"]["stale"], true);
    }

    #[test]
    fn data_projection_tracks_surface_selection_and_rejected_replacement() {
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ready')}", None).unwrap();
        let projection = |surface: &str, label: &str| {
            serde_json::json!({
                "surface":{"id":surface},
                "__componentProps":{"label":label},
                "inventory":[{"id":"stable","label":label}]
            })
        };
        let left = projection("left", "original-left");
        let right = projection("right", "original-right");
        runtime.select_surface("left").unwrap();
        runtime.set_data_value(left.clone()).unwrap();
        runtime.select_surface("right").unwrap();
        runtime.set_data_value(right.clone()).unwrap();
        runtime.select_surface("left").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            left
        );

        runtime.begin_transaction().unwrap();
        runtime
            .set_data_value(projection("left", "rejected-left"))
            .unwrap();
        runtime.select_surface("right").unwrap();
        runtime
            .set_data_value(projection("right", "rejected-right"))
            .unwrap();
        runtime.finish_transaction(false).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            left
        );
        runtime.select_surface("right").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            right
        );

        runtime.drop_surface("right").unwrap();
        runtime.select_surface("right").unwrap();
        let replacement = projection("right", "replacement");
        runtime.set_data_value(replacement.clone()).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            replacement
        );
        runtime.select_surface("left").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(twinkle.data)")
                .unwrap(),
            left
        );
    }

    #[test]
    fn locale_store_has_stable_fallback_and_normalizes_authoritative_tags() {
        let mut runtime=super::JsxRuntime::new("globalThis.seen=[];function App(){const locale=useLocale();seen.push(locale);return h(Text,null,locale.tag)}",None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(seen[0])")
                .unwrap(),
            serde_json::json!({"generation":0,"tag":"und","direction":"ltr","known":false})
        );
        assert!(
            !runtime
                .set_locale_store(&serde_json::json!({"known":false,"tag":"ar","direction":"rtl"}))
                .unwrap()
        );
        assert!(
            runtime
                .set_locale_store(
                    &serde_json::json!({"known":true,"tag":"ZH-hant-tw","direction":"rtl"})
                )
                .unwrap()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(seen[1])")
                .unwrap(),
            serde_json::json!({"generation":1,"tag":"zh-Hant-TW","direction":"rtl","known":true})
        );
        assert!(
            runtime
                .eval_json::<bool>("Object.isFrozen(seen[1])")
                .unwrap()
        );
        assert!(
            runtime
                .set_locale_store(
                    &serde_json::json!({"known":true,"tag":"en-us","direction":"ltr"})
                )
                .unwrap()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(seen[2].tag)")
                .unwrap(),
            "en-US"
        );
        assert!(
            runtime
                .set_locale_store(
                    &serde_json::json!({"known":true,"tag":"bad_tag","direction":"ltr"})
                )
                .is_err()
        );
    }

    #[test]
    fn memo_skips_props_but_not_store_or_context_updates() {
        let source = r#"
            const Context=createContext('cold');globalThis.runs={app:0,plain:0,custom:0,store:0,context:0};
            function Plain({label}){runs.plain++;return h(Text,null,label)}const MemoPlain=memo(Plain);
            function Custom({label}){runs.custom++;return h(Text,null,label)}const MemoCustom=memo(Custom,()=>true);
            function Store(){runs.store++;return h(Text,null,useLocale().tag)}const MemoStore=memo(Store);
            function Consumer(){runs.context++;return h(Text,null,useContext(Context))}const MemoConsumer=memo(Consumer);
            function App(){runs.app++;const [step,setStep]=useState(0);return h(Window,{},h(Button,{onClick:()=>setStep(v=>v+1)},'next'),h(MemoPlain,{label:step%2?'b':'a'}),h(MemoCustom,{label:String(step)}),h(MemoStore),h(Context.Provider,{value:step%2?'warm':'cold'},h(MemoConsumer)))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |value| {
                Ok(value.clone())
            })
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":2,"plain":2,"custom":1,"store":1,"context":2})
        );
        runtime
            .set_locale_store(&serde_json::json!({"known":true,"tag":"de-DE","direction":"ltr"}))
            .unwrap();
        let incremental = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert_eq!(runtime.eval_json::<u64>("runs.store").unwrap(), 2);
        assert_eq!(incremental["children"][2]["children"][0], "0");
        assert_eq!(incremental["children"][3]["children"][0], "de-DE");
        assert_ne!(initial, incremental);
    }

    #[test]
    fn memo_compare_errors_roll_back_the_accepted_tree() {
        let source = "let fail=true;function Child({value}){return h(Text,null,String(value))}const Memo=memo(Child,()=>{if(fail){fail=false;throw Error('compare failed')}return true});function App(){const [value,setValue]=useState(0);return h(Button,{onClick:()=>setValue(1)},h(Memo,{value}))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert!(
            runtime
                .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |value| Ok(
                    value.clone()
                ))
                .is_err()
        );
        runtime.finish_event(false).unwrap();
        let restored = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert_eq!(initial, restored);
    }

    #[test]
    fn default_memo_incremental_output_matches_cold_render() {
        let source = "function Child({value}){return h(Text,null,String(value))}const Memo=memo(Child);function App(){const [value,setValue]=useState(twinkle.data.initial);return h(Button,{onClick:()=>setValue(1)},h(Memo,{value}))}";
        let mut incremental = super::JsxRuntime::new(source, Some(r#"{"initial":0}"#)).unwrap();
        incremental
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        let updated = match incremental
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |value| {
                Ok(value.clone())
            })
            .unwrap()
        {
            super::ScheduledRender::Rendered { value, .. } => value,
            super::ScheduledRender::Unchanged => panic!("memo update must render"),
        };
        incremental.finish_event(true).unwrap();
        let mut cold = super::JsxRuntime::new(source, Some(r#"{"initial":1}"#)).unwrap();
        let expected = cold
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert_eq!(updated, expected);
    }

    #[test]
    fn theme_store_is_always_readable_versioned_and_structurally_shared() {
        let source = r#"
            globalThis.observed=[];
            function App(){const theme=useTheme();observed.push(theme);return h(Text,null,theme.mode)}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(observed[0])")
                .unwrap(),
            serde_json::json!({"generation":0,"mode":"unknown","accent":null,"accentHue":null,"accentIntensity":null,"reducedMotion":null,"reducedTransparency":null,"palette":null})
        );
        let dark = serde_json::json!({
            "mode":"dark","accent":4294901760u64,"accentHue":0,"accentIntensity":70,
            "reducedMotion":false,"reducedTransparency":true,
            "palette":{"background":1,"panel":2,"surface":3,"surfaceHover":4,"text":5,
                "muted":6,"accent":7,"accentSoft":8,"complement":9}
        });
        assert!(runtime.set_theme_store(&dark).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_theme_store(&dark).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("observed[1] === observed[2] && Object.isFrozen(observed[1]) && Object.isFrozen(observed[1].palette)").unwrap());
        assert_eq!(
            runtime.eval_json::<u64>("__themeStore.generation").unwrap(),
            1
        );
        let light = serde_json::json!({"mode":"light","accent":4294901760u64,"accentHue":0,
            "accentIntensity":70,"reducedMotion":false,"reducedTransparency":true,
            "palette":{"background":1,"panel":2,"surface":3,"surfaceHover":4,"text":5,
                "muted":6,"accent":7,"accentSoft":8,"complement":9}});
        assert!(runtime.set_theme_store(&light).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("observed[2].palette === observed[3].palette")
                .unwrap()
        );
    }

    #[test]
    fn provider_change_executes_consumers_but_not_unrelated_siblings() {
        let source = r#"
            const Value = createContext('cold');
            globalThis.runs = {app:0, consumer:0, sibling:0};
            function Consumer() { runs.consumer++; return h(Text,{},useContext(Value)); }
            function Sibling() { runs.sibling++; return h(Text,{},'stable'); }
            function App() {
                runs.app++;
                const [value,setValue]=useState('cold');
                return h(Window,{},h(Button,{onClick:()=>setValue('warm')},'change'),
                    h(Value.Provider,{value},h(Consumer),h(Sibling)));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let outcome = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        let super::ScheduledRender::Rendered { value, .. } = outcome else {
            panic!("provider update must render");
        };
        runtime.finish_event(true).unwrap();
        assert!(value.to_string().contains("warm"));
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":2,"consumer":2,"sibling":1})
        );
    }

    #[test]
    fn incrementally_resolved_output_matches_a_cold_render() {
        let source = r#"
            globalThis.initial ??= 0;
            function Leaf() { const [value,setValue]=useState(initial); return h(Button,{onClick:()=>setValue(value+1)},String(value)); }
            function Stable() { return h(Text,{className:'stable'},'same'); }
            function App() { return h(Window,{className:'root'},h(Leaf),h(Stable)); }
        "#;
        let mut incremental = super::JsxRuntime::new(source, None).unwrap();
        incremental.render("__nickelRender()", |_| Ok(())).unwrap();
        let updated = incremental
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        incremental.finish_event(true).unwrap();
        let super::ScheduledRender::Rendered { value: updated, .. } = updated else {
            panic!("leaf update must render");
        };

        let mut cold = super::JsxRuntime::new(source, None).unwrap();
        cold.eval("initial = 1").unwrap();
        let expected = cold
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(updated, expected);
    }

    #[test]
    fn virtual_event_bindings_are_opaque_until_materialized() {
        let source = r#"
            globalThis.virtualNode = null;
            function App() {
                virtualNode = h(Button, {onClick: () => {}}, 'Bound');
                return virtualNode;
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let node = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(node["action"], 0);
        assert!(runtime.eval("JSON.stringify(virtualNode)").is_err());
        assert!(
            runtime
                .eval("JSON.stringify(h(Text, {}, 'Plain'))")
                .is_err()
        );
        assert_eq!(
            runtime
                .eval_json::<usize>("JSON.stringify(__handlers.length)")
                .unwrap(),
            1
        );
    }

    #[test]
    fn materialization_rebuilds_handler_indexes_in_tree_order() {
        let source = r#"
            globalThis.calls = [];
            function App() {
                return h(Column, {},
                    h(Button, {id:'first', onClick:()=>calls.push('first')}, 'First'),
                    h(Button, {id:'second', onClick:()=>calls.push('second')}, 'Second'));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        for _ in 0..2 {
            let node = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert_eq!(node["children"][0]["action"], 0);
            assert_eq!(node["children"][1]["action"], 1);
            assert_eq!(
                runtime
                    .eval_json::<usize>("JSON.stringify(__handlers.length)")
                    .unwrap(),
                2
            );
        }
        runtime.render("__nickelDispatch(1)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(calls)")
                .unwrap(),
            vec!["second"]
        );
    }

    #[test]
    fn rejected_materialized_render_restores_accepted_handlers() {
        let source = r#"
            globalThis.calls = [];
            globalThis.changed = false;
            function App() {
                const label = changed ? 'changed' : 'accepted';
                return h(Button, {onClick:()=>calls.push(label)}, 'Run');
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.eval("changed = true").unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(calls)")
                .unwrap(),
            vec!["accepted"]
        );
    }

    #[test]
    fn ids_are_stable_opaque_and_distinct_by_hook_component_and_surface() {
        let source = r#"
            globalThis.ids ||= [];
            function Child() { return h(Text, null, useId()); }
            function App() {
                const first = useId();
                const second = useId();
                ids.push([first, second]);
                return h(Window, {}, h(Text, null, first), h(Text, null, second), h(Child));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("ids[0][0] === ids[1][0] && ids[0][1] === ids[1][1]")
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>("ids[0][0] !== ids[0][1]")
                .unwrap()
        );
        let default_id = runtime
            .eval_json::<String>("JSON.stringify(ids[0][0])")
            .unwrap();
        assert!(default_id.starts_with(":nickel:default:"));

        runtime.register_surface_entry("other", source).unwrap();
        runtime.select_surface("other").unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let other_id = runtime
            .eval_json::<String>("JSON.stringify(ids[ids.length - 1][0])")
            .unwrap();
        assert_ne!(default_id, other_id);
        assert!(other_id.starts_with(":nickel:other:"));
    }

    #[test]
    fn rejected_render_does_not_admit_id_hook_state() {
        let source = r#"
            globalThis.ids = [];
            function App() { const id = useId(); ids.push(id); return h(Text, null, id); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("ids.length === 2 && ids[0] === ids[1]")
                .unwrap()
        );
    }

    #[test]
    fn retired_surface_ids_do_not_retain_live_hook_state() {
        let source = r#"
            globalThis.ids ||= [];
            function App() { const id = useId(); ids.push(id); return h(Text, null, id); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.drop_surface("default").unwrap();
        runtime.register_surface_entry("default", source).unwrap();
        runtime.select_surface("default").unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("ids.length === 2 && ids[0] !== ids[1]")
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>("__componentHooks.size === 1")
                .unwrap()
        );
    }

    #[test]
    fn react_style_hooks_preserve_identity_and_memoize_with_object_is() {
        let source = r#"
            globalThis.observed = [];
            function App() {
                const [state, setState] = useState(0);
                const [reduced, dispatch] = useReducer((value, action) => action === 'same' ? value : value + 1, 0);
                const memo = useMemo(() => ({state}), [state]);
                const callback = useCallback(() => state, [state]);
                observed.push({setState, dispatch, memo, callback, state, reduced});
                return h(Button, {onClick: () => { setState(value => value); dispatch('same'); }}, String(state + reduced));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("observed[0].setState === observed[1].setState")
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>("observed[0].dispatch === observed[1].dispatch")
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>("observed[0].memo === observed[1].memo")
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>("observed[0].callback === observed[1].callback")
                .unwrap()
        );
        assert_eq!(
            runtime
                .eval_json::<u64>("observed[1].state + observed[1].reduced")
                .unwrap(),
            0
        );
    }

    #[test]
    fn state_and_reducer_updates_during_render_are_rejected_without_committing() {
        for source in [
            "function App(){const [value,setValue]=useState(0);if(value===0)setValue(1);return h(Text,null,String(value))}",
            "function App(){const [value,dispatch]=useReducer(value=>value+1,0);if(value===0)dispatch();return h(Text,null,String(value))}",
        ] {
            let mut runtime = super::JsxRuntime::new(source, None).unwrap();
            let error = runtime.render("__nickelRender()", |_| Ok(())).unwrap_err();
            assert!(error.contains("updates are not allowed during component render"));
            assert_eq!(
                runtime
                    .eval_json::<usize>("JSON.stringify(__componentRecords.size)")
                    .unwrap(),
                0,
                "the forbidden render must not admit component state"
            );
        }
    }

    #[test]
    fn scheduled_dispatch_skips_render_when_hook_values_do_not_change() {
        let source = r#"
            globalThis.renders = 0;
            function App() {
                renders++;
                const [state, setState] = useState(0);
                const [reduced, dispatch] = useReducer((value, action) => action === 'same' ? value : value + 1, 0);
                return h(Button, {onClick: () => { setState(value => value); dispatch('same'); }}, String(state + reduced));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let outcome = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |_| Ok(()))
            .unwrap();
        assert_eq!(outcome, super::ScheduledRender::Unchanged);
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime.eval_json::<u64>("JSON.stringify(renders)").unwrap(),
            1
        );
    }

    #[test]
    fn scheduled_dispatch_reports_dirty_owner_and_effect_follow_up() {
        let source = r#"
            function App() {
                const [state, setState] = useState(0);
                const [derived, setDerived] = useState(0);
                useEffect(() => { if (state === 1) setDerived(2); }, [state]);
                return h(Button, {onClick: () => setState(1)}, String(state + derived));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let outcome = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        let super::ScheduledRender::Rendered {
            value,
            dirty_components,
            reconciliation_requested,
        } = outcome
        else {
            panic!("changed state must render");
        };
        assert_eq!(value["children"][0], "1");
        assert_eq!(dirty_components.len(), 1);
        assert!(reconciliation_requested);
        runtime.finish_event(true).unwrap();
    }

    #[test]
    fn rejected_scheduled_render_restores_state_and_dirty_scheduler() {
        let source = r#"
            function App() {
                const [state, setState] = useState(0);
                return h(Button, {onClick: () => setState(1)}, String(state));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |_| {
                Err::<(), _>("native rejection".into())
            })
            .unwrap_err();
        runtime.finish_event(false).unwrap();
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "0");
        assert!(
            !runtime
                .eval_json::<super::ReconciliationRequest>("__nickelReconciliationRequest()")
                .unwrap()
                .requested
        );
    }

    #[test]
    fn surface_store_is_versioned_stable_and_dirties_only_on_public_change() {
        let source = r#"
            globalThis.observed = [];
            function App() {
                const surface = useSurface();
                const output = useOutput();
                const scale = useScaleFactor();
                const focused = useSurfaceFocus();
                const [value, setValue] = useState(0);
                observed.push({surface, output, scale, focused});
                return h(Button, {onClick:()=>setValue(current=>current)}, String(surface.logicalSize?.width ?? 0));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        assert!(
            runtime
                .set_surface_store(
                    "native-mount-7",
                    &serde_json::json!({"id":"settings","kind":"window","width":640,"height":480})
                )
                .unwrap()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            !runtime
                .set_surface_store(
                    "native-mount-7",
                    &serde_json::json!({"id":"settings","kind":"window","width":640,"height":480})
                )
                .unwrap()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("observed[0].surface === observed[1].surface")
                .unwrap()
        );
        let unchanged = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |_| Ok(()))
            .unwrap();
        assert_eq!(unchanged, super::ScheduledRender::Unchanged);
        runtime.finish_event(true).unwrap();

        assert!(
            runtime
                .set_surface_store(
                    "native-mount-7",
                    &serde_json::json!({"id":"settings","kind":"window","width":800,"height":480})
                )
                .unwrap()
        );
        let changed = runtime
            .dispatch_scheduled("__nickelDispatchBatchScheduled([[0,null]])", |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert!(matches!(changed, super::ScheduledRender::Rendered { .. }));
        runtime.finish_event(true).unwrap();
        assert!(
            runtime
                .eval_json::<bool>(
                    "JSON.stringify(observed[0].surface.generation === 1 && observed[2].surface.generation === 2 && observed[2].surface.mountId === 'native-mount-7' && observed[2].surface.logicalSize.width === 800 && observed[2].output === null && observed[2].scale === null && observed[2].focused === null)"
                )
                .unwrap()
        );
    }

    #[test]
    fn virtual_components_execute_parent_first_and_match_cold_native_output() {
        let source = r#"
            globalThis.order = [];
            function Child({label}) { order.push('child'); return h(Text, {className:'value'}, label); }
            function App() {
                order.push('parent:start');
                const child = h(Child, {label:'same'});
                order.push('parent:end');
                return h(Column, {className:'root'}, child);
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let component_tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(order)")
                .unwrap(),
            ["parent:start", "parent:end", "child"]
        );
        let native_tree = runtime
            .eval_json::<serde_json::Value>(
                "JSON.stringify(__nickelMaterializeVirtual(h(Column,{className:'root'},h(Text,{className:'value'},'same'))))",
            )
            .unwrap();
        assert_eq!(component_tree, native_tree);
    }

    #[test]
    fn contexts_use_defaults_and_nested_parent_first_provider_scopes() {
        let source = r#"
            const Theme = createContext('default');
            globalThis.seen = [];
            function Consumer({name}) { const value = useContext(Theme); seen.push(name + ':' + value); return h(Text,null,value); }
            function App() { return h(Column,null,
                h(Consumer,{name:'outside'}),
                h(Theme.Provider,{value:'outer'},
                    h(Consumer,{name:'outer'}),
                    h(Theme.Provider,{value:'inner'},h(Consumer,{name:'inner'}))),
                h(Consumer,{name:'after'})); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(seen)")
                .unwrap(),
            [
                "outside:default",
                "outer:outer",
                "inner:inner",
                "after:default"
            ]
        );
    }

    #[test]
    fn context_updates_rollback_with_rejected_provider_render() {
        let source = r#"
            const Value = createContext('default');
            globalThis.seen = [];
            function Consumer() { const value=useContext(Value); seen.push(value); return h(Text,null,value); }
            function App() { const [value,setValue]=useState('accepted'); return h(Button,{onClick:()=>setValue('rejected')},h(Value.Provider,{value},h(Consumer))); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .render("__nickelDispatch(0)", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(seen)")
                .unwrap(),
            ["accepted", "rejected", "accepted"]
        );
    }

    #[test]
    fn keyed_context_consumers_reparent_and_retire_without_leaking_scope() {
        let source = r#"
            const Value = createContext('default');
            globalThis.seen = [];
            function Consumer() { const value=useContext(Value); seen.push(value); return h(Text,null,value); }
            function App() { const [right,setRight]=useState(false); return h(Button,{onClick:()=>setRight(true)},
                h(Value.Provider,{key:'left',value:'left'},right?null:h(Consumer,{key:'moving'})),
                h(Value.Provider,{key:'right',value:'right'},right?h(Consumer,{key:'moving'}):null)); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(seen)")
                .unwrap(),
            ["left", "right"]
        );
        assert!(runtime.eval_json::<bool>("Array.from(__componentHooks.values()).filter(h=>h.some(e=>e?.kind==='context')).length === 1").unwrap());
    }

    #[test]
    fn context_state_is_isolated_between_surfaces() {
        let source = r#"
            const Value = createContext('default');
            globalThis.seen ||= [];
            function Consumer(){const value=useContext(Value);seen.push(twinkle.data.surface.id+':'+value);return h(Text,null,value)}
            function App(){return h(Value.Provider,{value:twinkle.data.surface.id},h(Consumer))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.set_data(r#"{"surface":{"id":"first"}}"#).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.register_surface_entry("second", source).unwrap();
        runtime.select_surface("second").unwrap();
        runtime.set_data(r#"{"surface":{"id":"second"}}"#).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(seen)")
                .unwrap(),
            ["first:first", "second:second"]
        );
    }

    #[test]
    fn virtual_component_declarations_never_serialize_unresolved() {
        let mut runtime = super::JsxRuntime::new(
            "function Child(){return h(Text,null,'child')} function App(){return h(Child)}",
            None,
        )
        .unwrap();
        assert!(runtime.eval("JSON.stringify(h(Child))").is_err());
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["kind"], "text");
    }

    #[test]
    fn virtual_component_effect_lifecycle_follows_parent_first_reconciliation() {
        let source = r#"
            globalThis.log = [];
            function Child() {
                log.push('render child');
                useEffect(()=>{log.push('setup child');return()=>log.push('cleanup child')},[]);
                return h(Text,null,'child');
            }
            function App() {
                log.push('render parent');
                const [shown,setShown]=useState(true);
                useEffect(()=>{log.push('setup parent');return()=>log.push('cleanup parent')},[]);
                return h(Button,{onClick:()=>setShown(false)},shown?h(Child):'gone');
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            [
                "render parent",
                "render child",
                "setup parent",
                "setup child"
            ]
        );
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            [
                "render parent",
                "render child",
                "setup parent",
                "setup child",
                "render parent",
                "cleanup child"
            ]
        );
    }

    #[test]
    fn virtual_component_keys_retain_hook_identity_across_reorder() {
        let source = r#"
            function Child({label}) { const [identity]=useState(label); return h(Text,null,identity); }
            function App() {
                const [reversed,setReversed]=useState(false);
                const labels=reversed?['b','a']:['a','b'];
                return h(Column,null,h(Button,{onClick:()=>setReversed(true)},'reverse'),labels.map(label=>h(Child,{key:label,label})));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let first = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(first["children"][1]["children"][0], "a");
        assert_eq!(first["children"][2]["children"][0], "b");
        let second = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(second["children"][1]["children"][0], "b");
        assert_eq!(second["children"][2]["children"][0], "a");
    }

    #[test]
    fn effects_run_after_accepted_commit_and_cleanup_before_rerun_and_unmount() {
        let source = r#"
            globalThis.log = [];
            function App() {
                const [value, setValue] = useState(0);
                useEffect(() => { log.push('setup ' + value); return () => log.push('cleanup ' + value); }, [value]);
                return h(Button, {onClick: () => setValue(value + 1)}, String(value));
            }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        assert!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap()
                .is_empty()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            ["setup 0"]
        );
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            ["setup 0", "cleanup 0", "setup 1"]
        );
        runtime.drop_surface("default").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            ["setup 0", "cleanup 0", "setup 1", "cleanup 1"]
        );
    }

    #[test]
    fn removed_component_effect_cleanup_waits_for_accepted_commit() {
        let source = r#"
            globalThis.log = [];
            function Child() { useEffect(() => () => log.push('removed'), []); return h(Text, null, 'child'); }
            function App() { const [shown, setShown] = useState(true); return h(Button, {onClick: () => setShown(false)}, shown ? h(Child) : 'gone'); }
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .render("__nickelDispatch(0)", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        assert!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap()
                .is_empty()
        );
        runtime.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<String>>("JSON.stringify(log)")
                .unwrap(),
            ["removed"]
        );
    }

    #[test]
    fn checkpoint_discards_consumed_effects_and_restores_supported_hook_objects_in_place() {
        let mut runtime=super::JsxRuntime::new("function App(){const [state]=useState({count:0});return h(Button,{onClick:()=>{state.count++;globalThis.outside=(globalThis.outside||0)+1;twinkle.request('show-launcher');}},String(state.count));}",None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.begin_transaction().unwrap();
        runtime
            .render("__nickelDispatch(0,null)", |_| Ok(()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_eq!(runtime.take_effects().unwrap().len(), 1);
        runtime.finish_transaction(false).unwrap();
        assert!(runtime.take_effects().unwrap().is_empty());
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "0");
        // Arbitrary package globals are explicitly outside the checkpoint.
        assert_eq!(
            runtime
                .eval_json::<u64>("JSON.stringify(globalThis.outside)")
                .unwrap(),
            1
        );
    }

    #[test]
    fn slider_numeric_ranges_normalize_native_values_and_quantize_changes() {
        let mut runtime = JsxRuntime::new("function App() { return h(Slider, {id:'volume', accessibilityLabel:'Volume', min:10, max:110, step:5, value:60, onChange:value=>twinkle.request({type:'changed',value})}); }", None).unwrap();
        let rendered = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(rendered["value"], serde_json::json!(0.5));
        let action = rendered["action"].as_u64().unwrap();
        runtime
            .render(&format!("__nickelDispatch({action},0.53)"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"changed","value":65})]
        );
        assert!(
            runtime
                .eval("__nickelResolveVirtual(h(Slider,{min:1,max:1,value:1,onChange:()=>{}}))")
                .is_err()
        );
        assert!(
            runtime
                .eval("__nickelResolveVirtual(h(Slider,{min:0,max:10,value:11,onChange:()=>{}}))")
                .is_err()
        );
        assert!(
            runtime
                .eval("__nickelResolveVirtual(h(Slider,{value:0.5,step:0,onChange:()=>{}}))")
                .is_err()
        );
    }

    #[test]
    fn drop_handler_receives_serializable_event_data() {
        let source = "function App() { return h('div', {id: 'target', onDrop: event => twinkle.request({type: 'dropped', ...event})}); }";
        let mut runtime = JsxRuntime::new(source, None).unwrap();
        let rendered = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = rendered["dropAction"].as_u64().unwrap();
        let event = serde_json::json!({"x": 12, "y": 14, "sourceId": "source", "targetId": "target",
            "sourceBounds": {"x": 0, "y": 0, "width": 20, "height": 20},
            "targetBounds": {"x": 10, "y": 0, "width": 20, "height": 20}});
        runtime
            .render(&format!("__nickelDispatch({action},{event})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type": "dropped", "x": 12,
            "y": 14, "sourceId": "source", "targetId": "target",
            "sourceBounds": {"x": 0, "y": 0, "width": 20, "height": 20},
            "targetBounds": {"x": 10, "y": 0, "width": 20, "height": 20}})]
        );
    }

    #[test]
    fn rejected_event_restores_hook_state_for_the_next_host() {
        let source = "function App() { const [count, setCount] = useState(0); return h(Window, {}, h(Button, {onClick: () => setCount(count + 1)}, String(count))); }";
        let mut runtime = JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let changed = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        assert_ne!(initial, changed);
        assert!(runtime.take_effects().unwrap().is_empty());
        runtime.finish_event(false).unwrap();
        let restored = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(initial, restored);
    }

    #[test]
    fn sibling_surfaces_keep_independent_hooks_and_handlers_in_one_runtime() {
        let source = "function App() { const [count, setCount] = useState(0); return h(Window, {}, h(Button, {onClick: () => setCount(count + 1)}, `${twinkle.data.surface.id}:${count}`)); }";
        let mut runtime = JsxRuntime::new(source, None).unwrap();
        runtime.select_surface("first").unwrap();
        runtime.set_data(r#"{"surface":{"id":"first"}}"#).unwrap();
        let first = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(first.to_string().contains("first:0"));

        runtime.select_surface("second").unwrap();
        runtime.set_data(r#"{"surface":{"id":"second"}}"#).unwrap();
        let second = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(second.to_string().contains("second:0"));
        let changed = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert!(changed.to_string().contains("second:1"));

        runtime.select_surface("first").unwrap();
        let first = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(first.to_string().contains("first:0"));
        let changed = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert!(changed.to_string().contains("first:1"));

        runtime.select_surface("second").unwrap();
        let second = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(second.to_string().contains("second:1"));
        runtime.drop_surface("second").unwrap();
        runtime.select_surface("second").unwrap();
        runtime.set_data(r#"{"surface":{"id":"second"}}"#).unwrap();
        let reopened = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(reopened.to_string().contains("second:0"));
        runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(false).unwrap();
        runtime.select_surface("first").unwrap();
        let first = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(first.to_string().contains("first:1"));
        runtime.select_surface("second").unwrap();
        let second = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(second.to_string().contains("second:0"));
    }

    #[test]
    fn scoped_entry_does_not_replace_the_main_app() {
        let mut runtime = JsxRuntime::new(
            "function App() { const [count, setCount] = useState(0); return h(Window, {}, h(Button, {onClick: () => setCount(count + 1)}, `Main ${count}`)); }",
            None,
        )
        .unwrap();
        runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        runtime
            .register_surface_entry(
                "menu",
                "function App() { const [count, setCount] = useState(0); return h(Window, {}, h(Button, {onClick: () => setCount(count + 1)}, `Menu ${count}`)); }",
            )
            .unwrap();
        assert!(
            runtime
                .register_surface_entry("menu", "function App() { return h(Window, {}); }")
                .is_err()
        );
        runtime.select_surface("menu").unwrap();
        let menu = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(menu.to_string().contains("Menu 0"));
        let menu = runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert!(menu.to_string().contains("Menu 1"));
        runtime.select_surface("default").unwrap();
        let main = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(main.to_string().contains("Main 1"));
        runtime.drop_surface("menu").unwrap();
        assert!(
            !runtime
                .eval_json::<bool>("__surfaceApps.has('menu')")
                .unwrap()
        );
        runtime.select_surface("default").unwrap();
        let main = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(main.to_string().contains("Main 1"));
    }

    #[test]
    fn infinite_loop_at_startup_returns_an_error() {
        let result = JsxRuntime::new("while (true) {}", None);
        assert!(result.is_err());
    }

    #[test]
    fn infinite_loop_in_handler_does_not_poison_the_runtime() {
        let source = "function App() { return h(Window, {}, h(Button, {onClick: () => { while (true) {} }}, 'Loop')); }";
        let mut runtime = JsxRuntime::new(source, None).unwrap();
        let initial = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(
            runtime
                .render("__nickelDispatch(0)", |node| Ok(node.clone()))
                .is_err()
        );
        runtime.finish_event(false).unwrap();
        let restored = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(initial, restored);
    }
}
