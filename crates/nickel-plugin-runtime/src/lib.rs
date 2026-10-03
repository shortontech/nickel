//! Shared JavaScript evaluator for Nickel's native JSX hosts.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the Boa context and the bootstrap's render and event transactions.

use boa_engine::{Context, JsValue, Source, js_string};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Instant;

mod composition;
pub mod composition_runtime;
mod modules;
pub mod settings;

pub use composition::ComposedShellGraph;
pub use modules::{JsxModuleGraph, ModuleSource};

const BOOTSTRAP: &str = include_str!("../../../assets/plugin-runtime/bootstrap.js");
// Boa enforces this per JavaScript call frame. It bounds accidental infinite
// loops in plugin code without retaining an event or frame history.
const MAX_JS_LOOP_ITERATIONS: u64 = 100_000;

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
const PLUGIN_CAPABILITY_NAMES: &[&str] = &[
    "launcher-show",
    "control-center-show",
    "on-screen-keyboard-show",
    "on-screen-keyboard-read",
    "on-screen-keyboard-input",
    "applications-read",
    "applications-launch",
    "applications-pin",
    "associations-read",
    "associations-control",
    "plugins-read",
    "plugins-control",
    "features-read",
    "features-control",
    "shortcuts-read",
    "preferences-read",
    "preferences-control",
    "windows-read",
    "windows-focus",
    "windows-context",
    "desktop-read",
    "desktop-arrange",
    "desktop-files-open",
    "desktop-files-manage",
    "tray-read",
    "tray-activate",
    "tray-context",
    "appearance-read",
    "appearance-control",
    "wallpaper-read",
    "wallpaper-control",
    "audio-read",
    "audio-control",
    "network-read",
    "network-control",
    "bluetooth-read",
    "bluetooth-control",
    "desktop-control",
    "display-control",
    "session-control",
    "workspaces-read",
    "workspaces-switch",
    "notifications-read",
    "notifications-act",
    "settings-read",
    "settings-write",
    "settings-show",
    "projects-menu-show",
    "session-logout-request",
    "run-command",
];

pub struct JsxRuntime {
    context: Context,
    settings_provider: Option<String>,
    settings_pages: std::collections::BTreeSet<String>,
    settings_revision: u64,
    settings_data: Option<std::rc::Rc<Value>>,
    checkpoint: Option<(u64, Option<std::rc::Rc<Value>>)>,
    invalidated: bool,
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
            "windows" => "__nickelSetWindowsStore",
            "applications" => "__nickelSetApplicationsStore",
            "notifications" => "__nickelSetNotificationsStore",
            "workspaces" => "__nickelSetWorkspacesStore",
            "outputs" => "__nickelSetOutputsStore",
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
    pub fn inject_capabilities(
        &mut self,
        declared: &[nickel_core::plugins::PluginCapability],
        native_data: &Value,
    ) -> Result<bool, String> {
        self.runtime.set_capability_store(declared, native_data)
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

#[derive(Debug, PartialEq)]
pub enum ScheduledRender<T> {
    Unchanged,
    Rendered {
        value: T,
        dirty_components: Vec<String>,
        reconciliation_requested: bool,
    },
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativePatchEnvelope {
    pub version: u8,
    pub operations: Vec<NativePatchOperation>,
    pub counters: NativePatchCounters,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum NativePatchOperation {
    SetPrimitive {
        target: String,
        property: String,
        value: Value,
    },
    ReplaceHandlerSlot {
        slot: String,
        action: usize,
    },
    InsertChild {
        parent: String,
        key: String,
        child_id: String,
        index: usize,
        node: Value,
    },
    RemoveChild {
        parent: String,
        key: String,
        child_id: String,
        index: usize,
    },
    MoveChild {
        parent: String,
        key: String,
        child_id: String,
        from: usize,
        to: usize,
    },
    ReplaceSubtree {
        target: String,
        node: Value,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativePatchCounters {
    pub nodes_visited: u64,
    pub nodes_mutated: u64,
    /// Complete package-local trees materialized for compatibility/cold paths.
    #[serde(default)]
    pub local_materializations: u64,
    /// Nodes traversed by cross-package composition expansion.
    #[serde(default)]
    pub expansion_nodes: u64,
    /// Encoded complete-tree bytes crossing the package boundary.
    #[serde(default)]
    pub tree_bytes: u64,
}

#[derive(Debug, PartialEq)]
pub enum ScheduledPatch {
    Unchanged,
    Patched {
        patch: NativePatchEnvelope,
        dirty_components: Vec<String>,
        transport_bytes: usize,
        reconciliation_requested: bool,
    },
}

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
        let mut runtime = Self {
            context: Context::default(),
            settings_provider: None,
            settings_pages: Default::default(),
            settings_revision: 0,
            settings_data: None,
            checkpoint: None,
            invalidated: false,
        };
        runtime
            .context
            .runtime_limits_mut()
            .set_loop_iteration_limit(MAX_JS_LOOP_ITERATIONS);
        runtime.eval(BOOTSTRAP)?;
        runtime.set_capability_store(&[], &serde_json::json!({}))?;
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

    pub(crate) fn hot_install_modules(
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

    pub fn eval(&mut self, source: &str) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.context
            .eval(Source::from_bytes(source))
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn set_diagnostic_owner(&mut self, owner: &str) -> Result<(), String> {
        let owner = serde_json::to_string(owner).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelSetDiagnosticOwner({owner})"))
    }

    pub fn eval_json<T: DeserializeOwned>(&mut self, source: &str) -> Result<T, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let value = self
            .context
            .eval(Source::from_bytes(source))
            .map_err(|error| error.to_string())?;
        let text = value
            .to_string(&mut self.context)
            .map_err(|error| error.to_string())?
            .to_std_string_escaped();
        serde_json::from_str(&text).map_err(|error| error.to_string())
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
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        // Window data has already been capability-filtered by the native owner.
        // Publish it through its retained store before replacing the compatibility
        // projection so hooks never inspect an unfiltered host catalog.
        if let Some(windows) = data.get("windows") {
            self.set_windows_store(windows)?;
        }
        if let Some(applications) = data.get("applications")
            && applications.as_array().is_some_and(|applications| {
                applications.first().is_none_or(|application| {
                    application.get("name").is_some()
                        && application.get("icon").is_some()
                        && application.get("kind").is_some()
                        && application.get("launchClass").is_some()
                })
            })
        {
            self.set_applications_store(applications)?;
        }
        if let Some(notifications) = data.get("notifications")
            && notifications
                .get("history")
                .and_then(Value::as_array)
                .is_some()
            && notifications
                .get("notification")
                .is_none_or(|notification| {
                    notification.is_null()
                        || (notification.get("appName").is_some()
                            && notification.get("body").is_some()
                            && notification.get("actions").is_some())
                })
        {
            self.set_notifications_store(notifications)?;
        }
        if let Some(workspaces) = data.get("workspaces") {
            self.set_workspaces_store(workspaces)?;
        }
        if let Some(outputs) = data.get("outputs").or_else(|| data.get("displays"))
            && outputs
                .get("outputs")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items.first().is_none_or(|output| {
                        output.get("geometry").is_some()
                            && output.get("work_area").is_some()
                            && output.get("modes").is_some()
                    })
                })
        {
            self.set_outputs_store(outputs)?;
        }
        if let Some(locale) = data.get("locale") {
            self.set_locale_store(locale)?;
        }
        // Effective presentation state is globally readable and deliberately
        // separate from the capability-gated appearance configuration client.
        if let Some(appearance) = data.get("appearance") {
            let resolved = appearance.get("resolved").unwrap_or(&Value::Null);
            let configured = appearance.get("configured").unwrap_or(&Value::Null);
            let accent = resolved.get("accent").and_then(|value| {
                let channels = value.as_array()?;
                if channels.len() != 3 {
                    return None;
                }
                Some(
                    0xff00_0000u64
                        | (channels[0].as_u64()? << 16)
                        | (channels[1].as_u64()? << 8)
                        | channels[2].as_u64()?,
                )
            });
            let animations = configured.get("animations").and_then(Value::as_str);
            let theme = serde_json::json!({
                "mode": resolved.get("theme").and_then(Value::as_str).unwrap_or("unknown"),
                "accent": accent,
                "accentHue": resolved.get("hue").and_then(Value::as_u64),
                "accentIntensity": resolved.get("intensity").and_then(Value::as_u64),
                "reducedMotion": match animations {
                    Some("off" | "reduced") => Some(true),
                    Some("normal") => Some(false),
                    _ => None,
                },
                "reducedTransparency": configured.get("reduce_transparency").and_then(Value::as_bool),
                // The runtime data projection does not currently carry the
                // renderer's semantic palette. Absence is explicit, not guessed.
                "palette": Value::Null,
            });
            self.set_theme_store(&theme)?;
        }
        // Host snapshots are data, not source code. Compiling a large object
        // literal on every input/projection update stalls the compositor.
        let argument =
            JsValue::from_json(&data, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetData"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("host data setter is not callable")?
            .call(&JsValue::undefined(), &[argument], &mut self.context)
            .map_err(|error| error.to_string())?;
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

    /// Publish the package owner's already capability-filtered public window
    /// observation. The JavaScript store retains identity for unchanged values
    /// and advances its monotonic generation only for a public change.
    pub fn set_windows_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetWindowsStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("windows store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    /// Publish the package owner's already capability-filtered application
    /// catalog. Launch and activation remain separately revalidated effects.
    pub fn set_applications_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(
                js_string!("__nickelSetApplicationsStore"),
                &mut self.context,
            )
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("applications store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    /// Publish the owner's already filtered public notification feed. Invoke
    /// and dismiss effects continue through native authority validation.
    pub fn set_notifications_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(
                js_string!("__nickelSetNotificationsStore"),
                &mut self.context,
            )
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("notifications store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    pub fn set_workspaces_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetWorkspacesStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("workspaces store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    pub fn set_outputs_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetOutputsStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("outputs store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    pub fn set_locale_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetLocaleStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("locale store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    /// Publish host-owned effective presentation state. This is observation,
    /// not the capability-gated appearance configuration authority.
    pub fn set_theme_store(&mut self, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetThemeStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("theme store setter is not callable")?
            .call(&JsValue::undefined(), &[snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    /// Publish the current owner's declared grants and bounded runtime
    /// availability. This observation never participates in action authority.
    pub fn set_capability_store(
        &mut self,
        declared: &[nickel_core::plugins::PluginCapability],
        data: &Value,
    ) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let resource = |name: &str| -> Option<&Value> {
            let field = match name {
                "on-screen-keyboard-read" | "on-screen-keyboard-input" => "keyboard",
                "applications-read" | "applications-launch" | "applications-pin" => "applications",
                "associations-read" | "associations-control" => "associations",
                "plugins-read" | "plugins-control" => "plugins",
                "features-read" | "features-control" => "features",
                "shortcuts-read" => "shortcuts",
                "preferences-read" | "preferences-control" => "preferences",
                "windows-read" | "windows-focus" | "windows-context" => "windows",
                "desktop-read"
                | "desktop-arrange"
                | "desktop-files-open"
                | "desktop-files-manage"
                | "desktop-control" => "desktop",
                "tray-read" | "tray-activate" | "tray-context" => "tray",
                "appearance-read" | "appearance-control" => "appearance",
                "wallpaper-read" | "wallpaper-control" => "wallpaper",
                "audio-read" | "audio-control" => "audio",
                "network-read" | "network-control" => "wifi",
                "bluetooth-read" | "bluetooth-control" => "bluetooth",
                "display-control" => "displays",
                "session-control" | "session-logout-request" => "session",
                "workspaces-read" | "workspaces-switch" => "workspaces",
                "notifications-read" | "notifications-act" => "notifications",
                "settings-read" | "settings-write" | "settings-show" => "navigation",
                "run-command" => "run",
                _ => return None,
            };
            data.get(field)
        };
        let mut entries = serde_json::Map::new();
        for &name in PLUGIN_CAPABILITY_NAMES {
            let declared = declared
                .iter()
                .any(|capability| capability.as_str() == name);
            let (available, reason) = if !declared {
                (Some(false), None)
            } else if let Some(snapshot) = resource(name) {
                let available = snapshot
                    .get("available")
                    .and_then(Value::as_bool)
                    .or(Some(true));
                let reason = snapshot
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|reason| reason.chars().take(480).collect::<String>());
                (available, reason)
            } else {
                (None, None)
            };
            entries.insert(
                name.into(),
                serde_json::json!({"declared":declared,"available":available,"reason":reason}),
            );
        }
        let value = serde_json::json!({"known":PLUGIN_CAPABILITY_NAMES,"entries":entries});
        let value =
            JsValue::from_json(&value, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetCapabilityStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("capability store setter is not callable")?
            .call(&JsValue::undefined(), &[value], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    pub fn select_surface(&mut self, id: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelSelectSurface({id})"))
    }

    /// Publish one host-owned, mount-scoped surface observation. JavaScript
    /// retains the previous immutable snapshot when these public fields are
    /// unchanged and advances its monotonic generation otherwise.
    pub fn set_surface_store(&mut self, mount_id: &str, snapshot: &Value) -> Result<bool, String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        let mount = JsValue::from(js_string!(mount_id));
        let snapshot =
            JsValue::from_json(snapshot, &mut self.context).map_err(|error| error.to_string())?;
        let setter = self
            .context
            .global_object()
            .get(js_string!("__nickelSetSurfaceStore"), &mut self.context)
            .map_err(|error| error.to_string())?;
        setter
            .as_callable()
            .ok_or("surface store setter is not callable")?
            .call(&JsValue::undefined(), &[mount, snapshot], &mut self.context)
            .map(|changed| changed.to_boolean())
            .map_err(|error| error.to_string())
    }

    pub fn register_surface_entry(&mut self, id: &str, source: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!(
            "__nickelRegisterSurfaceApp({id}, (function() {{\n{source}\nreturn App;\n}})())"
        ))
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
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelDropSurface({id})"))
    }

    pub fn render<T>(
        &mut self,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let value = self.eval_json::<Value>(expression);
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
            "__nickelCommitRender()"
        } else {
            "__nickelRollbackRender()"
        };
        self.eval(finalizer)
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
        self.eval(if parsed.is_ok() {
            "__nickelCommitRender()"
        } else {
            "__nickelRollbackRender()"
        })
        .map_err(|error| format!("could not finalize scheduled plugin render: {error}"))?;
        self.report_host_profile("cold-tree", validation_micros as u64, transport_bytes)?;
        let value = parsed?;
        let reconciliation_requested = self
            .eval_json::<ReconciliationRequest>("__nickelReconciliationRequest()")?
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
            .eval_json::<ReconciliationRequest>("__nickelReconciliationRequest()")?
            .requested;
        Ok(ScheduledPatch::Patched {
            patch,
            dirty_components: outcome.dirty,
            transport_bytes,
            reconciliation_requested,
        })
    }

    pub fn finish_patch_render(&mut self, accepted: bool) -> Result<(), String> {
        self.eval(if accepted {
            "__nickelCommitRender()"
        } else {
            "__nickelRollbackRender()"
        })
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
        self.eval("__nickelBeginCheckpoint()")?;
        self.checkpoint = Some((self.settings_revision, self.settings_data.clone()));
        Ok(())
    }

    pub fn finish_transaction(&mut self, accepted: bool) -> Result<(), String> {
        let (revision, data) = self
            .checkpoint
            .take()
            .ok_or("runtime transaction is unavailable")?;
        if let Err(error) = self.eval(if accepted {
            "__nickelFinishCheckpoint(true)"
        } else {
            "__nickelFinishCheckpoint(false)"
        }) {
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
        self.eval_json("__nickelTakeEffects()")
    }

    pub fn reconciliation_requested(&mut self) -> Result<bool, String> {
        Ok(self
            .eval_json::<ReconciliationRequest>("__nickelReconciliationRequest()")?
            .requested)
    }

    /// Returns the bounded, coalesced component-failure evidence retained by
    /// this package context. Reading diagnostics does not clear them.
    pub fn boundary_diagnostics(&mut self) -> Result<Vec<Value>, String> {
        self.eval_json("__nickelBoundaryDiagnostics()")
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
        self.eval_json("__nickelRuntimeDiagnostics()")
    }

    fn report_host_profile(
        &mut self,
        transport_kind: &str,
        native_validation_micros: u64,
        transport_bytes: usize,
    ) -> Result<(), String> {
        self.eval(&format!(
            "__nickelReportHostProfile({transport_kind:?},{native_validation_micros},{transport_bytes})"
        ))
    }

    /// Record one host-owned typed patch application without retaining the
    /// patch, candidate tree, or validation error in JavaScript diagnostics.
    pub fn report_typed_patch_apply(
        &mut self,
        application_micros: u64,
        accepted: bool,
    ) -> Result<(), String> {
        self.eval(&format!(
            "__nickelReportTypedPatchApply({application_micros},{accepted})"
        ))
    }

    pub fn finish_event(&mut self, accepted: bool) -> Result<(), String> {
        if accepted {
            self.settings_revision = self.settings_revision.wrapping_add(1);
        }
        self.eval(if accepted {
            "__nickelAcceptEvent()"
        } else {
            "__nickelRollbackEvent()"
        })
        .map_err(|error| format!("could not finalize plugin event: {error}"))
    }
}

#[cfg(test)]
mod tests {
    fn declaration_object_body<'a>(source: &'a str, marker: &str) -> &'a str {
        let start = source
            .find(marker)
            .unwrap_or_else(|| panic!("missing declaration marker {marker}"));
        let open = source[start..].find('{').unwrap() + start;
        let mut depth = 0usize;
        for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[open + 1..open + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated declaration object after {marker}")
    }

    fn declaration_members(body: &str) -> std::collections::BTreeMap<String, String> {
        let mut members = std::collections::BTreeMap::new();
        let insert =
            |declaration: &str, members: &mut std::collections::BTreeMap<String, String>| {
                let declaration = declaration.trim();
                let declaration = declaration.strip_prefix("readonly ").unwrap_or(declaration);
                let name = declaration
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect::<String>();
                if !name.is_empty() {
                    members.insert(name, declaration.to_owned());
                }
            };
        let mut start = 0usize;
        let mut round = 0usize;
        let mut square = 0usize;
        let mut curly = 0usize;
        let mut angle = 0usize;
        for (index, byte) in body.as_bytes().iter().enumerate() {
            match byte {
                b'(' => round += 1,
                b')' => round = round.saturating_sub(1),
                b'[' => square += 1,
                b']' => square = square.saturating_sub(1),
                b'{' => curly += 1,
                b'}' => curly = curly.saturating_sub(1),
                b'<' => angle += 1,
                b'>' => angle = angle.saturating_sub(1),
                b';' if round == 0 && square == 0 && curly == 0 && angle == 0 => {
                    insert(&body[start..index], &mut members);
                    start = index + 1;
                }
                _ => {}
            }
        }
        insert(&body[start..], &mut members);
        members
    }

    fn declarations_without_comments(source: &str) -> String {
        let bytes = source.as_bytes();
        let mut result = String::with_capacity(source.len());
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index..].starts_with(b"/*") {
                index += 2;
                while index < bytes.len() && !bytes[index..].starts_with(b"*/") {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            } else if bytes[index..].starts_with(b"//") {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                result.push('\n');
            } else {
                result.push(bytes[index] as char);
                index += 1;
            }
        }
        result
    }

    fn declared_runtime_surface(
        declarations: &str,
    ) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
        let declarations = declarations_without_comments(declarations);
        let declarations = declarations.as_str();
        let mut surface = std::collections::BTreeMap::new();
        let globals = declarations
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                ["declare function ", "declare const "]
                    .into_iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
                    .map(|rest| {
                        rest.split(|character: char| {
                            character == '(' || character == ':' || character == '<'
                        })
                        .next()
                        .unwrap()
                        .trim()
                        .to_owned()
                    })
            })
            .collect();
        surface.insert("globalThis".into(), globals);

        let nickel_members = declaration_members(declaration_object_body(
            declarations,
            "declare const nickel: Readonly<",
        ));
        surface.insert("nickel".into(), nickel_members.keys().cloned().collect());
        for (name, declaration) in nickel_members {
            if name == "data" {
                continue;
            }
            let Some(colon) = declaration.find(':') else {
                continue;
            };
            let object_type = declaration[colon + 1..]
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if !object_type.starts_with("Readonly<{") {
                continue;
            }
            let nested = declaration_object_body(&declaration[colon + 1..], "Readonly<{");
            surface.insert(
                format!("nickel.{name}"),
                declaration_members(nested).keys().cloned().collect(),
            );
        }

        let stores = declaration_members(declaration_object_body(
            declarations,
            "declare const NickelStores:Readonly<",
        ));
        surface.insert("NickelStores".into(), stores.keys().cloned().collect());
        let store_members = declaration_members(declaration_object_body(
            declarations,
            "interface NickelExternalStore",
        ));
        for name in stores.keys() {
            surface.insert(
                format!("NickelStores.{name}"),
                store_members.keys().cloned().collect(),
            );
        }
        surface
    }

    fn surface_difference(
        declared: &std::collections::BTreeSet<String>,
        runtime: &std::collections::BTreeSet<String>,
    ) -> (Vec<String>, Vec<String>) {
        (
            declared.difference(runtime).cloned().collect(),
            runtime.difference(declared).cloned().collect(),
        )
    }

    #[test]
    fn jsx_test_harness_queries_events_batches_and_injects_domain_stores() {
        use super::{JsxTestEvent, JsxTestHarness};

        let source = r#"
            function App() {
                const [count,setCount]=useState(0);
                const locale=useLocale(), surface=useSurface(), capability=useCapability('windows-read');
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
                &[nickel_core::plugins::PluginCapability::WindowsRead],
                &serde_json::json!({"windows":{"available":true}}),
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
    fn plugin_declarations_match_the_bidirectional_runtime_surface() {
        let entrypoint = include_str!("../../../assets/plugin-runtime/index.d.ts");
        assert!(
            entrypoint.contains(r#"<reference path="../plugins/nickel-plugin.d.ts" />"#),
            "plugin-runtime/index.d.ts must route to the canonical ambient declarations"
        );
        let declarations = include_str!("../../../assets/plugins/nickel-plugin.d.ts");
        let declared = declared_runtime_surface(declarations);
        let mut runtime =
            super::JsxRuntime::new("function App(){return h(Text,null,'ok')}", None).unwrap();
        let runtime_globals = runtime
            .eval_json::<std::collections::BTreeSet<String>>(
                "JSON.stringify(__nickelPublicRuntimeGlobals)",
            )
            .unwrap();
        let (declaration_only, runtime_only) =
            surface_difference(&declared["globalThis"], &runtime_globals);
        assert!(
            declaration_only.is_empty() && runtime_only.is_empty(),
            "public global drift: declaration-only={declaration_only:?}, runtime-only={runtime_only:?}"
        );

        for name in &runtime_globals {
            let expression = format!("typeof {name} !== 'undefined'");
            let present = runtime
                .eval_json::<bool>(&format!("JSON.stringify({expression})"))
                .unwrap();
            assert!(
                present,
                "nickel-plugin.d.ts declares missing runtime global {name}"
            );
        }

        let observed = runtime
            .eval_json::<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>>(
                r#"JSON.stringify((()=>{const result={nickel:Object.keys(nickel),NickelStores:Object.keys(NickelStores)};
                for(const [name,value] of Object.entries(nickel))if(name!=='data'&&value!==null&&typeof value==='object')result['nickel.'+name]=Object.keys(value);
                for(const [name,value] of Object.entries(NickelStores))result['NickelStores.'+name]=Object.keys(value);return result})())"#,
            )
            .unwrap();
        for (namespace, declared_members) in
            declared.iter().filter(|(name, _)| *name != "globalThis")
        {
            let runtime_members = observed
                .get(namespace)
                .unwrap_or_else(|| panic!("declared namespace {namespace} is absent at runtime"));
            let (declaration_only, runtime_only) =
                surface_difference(declared_members, runtime_members);
            assert!(
                declaration_only.is_empty() && runtime_only.is_empty(),
                "public namespace drift in {namespace}: declaration-only={declaration_only:?}, runtime-only={runtime_only:?}"
            );
        }
        let unexpected_namespaces = observed
            .keys()
            .filter(|name| !declared.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            unexpected_namespaces.is_empty(),
            "runtime-only public namespaces: {unexpected_namespaces:?}"
        );

        let declared_capabilities = declarations
            .split("type NickelCapability =")
            .nth(1)
            .unwrap()
            .split("interface NickelCapabilitySnapshot")
            .next()
            .unwrap();
        for capability in super::PLUGIN_CAPABILITY_NAMES {
            assert!(
                declared_capabilities.contains(&format!("\"{capability}\"")),
                "runtime capability {capability} is absent from nickel-plugin.d.ts"
            );
        }
    }

    #[test]
    fn bidirectional_surface_comparison_rejects_both_drift_directions() {
        let declared = std::collections::BTreeSet::from(["shared".into(), "typedOnly".into()]);
        let runtime = std::collections::BTreeSet::from(["shared".into(), "runtimeOnly".into()]);
        let (declaration_only, runtime_only) = surface_difference(&declared, &runtime);
        assert_eq!(declaration_only, ["typedOnly"]);
        assert_eq!(runtime_only, ["runtimeOnly"]);
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
    fn native_rejection_activates_nearest_boundary_after_rollback() {
        let mut runtime = super::JsxRuntime::new(
            r#"
            function Leaf(){const [count,setCount]=useState(0);return h(Button,{onClick:()=>{setCount(count+1);nickel.windows.activate('provisional')}},'leaf:'+count)}
            function Inner(){return h(ErrorBoundary,{fallback:(error,reset)=>h(Button,{onClick:reset},'inner:'+error.message)},h(Leaf))}
            function App(){return h(ErrorBoundary,{fallback:h(Text,null,'outer')},h(Column,null,h(Inner),h(Text,null,'sibling')))}
            "#,
            None,
        )
        .unwrap();
        let initial: Value = runtime.eval_json("__nickelRender()").unwrap();
        runtime.eval("__nickelCommitRender()").unwrap();
        runtime.begin_transaction().unwrap();
        let candidate = runtime
            .dispatch_patched("__nickelDispatchBatchPatched([[0,null]])")
            .unwrap();
        assert!(matches!(candidate, super::ScheduledPatch::Patched { .. }));
        assert_eq!(runtime.take_effects().unwrap().len(), 1);
        runtime.finish_patch_render(true).unwrap();
        runtime.finish_event(true).unwrap();
        runtime.finish_transaction(false).unwrap();

        let owner = runtime
            .eval_json::<String>("JSON.stringify(Array.from(__componentRecords).find(([,record])=>record.kind.name==='Leaf')[0])")
            .unwrap();
        let captured = runtime
            .capture_native_failure(&[owner], "native shape rejected")
            .unwrap();
        assert_eq!(captured.len(), 1);
        let fallback: Value = runtime.eval_json("__nickelRender()").unwrap();
        runtime.eval("__nickelCommitRender()").unwrap();
        assert!(fallback.to_string().contains("inner:native shape rejected"));
        assert!(fallback.to_string().contains("sibling"));
        assert!(!fallback.to_string().contains("leaf:1"));
        assert_eq!(runtime.take_effects().unwrap(), Vec::<Value>::new());
        let diagnostics = runtime.boundary_diagnostics().unwrap();
        assert_eq!(diagnostics.last().unwrap()["phase"], "native-validation");
        assert_eq!(
            diagnostics.last().unwrap()["message"],
            "native shape rejected"
        );
        // The previously admitted tree remains the native authority until the
        // fallback patch is independently accepted.
        assert!(initial.to_string().contains("leaf:0"));
    }

    #[test]
    fn boundary_contains_selector_and_cleanup_failures() {
        let mut selector = super::JsxRuntime::new(
            "function Child(){useWindows(()=>{throw Error('selector boom')});return h(Text,null,'bad')} function App(){return h(ErrorBoundary,{fallback:h(Text,null,'selector fallback')},h(Child))}",
            None,
        )
        .unwrap();
        let tree = selector
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "selector fallback");

        let source = r#"
            function Child(){useEffect(()=>()=>{throw Error('cleanup boom')},[]);return h(Text,null,'child')}
            function App(){const [shown,setShown]=useState(true);return h(ErrorBoundary,{fallback:h(Text,null,'cleanup fallback')},h(Button,{onClick:()=>setShown(false)},shown?h(Child):'gone'))}
        "#;
        let mut cleanup = super::JsxRuntime::new(source, None).unwrap();
        cleanup.render("__nickelRender()", |_| Ok(())).unwrap();
        cleanup.render("__nickelDispatch(0)", |_| Ok(())).unwrap();
        cleanup.finish_event(true).unwrap();
        let tree = cleanup
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(tree["children"][0], "cleanup fallback");
        assert!(
            cleanup
                .boundary_diagnostics()
                .unwrap()
                .iter()
                .any(|entry| entry["phase"] == "cleanup")
        );
    }

    #[test]
    fn failing_surface_does_not_replace_an_independent_surface_or_its_handlers() {
        let source = r#"
            function App(){return h(ErrorBoundary,{fallback:h(Text,null,'settings failed')},
                nickel.data.fail ? h((()=>{throw Error('settings')})) : h(Button,{onClick:()=>nickel.request('show-launcher')},nickel.data.name))}
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
    fn developer_diagnostics_cover_identity_selectors_props_depth_and_loops() {
        fn kinds(runtime: &mut super::JsxRuntime) -> Vec<String> {
            runtime.runtime_diagnostics().unwrap()["developerDiagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["kind"].as_str().unwrap().to_owned())
                .collect()
        }

        let mut identity = super::JsxRuntime::new(
            "function App(){return h(Column,null,[h(Text,null,'unkeyed')])}",
            None,
        )
        .unwrap();
        identity.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(kinds(&mut identity).contains(&"positional-identity-churn".into()));

        let mut selector = super::JsxRuntime::new(
            "function App(){const value=useWindows(items=>({count:items.length}));return h(Text,null,String(value.count))}",
            None,
        )
        .unwrap();
        selector.render("__nickelRender()", |_| Ok(())).unwrap();
        selector.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(kinds(&mut selector).contains(&"unstable-selector".into()));

        let mut props = super::JsxRuntime::new(
            "function Bad(){return h(Slider,{min:1,max:0,value:0,onChange:()=>{}})} function App(){return h(ErrorBoundary,{fallback:h(Text,null,'fallback')},h(Bad))}",
            None,
        )
        .unwrap();
        props.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(kinds(&mut props).contains(&"invalid-props".into()));

        let mut depth = super::JsxRuntime::new(
            "function Nest({depth}){return depth?h(Nest,{depth:depth-1}):h(Text,null,'leaf')} function App(){return h(Nest,{depth:52})}",
            None,
        )
        .unwrap();
        depth.render("__nickelRender()", |_| Ok(())).unwrap();
        let diagnostics = depth.runtime_diagnostics().unwrap();
        let depth_diagnostic = diagnostics["developerDiagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "excessive-component-depth")
            .unwrap();
        assert!(depth_diagnostic["stack"].as_array().unwrap().len() <= 16);
        assert!(
            depth_diagnostic["suggestion"]
                .as_str()
                .unwrap()
                .contains("Flatten")
        );
        assert!(kinds(&mut depth).contains(&"render-loop".into()));

        let mut effect_loop = super::JsxRuntime::new(
            "function App(){const [value,setValue]=useState(0);useEffect(()=>setValue(value+1));return h(Text,null,String(value))}",
            None,
        )
        .unwrap();
        for _ in 0..25 {
            effect_loop.render("__nickelRender()", |_| Ok(())).unwrap();
        }
        assert!(kinds(&mut effect_loop).contains(&"effect-loop".into()));
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
        assert_eq!(profile["lifecycleCommits"], 2);
        assert!(profile["coldTreeTransportBytes"].as_u64().unwrap() > 0);
        assert!(profile["patchEnvelopeTransportBytes"].as_u64().unwrap() > 0);
        assert!(profile["nativeValidationMicros"].is_u64());
        assert_eq!(profile["typedPatchApplyAttempts"], 2);
        assert_eq!(profile["typedPatchApplyRejections"], 1);
        assert_eq!(profile["typedPatchApplyMicros"], 48);
        assert_eq!(profile["timingPrecision"], "wall-clock-milliseconds");
        assert_eq!(profile["hostTimingPrecision"], "wall-clock-microseconds");

        runtime.select_surface("settings").unwrap();
        runtime
            .set_surface_store("mount-settings", &serde_json::json!({"id":"settings"}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let diagnostics = runtime.runtime_diagnostics().unwrap();
        assert_eq!(diagnostics["profiles"].as_array().unwrap().len(), 2);
        assert!(diagnostics["profiles"].as_array().unwrap().iter().any(
            |profile| profile["surface"] == "settings" && profile["mount"] == "mount-settings"
        ));
    }

    #[test]
    fn generated_jsx_updates_match_cold_oracle_through_production_patch_scheduler() {
        use super::{NativePatchEnvelope, NativePatchOperation, ScheduledPatch};

        const SOURCE: &str = r#"
            const OracleContext=createContext('missing');
            function Item({item,revision}) {
                const context=useContext(OracleContext);
                const windows=useWindows(values=>values.map(value=>value.title).join('|'));
                return h(Text,{key:'native-'+item},`${item}:${revision}:${context}:${windows}`);
            }
            function App() {
                const initial=nickel.data.oracle;
                const [items,setItems]=useState(initial.items);
                const [state,setState]=useState(initial.state);
                const [reduced,dispatch]=useReducer((value,delta)=>value+delta,initial.reduced);
                const [context,setContext]=useState(initial.context);
                const [revision,setRevision]=useState(initial.revision);
                return h(Window,{id:'oracle'},
                    h(Button,{id:'mutate',key:'mutate',onClick:update=>{
                        setItems(update.items);setState(update.state);dispatch(update.delta);
                        setContext(update.context);setRevision(update.revision);
                    }},'mutate'),
                    h(Text,{id:'state',key:'state'},`${state}:${reduced}`),
                    h(OracleContext.Provider,{key:'provider',value:context},
                        h(Column,{id:'items'},...items.map(item=>h(Item,{key:'item-'+item,item,revision})))));
            }
        "#;

        #[derive(Clone)]
        struct Model {
            items: Vec<u8>,
            state: u64,
            reduced: u64,
            context: String,
            revision: u64,
            window_revision: u64,
        }

        fn data(model: &Model) -> Value {
            serde_json::json!({"oracle":{"items":model.items,"state":model.state,
                "reduced":model.reduced,"context":model.context,"revision":model.revision}})
        }

        fn windows(model: &Model) -> Value {
            serde_json::json!([{"id":"window","title":format!("Window {}",model.window_revision),
                "active":true,"canActivate":true}])
        }

        fn find_native_mut<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
            if value.get("__nativeId").and_then(Value::as_str) == Some(id) {
                return Some(value);
            }
            match value {
                Value::Array(values) => values
                    .iter_mut()
                    .find_map(|value| find_native_mut(value, id)),
                Value::Object(object) => object
                    .values_mut()
                    .find_map(|value| find_native_mut(value, id)),
                _ => None,
            }
        }

        fn find_handler_mut<'a>(
            value: &'a mut Value,
            slot: &str,
        ) -> Option<(&'a mut Value, String)> {
            if let Some(property) = value
                .get("__handlerSlots")
                .and_then(Value::as_object)
                .and_then(|slots| {
                    slots.iter().find_map(|(property, candidate)| {
                        (candidate.as_str() == Some(slot)).then(|| property.clone())
                    })
                })
            {
                return Some((value, property));
            }
            match value {
                Value::Array(values) => values
                    .iter_mut()
                    .find_map(|value| find_handler_mut(value, slot)),
                Value::Object(object) => object
                    .values_mut()
                    .find_map(|value| find_handler_mut(value, slot)),
                _ => None,
            }
        }

        fn apply_patch(tree: &mut Value, patch: &NativePatchEnvelope) {
            for operation in &patch.operations {
                match operation {
                    NativePatchOperation::SetPrimitive {
                        target,
                        property,
                        value,
                    } => {
                        find_native_mut(tree, target).unwrap()[property] = value.clone();
                    }
                    NativePatchOperation::ReplaceHandlerSlot { slot, action } => {
                        let (node, property) = find_handler_mut(tree, slot).unwrap();
                        node[&property] = serde_json::json!(action);
                    }
                    NativePatchOperation::InsertChild {
                        parent,
                        child_id,
                        index,
                        node,
                        ..
                    } => {
                        assert_eq!(node["__nativeId"], child_id.as_str());
                        find_native_mut(tree, parent).unwrap()["children"]
                            .as_array_mut()
                            .unwrap()
                            .insert(*index, node.clone());
                    }
                    NativePatchOperation::RemoveChild {
                        parent,
                        child_id,
                        index,
                        ..
                    } => {
                        let children = find_native_mut(tree, parent).unwrap()["children"]
                            .as_array_mut()
                            .unwrap();
                        assert_eq!(children[*index]["__nativeId"], child_id.as_str());
                        children.remove(*index);
                    }
                    NativePatchOperation::MoveChild {
                        parent,
                        child_id,
                        from,
                        to,
                        ..
                    } => {
                        let children = find_native_mut(tree, parent).unwrap()["children"]
                            .as_array_mut()
                            .unwrap();
                        assert_eq!(children[*from]["__nativeId"], child_id.as_str());
                        let child = children.remove(*from);
                        children.insert(*to, child);
                    }
                    NativePatchOperation::ReplaceSubtree { target, node } => {
                        *find_native_mut(tree, target).unwrap() = node.clone();
                    }
                }
            }
        }

        fn cold(model: &Model) -> Value {
            let serialized = data(model).to_string();
            let mut runtime = super::JsxRuntime::new(SOURCE, Some(&serialized)).unwrap();
            runtime.set_windows_store(&windows(model)).unwrap();
            runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap()
        }

        fn native_ids(value: &Value, ids: &mut std::collections::BTreeMap<String, String>) {
            if let (Some(key), Some(id)) = (
                value.get("key").and_then(Value::as_str),
                value.get("__nativeId").and_then(Value::as_str),
            ) {
                ids.insert(key.to_owned(), id.to_owned());
            }
            match value {
                Value::Array(values) => values.iter().for_each(|value| native_ids(value, ids)),
                Value::Object(object) => object.values().for_each(|value| native_ids(value, ids)),
                _ => {}
            }
        }

        for seed in [0x1234_5678_u64, 0x9abc_def0, 0x0ddc_0ffe] {
            let mut random = seed;
            let mut model = Model {
                items: vec![0, 1, 2],
                state: 0,
                reduced: 0,
                context: "context-0".into(),
                revision: 0,
                window_revision: 0,
            };
            let serialized = data(&model).to_string();
            let mut runtime = super::JsxRuntime::new(SOURCE, Some(&serialized)).unwrap();
            runtime.set_windows_store(&windows(&model)).unwrap();
            let mut accepted = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            assert_eq!(accepted, cold(&model));

            for step in 0..12_u64 {
                random = random
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                let (operation, key) = match step {
                    0 => (0, 6), // guaranteed insertion
                    1 => (1, 1), // guaranteed removal
                    2 => (2, 2), // guaranteed reorder, rejected once before admission
                    3 => (3, 0), // guaranteed prop-only revision
                    _ => (((random >> 32) % 4) as u8, ((random >> 40) % 7) as u8),
                };
                let mut next = model.clone();
                match operation {
                    0 if !next.items.contains(&key) => {
                        let index = key as usize % (next.items.len() + 1);
                        next.items.insert(index, key);
                    }
                    1 if next.items.len() > 1 && next.items.contains(&key) => {
                        next.items.retain(|item| *item != key);
                    }
                    2 if next.items.len() > 1 => {
                        let by = key as usize % next.items.len();
                        next.items.rotate_left(by);
                    }
                    _ => next.revision += 1,
                }
                next.state = step + 1;
                next.reduced += 1;
                next.context = format!("context-{}", (random >> 48) % 5);
                if step % 3 == 0 {
                    next.window_revision += 1;
                    runtime.set_windows_store(&windows(&next)).unwrap();
                }

                let mut before_ids = std::collections::BTreeMap::new();
                native_ids(&accepted, &mut before_ids);
                let action = accepted["children"][0]["action"].as_u64().unwrap();
                let update = serde_json::json!({"items":next.items,"state":next.state,
                    "delta":1,"context":next.context,"revision":next.revision});
                let expression = format!("__nickelDispatchBatchPatched([[{action},{update}]])");

                if step % 5 == 2 {
                    let rejected = runtime.dispatch_patched(&expression).unwrap();
                    assert!(matches!(rejected, ScheduledPatch::Patched { .. }));
                    runtime.finish_patch_render(false).unwrap();
                    runtime.finish_event(false).unwrap();
                    assert_eq!(
                        accepted,
                        cold(&model),
                        "rejected step changed admitted output"
                    );
                }

                let outcome = runtime.dispatch_patched(&expression).unwrap();
                let ScheduledPatch::Patched { patch, .. } = outcome else {
                    panic!("generated update must produce a typed patch")
                };
                let has_operation = |expected: &str| {
                    patch.operations.iter().any(|operation| {
                        matches!(
                            (expected, operation),
                            ("insert", NativePatchOperation::InsertChild { .. })
                                | ("remove", NativePatchOperation::RemoveChild { .. })
                                | ("move", NativePatchOperation::MoveChild { .. })
                                | ("property", NativePatchOperation::SetPrimitive { .. })
                        )
                    })
                };
                match step {
                    0 => assert!(
                        has_operation("insert"),
                        "forced insert emitted no insertion"
                    ),
                    1 => assert!(has_operation("remove"), "forced remove emitted no removal"),
                    2 => assert!(has_operation("move"), "forced reorder emitted no move"),
                    3 => assert!(
                        has_operation("property"),
                        "forced prop update emitted no primitive update"
                    ),
                    _ => {}
                }
                assert_eq!(patch.counters.local_materializations, 0);
                assert_eq!(patch.counters.tree_bytes, 0);
                apply_patch(&mut accepted, &patch);
                runtime.finish_patch_render(true).unwrap();
                runtime.finish_event(true).unwrap();
                model = next;

                assert_eq!(accepted, cold(&model), "seed={seed:#x} step={step}");
                let mut after_ids = std::collections::BTreeMap::new();
                native_ids(&accepted, &mut after_ids);
                for key in model.items.iter().map(|key| format!("native-{key}")) {
                    if let Some(before) = before_ids.get(&key) {
                        assert_eq!(after_ids.get(&key), Some(before), "key {key} lost identity");
                    }
                }
                assert_eq!(accepted["children"][0]["action"], action);
            }
        }
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
    fn windows_store_is_versioned_immutable_and_retains_unchanged_identity() {
        let source = r#"
            globalThis.observed=[];
            function App(){const windows=useWindows();observed.push(windows);return h(Text,null,String(windows.length))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let first =
            serde_json::json!([{"id":"1","title":"Editor","active":true,"canActivate":true}]);
        assert!(runtime.set_windows_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_windows_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>(
                    "observed[0] === observed[1] && observed[0][0] === observed[1][0]"
                )
                .unwrap()
        );
        assert!(
            runtime
                .eval_json::<bool>(
                    "Object.isFrozen(observed[0]) && Object.isFrozen(observed[0][0])"
                )
                .unwrap()
        );
        assert_eq!(
            runtime
                .eval_json::<u64>("__windowsStore.generation")
                .unwrap(),
            1
        );

        let second = serde_json::json!([
            {"id":"1","title":"Editor","active":true,"canActivate":true},
            {"id":"2","title":"Terminal","active":false}
        ]);
        assert!(runtime.set_windows_store(&second).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>(
                    "observed[1] !== observed[2] && observed[1][0] === observed[2][0]"
                )
                .unwrap()
        );
        assert_eq!(
            runtime
                .eval_json::<u64>("__windowsStore.generation")
                .unwrap(),
            2
        );
    }

    #[test]
    fn active_window_follows_public_store_order_and_updates() {
        let source = r#"
            globalThis.seen=[];
            function App(){const active=useActiveWindow();seen.push(active?.id ?? null);return h(Text,null,active?.title ?? 'none')}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .set_windows_store(&serde_json::json!([
                {"id":"first","title":"First","active":true},
                {"id":"second","title":"Second","active":true}
            ]))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_windows_store(&serde_json::json!([
                {"id":"first","title":"First","active":false},
                {"id":"second","title":"Second","active":true}
            ]))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.set_windows_store(&serde_json::json!([])).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<Vec<Option<String>>>("JSON.stringify(seen)")
                .unwrap(),
            [Some("first".into()), Some("second".into()), None]
        );
    }

    #[test]
    fn window_and_surface_stores_dirty_only_their_subscribers() {
        let source = r#"
            globalThis.runs={app:0,windows:0,surface:0,sibling:0};
            function Windows(){runs.windows++;return h(Text,null,useWindows(items=>items[0]?.title ?? 'none'))}
            function Surface(){runs.surface++;return h(Text,null,String(useSurface().logicalSize?.width ?? 0))}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Windows),h(Surface),h(Sibling))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime
            .set_windows_store(&serde_json::json!([{"id":"1","title":"One"}]))
            .unwrap();
        runtime
            .set_surface_store(
                "mount",
                &serde_json::json!({"id":"main","kind":"window","width":640,"height":480}),
            )
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
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
            serde_json::json!({"app":1,"windows":1,"surface":2,"sibling":1})
        );
        runtime
            .set_windows_store(&serde_json::json!([{"id":"1","title":"Two"}]))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"windows":2,"surface":2,"sibling":1})
        );
    }

    #[test]
    fn rejected_window_consumer_render_does_not_install_subscription() {
        let mut runtime = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useWindows().length))}",
            None,
        )
        .unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime
            .set_windows_store(&serde_json::json!([{"id":"1","title":"One"}]))
            .unwrap();
        assert!(
            !runtime
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
        );
    }

    #[test]
    fn applications_store_is_versioned_and_structurally_shares_records() {
        let mut runtime = super::JsxRuntime::new(
            "globalThis.seen=[];function App(){const applications=useApplications();seen.push(applications);return h(Text,null,applications.map(value=>value.name).join(','))}",
            None,
        ).unwrap();
        let first = serde_json::json!([{"id":"editor","name":"Editor","icon":"application:1","pinned":true,
            "pinOrder":0,"recentOrder":1,"kind":"application","launchClass":"graphical"}]);
        assert!(runtime.set_applications_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_applications_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[0] === seen[1] && seen[0][0] === seen[1][0] && Object.isFrozen(seen[0]) && Object.isFrozen(seen[0][0])").unwrap());
        assert_eq!(
            runtime
                .eval_json::<u64>("__applicationsStore.generation")
                .unwrap(),
            1
        );
        let second = serde_json::json!([
            {"id":"editor","name":"Editor","icon":"application:1","pinned":true,"pinOrder":0,
                "recentOrder":1,"kind":"application","launchClass":"graphical"},
            {"id":"terminal","name":"Terminal","icon":"application:2","pinned":false,"pinOrder":null,
                "recentOrder":0,"kind":"application","launchClass":"terminal"}
        ]);
        runtime.set_applications_store(&second).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("seen[1] !== seen[2] && seen[1][0] === seen[2][0]")
                .unwrap()
        );
    }

    #[test]
    fn applications_store_dirties_only_changed_application_selections() {
        let source = r#"
            globalThis.runs={app:0,applications:0,windows:0,theme:0,sibling:0};
            function Applications(){runs.applications++;return h(Text,null,useApplications(items=>items[0]?.name ?? 'none'))}
            function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}
            function Theme(){runs.theme++;return h(Text,null,useTheme(value=>value.mode))}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Applications),h(Windows),h(Theme),h(Sibling))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.set_applications_store(&serde_json::json!([{"id":"one","name":"One","icon":"application:1","kind":"application","launchClass":"graphical"}])).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"applications":2,"windows":1,"theme":1,"sibling":1})
        );
    }

    #[test]
    fn rejected_application_consumer_does_not_install_a_subscription() {
        let mut runtime = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useApplications().length))}",
            None,
        )
        .unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime.set_applications_store(&serde_json::json!([{"id":"one","name":"One","icon":"application:1","kind":"application","launchClass":"graphical"}])).unwrap();
        assert!(
            !runtime
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
        );
    }

    #[test]
    fn notifications_store_versions_visibility_and_structurally_shares_history() {
        let mut runtime = super::JsxRuntime::new(
            "globalThis.seen=[];function App(){const value=useNotifications();seen.push(value);return h(Text,null,value.notification?.summary??'none')}", None).unwrap();
        let item = serde_json::json!({"id":1,"appName":"Mail","summary":"Hello","body":"Body","actions":[{"key":"open","label":"Open"}]});
        let first = serde_json::json!({"notification":item,"history":[item]});
        assert!(runtime.set_notifications_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_notifications_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[0]===seen[1] && seen[0].notification===seen[0].history[0] && Object.isFrozen(seen[0]) && Object.isFrozen(seen[0].history) && Object.isFrozen(seen[0].notification.actions)").unwrap());
        let hidden = serde_json::json!({"notification":item,"history":[item],"visible":false});
        assert!(runtime.set_notifications_store(&hidden).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[1].notification===seen[2].notification && seen[2].visible===false && __notificationsStore.generation===2").unwrap());
        let replaced = serde_json::json!({"notification":{"id":1,"appName":"Mail","summary":"Updated","body":"Body","actions":[]},"history":[]});
        runtime.set_notifications_store(&replaced).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(seen[3].notification.summary)")
                .unwrap(),
            "Updated"
        );
        runtime
            .set_notifications_store(&serde_json::json!({"notification":null,"history":[]}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<u64>("__notificationsStore.generation")
                .unwrap(),
            4
        );
    }

    #[test]
    fn notification_updates_are_isolated_and_rejected_subscriptions_roll_back() {
        let source = r#"
            globalThis.runs={app:0,notifications:0,applications:0,windows:0,sibling:0};
            function Notifications(){runs.notifications++;return h(Text,null,useNotifications(value=>value.notification?.summary??'none'))}
            function Applications(){runs.applications++;return h(Text,null,String(useApplications().length))}
            function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Notifications),h(Applications),h(Windows),h(Sibling))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.set_notifications_store(&serde_json::json!({"notification":{"id":1,"appName":"App","summary":"One","body":"","actions":[]},"history":[]})).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"notifications":2,"applications":1,"windows":1,"sibling":1})
        );

        let mut rejected = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useNotifications().visible))}",
            None,
        )
        .unwrap();
        rejected
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        rejected
            .set_notifications_store(
                &serde_json::json!({"notification":null,"history":[],"visible":true}),
            )
            .unwrap();
        assert!(
            !rejected
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
        );
    }

    #[test]
    fn workspaces_store_separates_observation_generation_from_mutation_revision() {
        let mut runtime=super::JsxRuntime::new("globalThis.seen=[];function App(){const all=useWorkspaces();const active=useWorkspace();seen.push({all,active});return h(Text,null,active?.id??'none')}",None).unwrap();
        let first = serde_json::json!({"available":true,"revision":"topology-1","workspaces":[{"id":"1","active":true},{"id":"2","active":false}],"activeWorkspace":"1","operations":{"switch":true,"create":true,"remove":true}});
        assert!(runtime.set_workspaces_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_workspaces_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[0].all===seen[1].all && seen[0].active===seen[1].active && Object.isFrozen(seen[0].all) && Object.isFrozen(seen[0].all.workspaces[0])").unwrap());
        let locked = serde_json::json!({"available":true,"revision":"topology-1","workspaces":[{"id":"1","active":true},{"id":"2","active":false}],"activeWorkspace":"1","operations":{}});
        assert!(runtime.set_workspaces_store(&locked).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[1].active===seen[2].active && seen[2].all.revision==='topology-1' && seen[2].all.generation===2 && seen[2].all.writable===false").unwrap());
        let switched = serde_json::json!({"available":true,"revision":"topology-2","workspaces":[{"id":"1","active":false},{"id":"2","active":true}],"activeWorkspace":"2","operations":{"switch":true}});
        runtime.set_workspaces_store(&switched).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(seen[3].active.id)")
                .unwrap(),
            "2"
        );
    }

    #[test]
    fn workspace_updates_are_isolated_and_rejected_subscriptions_roll_back() {
        let source = "globalThis.runs={app:0,workspace:0,windows:0,sibling:0};function Workspace(){runs.workspace++;return h(Text,null,useWorkspace()?.id??'none')}function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}function Sibling(){runs.sibling++;return h(Text,null,'stable')}function App(){runs.app++;return h(Window,{},h(Workspace),h(Windows),h(Sibling))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime.set_workspaces_store(&serde_json::json!({"available":true,"revision":"one","workspaces":[{"id":"1","active":true}],"operations":{}})).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"workspace":2,"windows":1,"sibling":1})
        );
        let mut rejected = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useWorkspaces().available))}",
            None,
        )
        .unwrap();
        rejected
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        rejected.set_workspaces_store(&serde_json::json!({"available":false,"reason":"absent","workspaces":[],"operations":{}})).unwrap();
        assert!(
            !rejected
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
        );
    }

    #[test]
    fn outputs_store_versions_unavailable_transitions_and_resolves_surface_output() {
        let output = serde_json::json!({"name":"DP-1","model":"Panel","geometry":{"x":0,"y":0,"width":1920,"height":1080},"work_area":{"x":0,"y":0,"width":1920,"height":1040},"scale_120":120,"transform":"normal","physical_width_mm":500,"physical_height_mm":300,"primary":true,"enabled":true,"modes":[{"width":1920,"height":1080,"refresh_millihz":60000}],"current_mode":{"width":1920,"height":1080,"refresh_millihz":60000}});
        let mut runtime=super::JsxRuntime::new("globalThis.seen=[];function App(){const all=useOutputs();const current=useOutput();seen.push({all,current});return h(Text,null,current?.name??'none')}",None).unwrap();
        runtime
            .set_surface_store("mount", &serde_json::json!({"output":"DP-1"}))
            .unwrap();
        let first = serde_json::json!({"available":true,"revision":"layout-1","outputs":[output]});
        assert!(runtime.set_outputs_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(!runtime.set_outputs_store(&first).unwrap());
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[0].all===seen[1].all && seen[0].current===seen[1].current && seen[0].current.name==='DP-1' && Object.isFrozen(seen[0].current)").unwrap());
        runtime.set_outputs_store(&serde_json::json!({"available":false,"reason":"backend gone","revision":"layout-1","outputs":[]})).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert!(runtime.eval_json::<bool>("seen[2].current===null && seen[2].all.generation===2 && seen[2].all.revision==='layout-1'").unwrap());
    }

    #[test]
    fn output_updates_are_isolated_and_rejected_subscriptions_roll_back() {
        let source = "globalThis.runs={app:0,outputs:0,windows:0,sibling:0};function Outputs(){runs.outputs++;return h(Text,null,String(useOutputs().available))}function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}function Sibling(){runs.sibling++;return h(Text,null,'stable')}function App(){runs.app++;return h(Window,{},h(Outputs),h(Windows),h(Sibling))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_outputs_store(&serde_json::json!({"available":false,"reason":"none","outputs":[]}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"outputs":2,"windows":1,"sibling":1})
        );
        let mut rejected = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useOutputs().available))}",
            None,
        )
        .unwrap();
        rejected
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        rejected
            .set_outputs_store(&serde_json::json!({"available":false,"reason":"none","outputs":[]}))
            .unwrap();
        assert!(
            !rejected
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
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
    fn locale_updates_are_isolated_and_rejected_subscriptions_roll_back() {
        let source = "globalThis.runs={app:0,locale:0,windows:0,sibling:0};function Locale(){runs.locale++;return h(Text,null,useLocale().tag)}function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}function Sibling(){runs.sibling++;return h(Text,null,'stable')}function App(){runs.app++;return h(Window,{},h(Locale),h(Windows),h(Sibling))}";
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_locale_store(&serde_json::json!({"known":true,"tag":"fr-FR","direction":"ltr"}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"locale":2,"windows":1,"sibling":1})
        );
        let mut rejected =
            super::JsxRuntime::new("function App(){return h(Text,null,useLocale().tag)}", None)
                .unwrap();
        rejected
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        rejected
            .set_locale_store(&serde_json::json!({"known":true,"tag":"de-DE","direction":"ltr"}))
            .unwrap();
        assert!(
            !rejected
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
        );
    }

    #[test]
    fn sync_external_store_accepts_only_branded_contracts_and_tracks_generation() {
        let source = r#"
            globalThis.runs={app:0,store:0,sibling:0};globalThis.seen=[];
            function Store(){runs.store++;const value=useSyncExternalStore(NickelStores.locale.subscribe,NickelStores.locale.getSnapshot);seen.push(value);return h(Text,null,value.tag)}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Store),h(Sibling))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_locale_store(&serde_json::json!({"known":true,"tag":"en-US","direction":"ltr"}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"store":2,"sibling":1})
        );
        assert!(
            runtime
                .eval_json::<bool>("seen[1]===NickelStores.locale.getSnapshot()")
                .unwrap()
        );
        assert!(runtime.eval("function Bad(){return h(Text,null,String(useSyncExternalStore(()=>()=>{},()=>0)))}__nickelSetApp(Bad);__nickelRender()").is_err());
        assert!(
            runtime
                .eval("NickelStores.locale.subscribe(()=>{})")
                .is_err()
        );
    }

    #[test]
    fn rejected_sync_store_render_does_not_install_subscription() {
        let mut runtime=super::JsxRuntime::new("function App(){return h(Text,null,useSyncExternalStore(NickelStores.locale.subscribe,NickelStores.locale.getSnapshot).tag)}",None).unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime
            .set_locale_store(&serde_json::json!({"known":true,"tag":"fr-FR","direction":"ltr"}))
            .unwrap();
        assert!(
            !runtime
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
                .unwrap()
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
        let source = "function Child({value}){return h(Text,null,String(value))}const Memo=memo(Child);function App(){const [value,setValue]=useState(nickel.data.initial);return h(Button,{onClick:()=>setValue(1)},h(Memo,{value}))}";
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
    fn theme_store_dirties_only_changed_theme_selections() {
        let source = r#"
            globalThis.runs={app:0,mode:0,motion:0,windows:0,sibling:0};
            function Mode(){runs.mode++;return h(Text,null,useTheme(theme=>theme.mode))}
            function Motion(){runs.motion++;return h(Text,null,String(useReducedMotion()))}
            function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Mode),h(Motion),h(Windows),h(Sibling))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        runtime
            .set_theme_store(&serde_json::json!({"mode":"dark","reducedMotion":null}))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"mode":2,"motion":1,"windows":1,"sibling":1})
        );
        runtime
            .set_windows_store(&serde_json::json!([{"id":"one"}]))
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"mode":2,"motion":1,"windows":2,"sibling":1})
        );
    }

    #[test]
    fn appearance_projection_feeds_effective_theme_without_fabricating_palette() {
        let mut runtime = super::JsxRuntime::new(
            "globalThis.seen=[];function App(){seen.push([useTheme(),useReducedMotion()]);return h(Text,null,'theme')}",
            Some(r#"{"appearance":{"available":true,"configured":{"animations":"reduced","reduce_transparency":true},"resolved":{"theme":"light","hue":210,"intensity":55,"accent":[1,2,3]}}}"#),
        ).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(seen[0])")
                .unwrap(),
            serde_json::json!([{"generation":1,"mode":"light","accent":4278256131u64,"accentHue":210,
                "accentIntensity":55,"reducedMotion":true,"reducedTransparency":true,"palette":null},true])
        );
    }

    #[test]
    fn capability_names_are_checked_against_the_rust_wire_enum() {
        for &name in super::PLUGIN_CAPABILITY_NAMES {
            let capability: nickel_core::plugins::PluginCapability =
                serde_json::from_value(serde_json::Value::String(name.into())).unwrap();
            assert_eq!(capability.as_str(), name);
        }
    }

    #[test]
    fn capability_store_is_owner_scoped_versioned_and_rejects_unknown_names() {
        use nickel_core::plugins::PluginCapability;
        let source = r#"
            globalThis.seen=[];globalThis.runs={app:0,windows:0,audio:0};
            function Windows(){runs.windows++;const value=useCapability('windows-read');seen.push(value);return h(Text,null,String(value.available))}
            function Audio(){runs.audio++;return h(Text,null,String(useCapability('audio-read').declared))}
            function App(){runs.app++;return h(Window,{},h(Windows),h(Audio))}
        "#;
        let mut runtime = super::JsxRuntime::new(source, None).unwrap();
        let grants = [PluginCapability::WindowsRead];
        assert!(
            runtime
                .set_capability_store(&grants, &serde_json::json!({"windows":[]}))
                .unwrap()
        );
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(seen[0])")
                .unwrap(),
            serde_json::json!({"declared":true,"available":true,"reason":null})
        );
        assert!(runtime.eval_json::<bool>("Object.isFrozen(__capabilityStore.snapshot) && Object.isFrozen(seen[0]) && __capabilityStore.generation === 2").unwrap());
        assert!(
            !runtime
                .set_capability_store(&grants, &serde_json::json!({"windows":[]}))
                .unwrap()
        );
        runtime
            .set_capability_store(
                &grants,
                &serde_json::json!({"windows":{"available":false,"reason":"backend absent"}}),
            )
            .unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>("JSON.stringify(runs)")
                .unwrap(),
            serde_json::json!({"app":1,"windows":2,"audio":1})
        );
        assert!(runtime.eval("function Unknown(){useCapability('future-root')} __nickelSetApp(Unknown); __nickelRender()").is_err());
    }

    #[test]
    fn rejected_capability_consumer_does_not_install_a_subscription() {
        use nickel_core::plugins::PluginCapability;
        let mut runtime = super::JsxRuntime::new(
            "function App(){return h(Text,null,String(useCapability('audio-read').available))}",
            None,
        )
        .unwrap();
        runtime
            .set_capability_store(
                &[PluginCapability::AudioRead],
                &serde_json::json!({"audio":{"available":true}}),
            )
            .unwrap();
        runtime
            .render("__nickelRender()", |_| Err::<(), _>("reject".into()))
            .unwrap_err();
        runtime
            .set_capability_store(
                &[PluginCapability::AudioRead],
                &serde_json::json!({"audio":{"available":false}}),
            )
            .unwrap();
        assert!(
            !runtime
                .eval_json::<bool>("JSON.parse(__nickelReconciliationRequest()).requested")
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
            function Consumer(){const value=useContext(Value);seen.push(nickel.data.surface.id+':'+value);return h(Text,null,value)}
            function App(){return h(Value.Provider,{value:nickel.data.surface.id},h(Consumer))}
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
        let mut runtime=super::JsxRuntime::new("function App(){const [state]=useState({count:0});return h(Button,{onClick:()=>{state.count++;globalThis.outside=(globalThis.outside||0)+1;nickel.request('show-launcher');}},String(state.count));}",None).unwrap();
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
    fn session_client_copies_account_and_captures_revision_without_private_ui_requests() {
        let data = r#"{"session":{"revision":"current","account":{"displayName":"Ada","username":"ada"},"locked":false,"support":{"lock":true,"logout":true,"suspend":true,"reboot":true,"powerOff":true,"restartShell":false}}}"#;
        let mut runtime = JsxRuntime::new("", Some(data)).unwrap();
        runtime.eval("nickel.session.get().account.displayName='changed';nickel.session.lock();nickel.session.logout();nickel.session.suspend();nickel.session.reboot();nickel.session.powerOff()").unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(nickel.session.get().account)")
                .unwrap()["displayName"],
            "Ada"
        );
        assert!(runtime.eval("nickel.session.restartShell()").is_err());
        let effects = runtime.take_effects().unwrap();
        assert_eq!(effects.len(), 5);
        assert!(
            effects.iter().all(
                |effect| effect["type"] == "session.perform" && effect["revision"] == "current"
            )
        );
        runtime.set_data(r#"{"session":{"revision":"locked","account":null,"locked":true,"support":{"lock":true,"logout":true}}}"#).unwrap();
        assert!(runtime.eval("nickel.session.logout()").is_err());
        assert!(runtime.take_effects().unwrap().is_empty());
        let mut absent = JsxRuntime::new("", None).unwrap();
        assert!(absent.eval("nickel.session.lock()").is_err());
    }

    #[test]
    fn native_wallpaper_chooser_captures_revision_without_exposing_paths() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"wallpaper":{"available":true,"writable":true,"generation":8,"configured":{"custom_image_configured":false,"position":"fill"},"images":[],"chooser":{"available":true,"pending":false}}}"#)).unwrap();
        runtime.eval("nickel.wallpaper.chooseImage()").unwrap();
        let effects = runtime.take_effects().unwrap();
        assert_eq!(
            effects,
            vec![
                serde_json::json!({"type":"wallpaper.chooseImage","transaction":{"generation":8,"prior":{"custom_image_configured":false,"position":"fill"}}})
            ]
        );
        for blocked in [
            r#"{"wallpaper":{"available":true,"writable":false,"generation":8,"chooser":{"available":true,"pending":false}}}"#,
            r#"{"wallpaper":{"available":true,"writable":true,"generation":8,"chooser":{"available":true,"pending":true}}}"#,
            r#"{}"#,
        ] {
            runtime.set_data(blocked).unwrap();
            assert!(runtime.eval("nickel.wallpaper.chooseImage()").is_err());
            assert!(runtime.take_effects().unwrap().is_empty());
        }
    }

    #[test]
    fn appearance_clients_copy_preferences_and_capture_observed_transactions() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"appearance":{"available":true,"writable":true,"generation":7,"configured":{"theme":"system","accent_hue":null,"accent_intensity":null,"reduce_transparency":false,"animations":"normal"}},"wallpaper":{"available":true,"writable":true,"generation":8,"configured":{"custom_image_configured":false,"position":"fill"},"images":[{"id":"approved"}]}}"#)).unwrap();
        runtime.eval("let preferences = nickel.appearance.get().configured; preferences.accent_hue = 271; preferences.accent_intensity = 63; nickel.appearance.set(preferences); preferences.accent_hue = 0; nickel.wallpaper.selectImage('approved'); nickel.wallpaper.setPosition('fit'); nickel.wallpaper.resetCustomImage();").unwrap();
        let effects = runtime.take_effects().unwrap();
        assert_eq!(effects[0]["type"], "appearance.set");
        assert_eq!(effects[0]["transaction"]["generation"], 7);
        assert!(effects[0]["transaction"]["prior"]["accent_hue"].is_null());
        assert_eq!(effects[0]["transaction"]["requested"]["accent_hue"], 271);
        assert_eq!(
            effects[0]["transaction"]["requested"]["accent_intensity"],
            63
        );
        assert_eq!(effects[1]["transaction"]["change"]["image_id"], "approved");
        assert_eq!(effects[2]["transaction"]["change"]["position"], "fit");
        assert_eq!(
            effects[3]["transaction"]["change"]["kind"],
            "reset_custom_image"
        );
        assert_eq!(
            runtime
                .eval_json::<serde_json::Value>(
                    "JSON.stringify(nickel.appearance.get().configured.accent_hue)"
                )
                .unwrap(),
            serde_json::Value::Null
        );
        runtime.set_data(r#"{}"#).unwrap();
        assert!(runtime.eval("nickel.appearance.set({})").is_err());
        assert!(
            runtime
                .eval("nickel.wallpaper.selectImage('approved')")
                .is_err()
        );
    }

    #[test]
    fn native_ui_clients_emit_service_operations_and_copy_clock_snapshot() {
        let mut runtime = super::JsxRuntime::new(
            "",
            Some(r#"{"clock":{"unixMilliseconds":1770000000000,"utcOffsetMinutes":-420}}"#),
        )
        .unwrap();
        runtime.eval("nickel.clock.get().utcOffsetMinutes = 0; nickel.projects.show(); nickel.projects.toggle(); nickel.keyboard.toggle();").unwrap();
        assert_eq!(
            runtime
                .eval_json::<i32>("JSON.stringify(nickel.clock.get().utcOffsetMinutes)")
                .unwrap(),
            -420
        );
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"projects.show"}),
                serde_json::json!({"type":"projects.toggle"}),
                serde_json::json!({"type":"keyboard.toggle"})
            ]
        );
    }

    #[test]
    fn run_client_captures_revision_and_preserves_parsed_command_text() {
        let mut runtime = super::JsxRuntime::new(
            "",
            Some(r#"{"run":{"available":true,"revision":"owner:4","status":null}}"#),
        )
        .unwrap();
        runtime
            .eval(r#"nickel.run.execute(' editor "a b" ');"#)
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"run.execute","command":"editor \"a b\"","revision":"owner:4"})
            ]
        );
        for source in [
            "nickel.run.execute('')",
            "nickel.run.execute('a'.repeat(4097))",
            "nickel.run.execute('abc', '')",
        ] {
            assert!(runtime.eval(source).is_err());
        }
        let mut unavailable = super::JsxRuntime::new("", None).unwrap();
        assert!(unavailable.eval("nickel.run.execute('editor')").is_err());
        assert!(unavailable.take_effects().unwrap().is_empty());
    }

    #[test]
    fn notification_client_uses_stable_ids_and_validates_actions() {
        let mut runtime = super::JsxRuntime::new(
            "",
            Some(r#"{"notifications":{"notification":{"id":7,"summary":"Mail"},"history":[]}}"#),
        )
        .unwrap();
        runtime.eval("nickel.notifications.get().notification.summary = 'changed'; nickel.notifications.invoke(7, 'open'); nickel.notifications.dismiss(7);").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>(
                    "JSON.stringify(nickel.notifications.get().notification.summary)"
                )
                .unwrap(),
            "Mail"
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(
            effects[0],
            serde_json::json!({"type":"notifications.invoke","id":7,"key":"open"})
        );
        assert_eq!(
            effects[1],
            serde_json::json!({"type":"notifications.dismiss","id":7})
        );
        for call in [
            "nickel.notifications.dismiss(0)",
            "nickel.notifications.dismiss('7')",
            "nickel.notifications.dismiss(4294967296)",
            "nickel.notifications.invoke(7, '')",
        ] {
            assert!(runtime.eval(call).is_err());
        }
        assert!(runtime.take_effects().unwrap().is_empty());
    }

    #[test]
    fn desktop_clients_copy_snapshots_and_emit_stable_identity_actions() {
        let mut runtime = super::JsxRuntime::new(
            "",
            Some(r#"{"windows":[{"id":"42","title":"Editor"}],"applications":[{"id":"editor"}]}"#),
        )
        .unwrap();
        runtime.eval("nickel.windows.list()[0].title = 'changed'; nickel.windows.activate('42'); nickel.applications.launch('editor'); nickel.tray.activate('mail');").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(nickel.windows.list()[0].title)")
                .unwrap(),
            "Editor"
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(effects[0]["type"], "windows.focus");
        assert_eq!(effects[1]["id"], "editor");
        assert_eq!(effects[2]["type"], "tray.activate");
        assert!(runtime.eval("nickel.windows.activate(42)").is_err());
        assert!(runtime.eval("nickel.audio.setVolume(101)").is_err());
        runtime.eval("nickel.audio.setVolume(25)").unwrap();
        assert_eq!(runtime.take_effects().unwrap()[0]["value"], 25);
        assert!(
            runtime
                .eval("nickel.applications.movePin('editor', 0)")
                .is_err()
        );
        runtime
            .eval("nickel.applications.movePin('editor', -1)")
            .unwrap();
        assert_eq!(runtime.take_effects().unwrap()[0]["direction"], -1);
    }

    #[test]
    fn preferences_clients_copy_snapshots_and_emit_only_requested_patch_fields() {
        let data = serde_json::json!({"preferences":{"available":true,"writable":true,"revision":"0123456789abcdef","configured":{"barOnAllDisplays":true,"desktopCount":4}}}).to_string();
        let mut runtime = super::JsxRuntime::new("", Some(&data)).unwrap();
        runtime.eval("nickel.preferences.get().configured.desktopCount=9; nickel.preferences.set({desktopCount:6});").unwrap();
        assert_eq!(
            runtime
                .eval_json::<u8>("JSON.stringify(nickel.preferences.get().configured.desktopCount)")
                .unwrap(),
            4
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(effects[0]["type"], "preferences.set");
        assert_eq!(effects[0]["transaction"]["revision"], "0123456789abcdef");
        assert_eq!(
            effects[0]["transaction"]["changedFields"],
            serde_json::json!(["desktopCount"])
        );
        assert_eq!(effects[0]["transaction"]["prior"]["desktopCount"], 4);
        assert_eq!(effects[0]["transaction"]["requested"]["desktopCount"], 6);
        assert_eq!(
            effects[0]["transaction"]["requested"]["barOnAllDisplays"],
            true
        );
        assert!(
            runtime
                .eval("nickel.preferences.set({theme:'dark'})")
                .is_err()
        );
        assert!(runtime.eval("nickel.preferences.set({})").is_err());
        let mut denied = super::JsxRuntime::new("", None).unwrap();
        assert!(
            denied
                .eval("nickel.preferences.set({desktopCount:5})")
                .is_err()
        );
    }

    #[test]
    fn ordinary_plugins_page_registers_from_emitted_module_and_renders_inventory() {
        use super::{JsxModuleGraph, JsxRuntime, ModuleSource};
        let graph = JsxModuleGraph::new("entry.js", [
            ModuleSource {path:"entry.js",source:"import { Plugins } from './Plugins.js';\nexport default function App() { return h(Window,{id:'main',width:800,height:600},h(Plugins,{})); }"},
            ModuleSource {path:"Plugins.js",source:include_str!("../../../assets/plugins/nickel-default/src/Plugins.js")},
            ModuleSource {path:"styles/plugins.css",source:include_str!("../../../assets/plugins/nickel-default/src/styles/plugins.css")},
        ]).unwrap();
        let mut runtime = JsxRuntime::new_modules(&graph, Some(r#"{"plugins":{"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","name":"Example","enabled":false,"health":{"state":"running"},"grants":["windows-read"],"surfaces":[],"composition":[],"memory":{"jsHeapBytes":null,"nativeUiBytes":null,"textureBytes":null,"trackedPeakBytes":null,"timers":0,"subscriptions":0}}]}}"#)).unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "nickel-default")
            .unwrap();
        assert_eq!(
            registry.settings_pages_snapshot().pages[0].registration.id,
            "plugins"
        );
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let rendered = tree.to_string();
        assert!(rendered.contains("Example"));
        assert!(rendered.contains("Unavailable"));
        assert!(rendered.contains("Authorized capabilities"));
        fn find<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
            if node["id"] == id {
                return Some(node);
            }
            node["children"]
                .as_array()?
                .iter()
                .find_map(|child| find(child, id))
        }
        let action = find(&tree, "plugin-toggle-0").unwrap()["action"]
            .as_u64()
            .unwrap();
        let reviewed = runtime
            .render(&format!("__nickelDispatch({action})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert!(runtime.take_effects().unwrap().is_empty());
        let confirm = find(&reviewed, "plugin-review-confirm").unwrap();
        assert_ne!(confirm["disabled"], serde_json::json!(true));
        let action = confirm["action"].as_u64().unwrap();
        runtime
            .render(&format!("__nickelDispatch({action})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"plugins.enable","id":"example","revision":"7","priorEnabled":false})
            ]
        );
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let action = find(&tree, "plugin-toggle-0").unwrap()["action"]
            .as_u64()
            .unwrap();
        runtime
            .render(&format!("__nickelDispatch({action})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        let mut data: serde_json::Value = runtime.eval_json("JSON.stringify(nickel.data)").unwrap();
        data["plugins"]["revision"] = serde_json::json!("8");
        runtime.set_data(&data.to_string()).unwrap();
        let stale = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(
            find(&stale, "plugin-review-confirm").unwrap()["disabled"],
            serde_json::json!(true)
        );
        assert!(runtime.take_effects().unwrap().is_empty());
    }

    #[test]
    fn plugin_metadata_edits_capture_prior_values_and_lossless_revision() {
        let mut runtime=super::JsxRuntime::new("",Some(r#"{"plugins":{"available":true,"writable":true,"revision":"9007199254740993","plugins":[{"id":"example","settings":[{"id":"count","value":2}]}]},"system":{"available":true,"version":"0.1.0","platform":"linux","architecture":"x86_64"}}"#)).unwrap();
        runtime.eval("nickel.plugins.setSetting('example','count',3,'9007199254740993');nickel.system.get().version='mutated';").unwrap();
        assert_eq!(
            runtime.take_effects().unwrap()[0],
            serde_json::json!({"type":"plugins.setSetting","id":"example","key":"count","revision":"9007199254740993","priorValue":2,"value":3})
        );
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(nickel.system.get().version)")
                .unwrap(),
            "0.1.0"
        );
        assert!(
            runtime
                .eval("nickel.plugins.setSetting('example','unknown',3,'9007199254740993')")
                .is_err()
        );
        assert!(
            runtime
                .eval("nickel.plugins.setSetting('example','count',{},'9007199254740993')")
                .is_err()
        );
        assert!(
            runtime
                .eval("nickel.plugins.setSetting('example','count',3,'1')")
                .is_err()
        );
    }

    #[test]
    fn shell_selection_client_checks_declared_shell_and_inventory_revision() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"plugins":{"available":true,"writable":true,"revision":"8","plugins":[{"id":"theme","shell":true},{"id":"tool","shell":false}]}}"#)).unwrap();
        runtime
            .eval("nickel.plugins.selectShell('theme','8')")
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"plugins.selectShell","id":"theme","revision":"8"})]
        );
        assert!(
            runtime
                .eval("nickel.plugins.selectShell('tool','8')")
                .is_err()
        );
        assert!(
            runtime
                .eval("nickel.plugins.selectShell('theme','7')")
                .is_err()
        );
        let mut denied = super::JsxRuntime::new("", None).unwrap();
        assert!(
            denied
                .eval("nickel.plugins.selectShell('theme','8')")
                .is_err()
        );
    }

    #[test]
    fn plugins_clients_copy_inventory_and_emit_guarded_lifecycle_requests() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"plugins":{"available":true,"writable":true,"revision":"9007199254740993","plugins":[{"id":"example","enabled":true,"memory":{"jsHeapBytes":null}}]}}"#)).unwrap();
        runtime.eval("nickel.plugins.list()[0].enabled=false; nickel.plugins.disable('example','9007199254740993');").unwrap();
        assert!(
            runtime
                .eval_json::<bool>("JSON.stringify(nickel.plugins.list()[0].enabled)")
                .unwrap()
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(
            effects[0],
            serde_json::json!({"type":"plugins.disable","id":"example","revision":"9007199254740993","priorEnabled":true})
        );
        assert!(
            runtime
                .eval("nickel.plugins.enable('unknown','9007199254740993')")
                .is_err()
        );
        assert!(
            runtime
                .eval("nickel.plugins.enable('example','1')")
                .is_err()
        );
        assert!(
            runtime
                .eval("nickel.plugins.enable('example',9007199254740993)")
                .is_err()
        );
        let mut denied = super::JsxRuntime::new("", None).unwrap();
        assert!(denied.eval("nickel.plugins.enable('example','1')").is_err());
        assert!(
            !denied
                .eval_json::<bool>("JSON.stringify(nickel.plugins.get().available)")
                .unwrap()
        );
    }

    #[test]
    fn associations_clients_copy_snapshots_and_emit_expected_revision_and_handler() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"associations":{"available":true,"revision":"9007199254740993","targets":[{"id":"mime:text/plain","capability":"nativeConsent","canSetDefault":true,"protected":false,"effectiveHandlerId":"old.desktop","handlers":[{"id":"new.desktop","name":"New","protected":false},{"id":"protected.desktop","protected":true}]}]}}"#)).unwrap();
        runtime.eval("nickel.associations.getHandlers('mime:text/plain').handlers[0].name = 'mutated'; nickel.associations.setDefault('mime:text/plain','new.desktop','9007199254740993'); nickel.associations.openSystemSettings();").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>(
                    "JSON.stringify(nickel.associations.list()[0].handlers[0].name)"
                )
                .unwrap(),
            "New"
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(
            effects[0],
            serde_json::json!({"type":"associations.setDefault","targetId":"mime:text/plain","handlerId":"new.desktop","revision":"9007199254740993","expectedHandlerId":"old.desktop"})
        );
        assert_eq!(effects[1]["type"], "associations.openSystemSettings");
        assert!(runtime.eval("nickel.associations.setDefault('mime:text/plain','new.desktop',9007199254740993)").is_err());
        assert!(
            runtime
                .eval("nickel.associations.setDefault('mime:text/plain','new.desktop','1')")
                .is_err()
        );
        assert!(runtime.eval("nickel.associations.setDefault('mime:text/plain','protected.desktop','9007199254740993')").is_err());
        let mut denied = super::JsxRuntime::new("", None).unwrap();
        assert_eq!(
            denied
                .eval_json::<serde_json::Value>("JSON.stringify(nickel.associations.get())")
                .unwrap()["available"],
            false
        );
        assert!(
            denied
                .eval("nickel.associations.getHandlers('mime:text/plain')")
                .is_err()
        );
    }

    #[test]
    fn ordinary_connectivity_pages_preserve_disconnect_pair_and_native_details() {
        let graph = super::JsxModuleGraph::new("entry.js",[
            super::ModuleSource {path:"entry.js",source:"import { Wifi } from './Wifi.js';\nimport { Bluetooth } from './Bluetooth.js';\nexport default function App(){return h(Column,{},h(Wifi,{}),h(Bluetooth,{}));}"},
            super::ModuleSource {path:"Wifi.js",source:include_str!("../../../assets/plugins/nickel-default/src/Wifi.js")},
            super::ModuleSource {path:"Bluetooth.js",source:include_str!("../../../assets/plugins/nickel-default/src/Bluetooth.js")},
            super::ModuleSource {path:"styles/connectivity.css",source:include_str!("../../../assets/plugins/nickel-default/src/styles/connectivity.css")},
        ]).unwrap();
        let mut runtime = JsxRuntime::new_modules(&graph,Some(r#"{"wifi":{"available":true,"enabled":true,"revision":"0123456789abcdef","operations":{"disconnect":true},"adaptersAvailable":true,"adapters":[{"name":"eth0","description":"Ethernet","connected":true,"speedBitsPerSecond":null}],"networks":[{"id":"profile","name":"SSID","connected":true,"canDisconnect":true,"signalPercent":80}]},"bluetooth":{"available":true,"powered":true,"revision":"fedcba9876543210","adapterName":"Native radio","operations":{"pair":true},"devices":[{"id":"device","name":"Headset","paired":false,"connected":false,"batteryPercent":75,"signalDbm":-42,"kind":"audio-card"}]}}"#)).unwrap();
        fn find<'a>(node: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
            if node["id"] == id {
                return Some(node);
            }
            node["children"]
                .as_array()?
                .iter()
                .find_map(|child| find(child, id))
        }
        let tree = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        assert!(tree.to_string().contains("Battery: 75%"));
        assert!(tree.to_string().contains("eth0"));
        let action = find(&tree, "settings-wifi-disconnect/profile").unwrap()["action"]
            .as_u64()
            .unwrap();
        let tree = runtime
            .render(&format!("__nickelDispatch({action})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        let action = find(&tree, "settings-bluetooth-pair/device").unwrap()["action"]
            .as_u64()
            .unwrap();
        runtime
            .render(&format!("__nickelDispatch({action})"), |node| {
                Ok(node.clone())
            })
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"wifi.disconnect","id":"profile","revision":"0123456789abcdef"}),
                serde_json::json!({"type":"bluetooth.pair","id":"device","revision":"fedcba9876543210"})
            ]
        );
    }

    #[test]
    fn connectivity_clients_copy_snapshots_and_emit_revision_bound_effects() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"wifi":{"available":true,"revision":"0123456789abcdef","operations":{"connect":true,"disconnect":true,"setEnabled":true},"networks":[{"id":"stable-profile","name":"SSID"}]},"bluetooth":{"available":true,"revision":"fedcba9876543210","operations":{"connect":true},"devices":[{"id":"stable-device"}]}}"#)).unwrap();
        runtime.eval("nickel.wifi.listNetworks()[0].name = 'mutated'; nickel.wifi.connect('stable-profile'); nickel.bluetooth.connect('stable-device'); nickel.wifi.disconnect('stable-profile');").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(nickel.wifi.listNetworks()[0].name)")
                .unwrap(),
            "SSID"
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(
            effects[0],
            serde_json::json!({"type":"wifi.connect","id":"stable-profile","revision":"0123456789abcdef"})
        );
        assert_eq!(effects[1]["id"], "stable-device");
        assert_eq!(
            effects[2],
            serde_json::json!({"type":"wifi.disconnect","id":"stable-profile","revision":"0123456789abcdef"})
        );
        assert!(runtime.eval("nickel.wifi.setEnabled('yes')").is_err());
        assert!(
            runtime
                .eval("nickel.bluetooth.disconnect('stable-device')")
                .is_err()
        );
        let mut denied = super::JsxRuntime::new("", None).unwrap();
        assert!(
            denied
                .eval("nickel.wifi.connect('stable-profile')")
                .is_err()
        );
        assert!(
            !denied
                .eval_json::<bool>("nickel.bluetooth.get().available")
                .unwrap()
        );
    }

    #[test]
    fn application_search_client_emits_bounded_requests_and_copies_results() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"applicationSearch":{"available":true,"query":"ed","results":[{"id":"editor","name":"Editor"}],"total":1}}"#)).unwrap();
        runtime.eval("nickel.applications.searchResults().results[0].name='mutated'; nickel.applications.search('ed');").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>(
                    "JSON.stringify(nickel.applications.searchResults().results[0].name)"
                )
                .unwrap(),
            "Editor"
        );
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"applications.search","query":"ed"})]
        );
        assert!(
            runtime
                .eval("nickel.applications.search('x'.repeat(513))")
                .is_err()
        );
        assert!(runtime.eval("nickel.applications.search(12)").is_err());
    }

    #[test]
    fn optional_feature_clients_copy_observations_and_capture_native_revisions() {
        let revision = "a".repeat(64);
        let data = serde_json::json!({"features":{"available":true,"revision":revision,"operations":{"setKeyboardMode":true,"setCodexEnabled":true},"keyboard":{"mode":"automatic"}},"shortcuts":{"available":true,"editable":false,"shortcuts":[{"id":"launcher"}]}});
        let mut runtime = super::JsxRuntime::new("", Some(&data.to_string())).unwrap();
        runtime.eval("nickel.features.get().keyboard.mode='changed'; nickel.features.setKeyboardMode('enabled'); nickel.features.setCodexEnabled(false,true);").unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>("JSON.stringify(nickel.features.get().keyboard.mode)")
                .unwrap(),
            "automatic"
        );
        let effects = runtime.take_effects().unwrap();
        assert_eq!(effects[0]["revision"], revision);
        assert_eq!(effects[1]["confirmed"], true);
        assert!(
            runtime
                .eval("nickel.features.setKeyboardMode('other')")
                .is_err()
        );
        assert!(runtime.eval("nickel.features.retryCodex()").is_err());
        assert!(
            !runtime
                .eval_json::<bool>("nickel.shortcuts.get().editable")
                .unwrap()
        );
    }

    use super::*;

    #[test]
    fn surface_clients_emit_owned_surface_requests_and_bound_placement() {
        let mut runtime = JsxRuntime::new("", None).unwrap();
        runtime.eval("nickel.surfaces.show('settings'); nickel.surfaces.focus('settings'); nickel.surfaces.setPlacement('settings', {anchor:'bottom-right',offsetX:-16}); nickel.surfaces.hide('settings')").unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"surface.show","surfaceId":"settings"}),
                serde_json::json!({"type":"surface.focus","surfaceId":"settings"}),
                serde_json::json!({"type":"surface.setPlacement","surfaceId":"settings","anchor":"bottom-right","offsetX":-16,"offsetY":0}),
                serde_json::json!({"type":"surface.hide","surfaceId":"settings"}),
            ]
        );
        assert!(runtime.eval("nickel.surfaces.show('')").is_err());
        assert!(
            runtime
                .eval("nickel.surfaces.setPlacement('settings', {anchor:'center',offsetX:8193})")
                .is_err()
        );
        assert!(runtime.take_effects().unwrap().is_empty());
    }

    #[test]
    fn slider_numeric_ranges_normalize_native_values_and_quantize_changes() {
        let mut runtime = JsxRuntime::new("function App() { return h(Slider, {id:'volume', accessibilityLabel:'Volume', min:10, max:110, step:5, value:60, onChange:value=>nickel.request({type:'changed',value})}); }", None).unwrap();
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
        let source = "function App() { return h('div', {id: 'target', onDrop: event => nickel.request({type: 'dropped', ...event})}); }";
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
    fn displays_application_scale_and_identify_capture_current_revisions() {
        let mut runtime = JsxRuntime::new("", None).unwrap();
        assert!(
            runtime
                .eval("nickel.displays.setApplicationScale({policy:'follow'})")
                .is_err()
        );
        runtime.set_data(r#"{"displays":{"revision":"0123456789abcdef","operations":{"identify":true},"application_scale":{"available":true,"revision":"fedcba9876543210","configured":{"policy":"follow"},"supported_scales":[120,180]}}}"#).unwrap();
        runtime
            .eval("nickel.displays.getApplicationScale().configured.policy='custom'")
            .unwrap();
        assert_eq!(
            runtime
                .eval_json::<String>(
                    "JSON.stringify(nickel.displays.getApplicationScale().configured.policy)"
                )
                .unwrap(),
            "follow"
        );
        for script in [
            "nickel.displays.setApplicationScale({policy:'custom',scale_120:181})",
            "nickel.displays.setApplicationScale({policy:'custom',scale_120:180.5})",
            "nickel.displays.setApplicationScale({policy:'follow'},'0123456789abcdef')",
            "nickel.displays.identify('fedcba9876543210')",
        ] {
            assert!(runtime.eval(script).is_err());
        }
        assert!(runtime.take_effects().unwrap().is_empty());
        runtime.eval("nickel.displays.setApplicationScale({policy:'custom',scale_120:180});nickel.displays.identify()").unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({"type":"displays.setApplicationScale","revision":"fedcba9876543210","policy":{"policy":"custom","scale_120":180}}),
                serde_json::json!({"type":"displays.identify","revision":"0123456789abcdef"})
            ]
        );
        runtime
            .set_data(
                r#"{"displays":{"revision":"0123456789abcdef","operations":{"identify":false}}}"#,
            )
            .unwrap();
        assert!(runtime.eval("nickel.displays.identify()").is_err());
    }

    #[test]
    fn displays_facade_reads_latest_host_snapshot_and_emits_layout_effect() {
        let mut runtime = JsxRuntime::new("", None).unwrap();
        assert!(
            runtime
                .eval_json::<bool>("nickel.displays.get() === undefined")
                .unwrap()
        );
        runtime
            .set_data(r#"{"displays":{"generation":1,"outputs":[{"name":"HDMI-A-1"}]}}"#)
            .unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
                .unwrap(),
            serde_json::json!({"generation": 1, "outputs": [{"name": "HDMI-A-1"}]})
        );
        runtime
            .eval("nickel.displays.get().outputs[0].name = 'mutated'")
            .unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
                .unwrap(),
            serde_json::json!({"generation": 1, "outputs": [{"name": "HDMI-A-1"}]})
        );
        runtime
            .set_data(r#"{"displays":{"generation":2,"available":true,"revision":"0123456789abcdef","outputs":[]}}"#)
            .unwrap();
        assert_eq!(
            runtime
                .eval_json::<Value>("JSON.stringify(nickel.displays.get())")
                .unwrap(),
            serde_json::json!({"generation": 2, "available": true, "revision": "0123456789abcdef", "outputs": []})
        );

        assert!(runtime.eval("nickel.displays.setLayout({primary:'HDMI-A-1',placements:[{name:'HDMI-A-1',x:0,y:0,enabled:true}]}, 'fedcba9876543210')").is_err());
        runtime.eval("let requestedLayout = {primary: 'HDMI-A-1', placements: [{name: 'HDMI-A-1', x: 0, y: 0, enabled: true, scale_120: 120, transform:'rotate90', mode: {width: 1920, height: 1080, refresh_millihz: 60000}}]}; nickel.displays.setLayout(requestedLayout); requestedLayout.placements[0].x = 100; nickel.displays.confirm(); nickel.displays.revert()")
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![
                serde_json::json!({
                    "type": "displays.setLayout",
                    "revision": "0123456789abcdef",
                    "layout": {
                        "primary": "HDMI-A-1",
                        "placements": [{
                            "name": "HDMI-A-1", "x": 0, "y": 0, "enabled": true,
                            "scale_120": 120,
                            "transform": "rotate90",
                            "mode": {"width": 1920, "height": 1080, "refresh_millihz": 60000}
                        }]
                    }
                }),
                serde_json::json!({"type": "displays.confirm"}),
                serde_json::json!({"type": "displays.revert"})
            ]
        );
    }

    #[test]
    fn displays_facade_rejects_invalid_layout_envelopes_without_effects() {
        let mut runtime = JsxRuntime::new("", None).unwrap();
        for expression in [
            "nickel.displays.setLayout(null)",
            "nickel.displays.setLayout([])",
            "nickel.displays.setLayout({primary: 1, placements: []})",
            "nickel.displays.setLayout({primary: 'A', placements: []})",
            "nickel.displays.setLayout({primary: 'A', placements: Array(33).fill({})})",
        ] {
            assert!(runtime.eval(expression).is_err(), "{expression}");
        }
        assert!(runtime.take_effects().unwrap().is_empty());
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
        let source = "function App() { const [count, setCount] = useState(0); return h(Window, {}, h(Button, {onClick: () => setCount(count + 1)}, `${nickel.data.surface.id}:${count}`)); }";
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

#[cfg(test)]
mod preferences_page_tests;

#[cfg(test)]
mod wallpaper_page_tests;
