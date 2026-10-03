//! Shared JavaScript evaluator for Nickel's native JSX hosts.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the Boa context and the bootstrap's render and event transactions.

use boa_engine::{Context, JsValue, Source, js_string};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

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

pub struct JsxRuntime {
    context: Context,
    settings_provider: Option<String>,
    settings_pages: std::collections::BTreeSet<String>,
    settings_revision: u64,
    settings_data: Option<std::rc::Rc<Value>>,
    checkpoint: Option<(u64, Option<std::rc::Rc<Value>>)>,
    invalidated: bool,
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

#[derive(Deserialize)]
struct ScheduledRenderWire {
    rendered: bool,
    #[serde(default)]
    dirty: Vec<String>,
    node: Option<Value>,
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

    pub fn eval(&mut self, source: &str) -> Result<(), String> {
        if self.invalidated {
            return Err("runtime checkpoint was invalidated".into());
        }
        self.context
            .eval(Source::from_bytes(source))
            .map_err(|error| error.to_string())?;
        Ok(())
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

    pub fn drop_surface(&mut self, id: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelDropSurface({id})"))
    }

    pub fn render<T>(
        &mut self,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let parsed = self
            .eval_json::<Value>(expression)
            .and_then(|value| parse(&value));
        let finalizer = if parsed.is_ok() {
            "__nickelCommitRender()"
        } else {
            "__nickelRollbackRender()"
        };
        self.eval(finalizer)
            .map_err(|error| format!("could not finalize plugin render: {error}"))?;
        parsed
    }

    /// Dispatch an event batch through the retained hook scheduler. If no
    /// state or reducer value changed, no component is executed and no native
    /// tree needs validation. A rendered result is still a complete tree until
    /// the typed subtree mutation protocol is implemented.
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
        let parsed = parse(&node);
        self.eval(if parsed.is_ok() {
            "__nickelCommitRender()"
        } else {
            "__nickelRollbackRender()"
        })
        .map_err(|error| format!("could not finalize scheduled plugin render: {error}"))?;
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
        assert_eq!(
            runtime
                .eval_json::<bool>("JSON.stringify(nickel.plugins.list()[0].enabled)")
                .unwrap(),
            true
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
        assert_eq!(
            denied
                .eval_json::<bool>("JSON.stringify(nickel.plugins.get().available)")
                .unwrap(),
            false
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
        assert_eq!(
            denied
                .eval_json::<bool>("nickel.bluetooth.get().available")
                .unwrap(),
            false
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
        assert_eq!(
            runtime
                .eval_json::<bool>("nickel.displays.get() === undefined")
                .unwrap(),
            true
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
