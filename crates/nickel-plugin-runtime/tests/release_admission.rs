//! Ignored release-profile admission workloads for incremental JSX reconciliation.
//!
//! Run with:
//! `cargo test --release -p nickel-plugin-runtime --test release_admission -- --ignored --nocapture`

use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use nickel_plugin_runtime::{
    JsxRuntime, NativePatchCounters, NativePatchEnvelope, NativePatchOperation, ScheduledPatch,
};
use serde_json::{Value, json};

const ITEM_COUNT: usize = 200;
const WARMUP_ITERATIONS: usize = 5;
const MEASURED_ITERATIONS: usize = 50;
static RELEASE_ADMISSION: OnceLock<Mutex<()>> = OnceLock::new();

const SOURCE: &str = r#"
    function Item({item,initialState,initialReduced}) {
        const [state,setState]=useState(initialState);
        const [reduced,dispatch]=useReducer((value,next)=>next,initialReduced);
        return h(Button,{id:'item-'+item,key:'item-'+item,onClick:update=>{
            setState(update.state);
            dispatch(update.reduced);
        }},`${item}:${state}:${reduced}`);
    }
    function App() {
        const initial=nickel.data.admission;
        const [items,setItems]=useState(initial.items);
        return h(Window,{id:'admission'},
            h(Button,{id:'collection',key:'collection',onClick:update=>setItems(update.items)},'collection'),
            h(Column,{id:'items',key:'items'},...items.map(item=>h(Item,{key:'component-'+item,
                item,initialState:item===0?initial.leafState:0,
                initialReduced:item===0?initial.leafReduced:0}))));
    }
"#;

const LIFECYCLE_SOURCE_V1: &str = r#"
    function App() {
        const [state,setState]=useState(nickel.data.admission.state);
        const [reduced,dispatch]=useReducer((value,next)=>next,nickel.data.admission.reduced);
        const windows=useSyncExternalStore(NickelStores.windows.subscribe,NickelStores.windows.getSnapshot);
        const memoized=useMemo(()=>{lifecycle.memoRuns++;return state+reduced},[state,reduced]);
        const callback=useCallback(update=>{setState(update.state);dispatch(update.reduced)},[]);
        useEffect(()=>{lifecycle.effectSetups++;return()=>{lifecycle.effectCleanups++}},[state,reduced,windows.length]);
        return h(Window,{id:'lifecycle'},h(Button,{id:'lifecycle-leaf',key:'leaf',onClick:callback},
            'v1:'+state+':'+reduced+':'+memoized+':'+windows.length));
    }
"#;

const LIFECYCLE_SOURCE_V2: &str = r#"
    function App() {
        const [state,setState]=useState(nickel.data.admission.state);
        const [reduced,dispatch]=useReducer((value,next)=>next,nickel.data.admission.reduced);
        const windows=useSyncExternalStore(NickelStores.windows.subscribe,NickelStores.windows.getSnapshot);
        const memoized=useMemo(()=>{lifecycle.memoRuns++;return state+reduced},[state,reduced]);
        const callback=useCallback(update=>{setState(update.state);dispatch(update.reduced)},[]);
        useEffect(()=>{lifecycle.effectSetups++;return()=>{lifecycle.effectCleanups++}},[state,reduced,windows.length]);
        return h(Window,{id:'lifecycle'},h(Button,{id:'lifecycle-leaf',key:'leaf',onClick:callback},
            'v2:'+state+':'+reduced+':'+memoized+':'+windows.length));
    }
"#;

const LIFECYCLE_SOURCE_RESET: &str = r#"
    function App(){
        const [state]=useState(9);
        useEffect(()=>{lifecycle.effectSetups++;return()=>{lifecycle.effectCleanups++}},[]);
        return h(Window,{id:'lifecycle'},h(Text,{id:'lifecycle-reset'},'reset:'+state));
    }
"#;

const LIFECYCLE_CHURN: usize = 16;

#[derive(Clone, Copy, Debug)]
enum Workload {
    Insert,
    Remove,
    Reorder,
    LeafHookReducer,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Insert => "keyed_insert",
            Self::Remove => "keyed_remove",
            Self::Reorder => "keyed_reorder",
            Self::LeafHookReducer => "leaf_hook_reducer",
        }
    }
}

#[derive(Default)]
struct Samples {
    elapsed: Vec<Duration>,
    operations: u64,
    transport_bytes: u64,
    counters: NativePatchCounters,
    exact_sample: Option<(usize, usize, NativePatchCounters)>,
}

impl Samples {
    fn record(&mut self, elapsed: Duration, patch: &NativePatchEnvelope, transport_bytes: usize) {
        let exact_sample = (patch.operations.len(), transport_bytes, patch.counters);
        if let Some(expected) = self.exact_sample {
            assert_eq!(exact_sample, expected, "deterministic work count changed");
        } else {
            self.exact_sample = Some(exact_sample);
        }
        self.elapsed.push(elapsed);
        self.operations += patch.operations.len() as u64;
        self.transport_bytes += transport_bytes as u64;
        self.counters.nodes_visited += patch.counters.nodes_visited;
        self.counters.nodes_mutated += patch.counters.nodes_mutated;
        self.counters.local_materializations += patch.counters.local_materializations;
        self.counters.expansion_nodes += patch.counters.expansion_nodes;
        self.counters.tree_bytes += patch.counters.tree_bytes;
    }

    fn emit(&self, workload: Workload) {
        let distribution = Distribution::new(&self.elapsed);
        let report = json!({
            "schema": 1,
            "suite": "jsx_incremental",
            "workload": workload.name(),
            "metadata": {
                "items": ITEM_COUNT,
                "iterations": MEASURED_ITERATIONS,
                "warmupIterations": WARMUP_ITERATIONS,
            },
            "work": {
                "patchOperations": self.operations,
                "transportBytes": self.transport_bytes,
                "nodesVisited": self.counters.nodes_visited,
                "nodesMutated": self.counters.nodes_mutated,
                "localMaterializations": self.counters.local_materializations,
                "expansionNodes": self.counters.expansion_nodes,
                "completeTreeBytes": self.counters.tree_bytes,
            },
            "timings": {
                "dispatchAndReconcile": {
                    "p50_ns": distribution.p50.as_nanos() as u64,
                    "p95_ns": distribution.p95.as_nanos() as u64,
                    "p99_ns": distribution.p99.as_nanos() as u64,
                    "max_ns": distribution.max.as_nanos() as u64,
                }
            }
        });
        eprintln!("nickel_release_admission={report}");
    }
}

struct Distribution {
    p50: Duration,
    p95: Duration,
    p99: Duration,
    max: Duration,
}

impl Distribution {
    fn new(samples: &[Duration]) -> Self {
        assert!(!samples.is_empty());
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let at = |percentile: usize| {
            let rank = sorted.len().saturating_mul(percentile).div_ceil(100);
            sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
        };
        Self {
            p50: at(50),
            p95: at(95),
            p99: at(99),
            max: *sorted.last().unwrap(),
        }
    }
}

fn data(items: &[u64], leaf_state: u64, leaf_reduced: u64) -> Value {
    json!({"admission": {
        "items": items,
        "leafState": leaf_state,
        "leafReduced": leaf_reduced,
    }})
}

fn cold(items: &[u64], leaf_state: u64, leaf_reduced: u64) -> Value {
    let mut runtime = JsxRuntime::new(
        SOURCE,
        Some(&data(items, leaf_state, leaf_reduced).to_string()),
    )
    .unwrap();
    runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap()
}

fn find_by_id<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    if value.get("id").and_then(Value::as_str) == Some(id) {
        return Some(value);
    }
    match value {
        Value::Array(values) => values.iter().find_map(|value| find_by_id(value, id)),
        Value::Object(object) => object.values().find_map(|value| find_by_id(value, id)),
        _ => None,
    }
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

fn find_handler_mut<'a>(value: &'a mut Value, slot: &str) -> Option<(&'a mut Value, String)> {
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
            } => find_native_mut(tree, target).unwrap()[property] = value.clone(),
            NativePatchOperation::ReplaceHandlerSlot { slot, action } => {
                let (node, property) = find_handler_mut(tree, slot).unwrap();
                node[&property] = json!(action);
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

fn canonicalize_generation_local_actions(value: &mut Value) {
    match value {
        Value::Array(values) => values
            .iter_mut()
            .for_each(canonicalize_generation_local_actions),
        Value::Object(object) => {
            let slots = object
                .get("__handlerSlots")
                .and_then(Value::as_object)
                .map(|slots| {
                    slots
                        .iter()
                        .filter_map(|(property, slot)| {
                            slot.as_str()
                                .map(|slot| (property.clone(), Value::String(slot.to_owned())))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (property, slot) in slots {
                object.insert(property, slot);
            }
            object
                .values_mut()
                .for_each(canonicalize_generation_local_actions);
        }
        _ => {}
    }
}

fn lifecycle_data(state: u64, reduced: u64) -> String {
    json!({"admission":{"state":state,"reduced":reduced}}).to_string()
}

fn lifecycle_runtime() -> JsxRuntime {
    JsxRuntime::new(
        "globalThis.lifecycle={memoRuns:0,effectSetups:0,effectCleanups:0};function App(){return null}",
        Some(&lifecycle_data(0, 0)),
    )
    .unwrap()
}

fn cold_lifecycle(source: &str, state: u64, reduced: u64, windows: &Value) -> Value {
    let source =
        format!("globalThis.lifecycle={{memoRuns:0,effectSetups:0,effectCleanups:0}};{source}");
    let mut runtime = JsxRuntime::new(&source, Some(&lifecycle_data(state, reduced))).unwrap();
    runtime.set_windows_store(windows).unwrap();
    runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LifecycleWork {
    patch_operations: usize,
    patch_transport_bytes: usize,
    patch_nodes_visited: u64,
    patch_nodes_mutated: u64,
    patch_local_materializations: u64,
    patch_expansion_nodes: u64,
    patch_complete_tree_bytes: u64,
    cold_tree_transport_bytes: u64,
    component_executions: u64,
    lifecycle_commits: u64,
    effects_scheduled: u64,
    effects_run: u64,
    cleanups: u64,
    store_changes: u64,
    memo_factories: u64,
    retained_hooks_after_unmount: u64,
    retained_records_after_unmount: u64,
    retained_handlers_after_unmount: u64,
    retained_subscriptions_after_unmount: u64,
    retained_surface_states_after_unmount: u64,
    retained_surface_apps_after_unmount: u64,
}

struct LifecycleTimings {
    state_and_reducer: Duration,
    selector_subscription: Duration,
    compatible_hot_reload: Duration,
    incompatible_hot_reload: Duration,
    unmount_churn: Duration,
    total: Duration,
}

#[derive(Default)]
struct LifecycleSamples {
    state_and_reducer: Vec<Duration>,
    selector_subscription: Vec<Duration>,
    compatible_hot_reload: Vec<Duration>,
    incompatible_hot_reload: Vec<Duration>,
    unmount_churn: Vec<Duration>,
    total: Vec<Duration>,
    exact: Option<LifecycleWork>,
}

impl LifecycleSamples {
    fn record(&mut self, timings: LifecycleTimings, work: LifecycleWork) {
        if let Some(expected) = self.exact {
            assert_eq!(work, expected, "deterministic lifecycle work changed");
        } else {
            self.exact = Some(work);
        }
        self.state_and_reducer.push(timings.state_and_reducer);
        self.selector_subscription
            .push(timings.selector_subscription);
        self.compatible_hot_reload
            .push(timings.compatible_hot_reload);
        self.incompatible_hot_reload
            .push(timings.incompatible_hot_reload);
        self.unmount_churn.push(timings.unmount_churn);
        self.total.push(timings.total);
    }

    fn distribution(value: &[Duration]) -> Value {
        let distribution = Distribution::new(value);
        json!({
            "p50_ns": distribution.p50.as_nanos() as u64,
            "p95_ns": distribution.p95.as_nanos() as u64,
            "p99_ns": distribution.p99.as_nanos() as u64,
            "max_ns": distribution.max.as_nanos() as u64,
        })
    }

    fn emit(&self) {
        let work = self.exact.unwrap();
        let report = json!({
            "schema": 1,
            "suite": "jsx_incremental",
            "workload": "hook_lifecycle_churn",
            "metadata": {
                "iterations": MEASURED_ITERATIONS,
                "warmupIterations": WARMUP_ITERATIONS,
                "unmountCyclesPerIteration": LIFECYCLE_CHURN + 1,
            },
            "work": {
                "patchOperationsPerIteration": work.patch_operations,
                "patchTransportBytesPerIteration": work.patch_transport_bytes,
                "patchNodesVisitedPerIteration": work.patch_nodes_visited,
                "patchNodesMutatedPerIteration": work.patch_nodes_mutated,
                "patchLocalMaterializationsPerIteration": work.patch_local_materializations,
                "patchExpansionNodesPerIteration": work.patch_expansion_nodes,
                "patchCompleteTreeBytesPerIteration": work.patch_complete_tree_bytes,
                "coldTreeTransportBytesPerIteration": work.cold_tree_transport_bytes,
                "componentExecutionsPerIteration": work.component_executions,
                "lifecycleCommitsPerIteration": work.lifecycle_commits,
                "effectsScheduledPerIteration": work.effects_scheduled,
                "effectsRunPerIteration": work.effects_run,
                "cleanupsPerIteration": work.cleanups,
                "storeChangesPerIteration": work.store_changes,
                "memoFactoriesPerIteration": work.memo_factories,
                "retainedHooksAfterUnmount": work.retained_hooks_after_unmount,
                "retainedRecordsAfterUnmount": work.retained_records_after_unmount,
                "retainedHandlersAfterUnmount": work.retained_handlers_after_unmount,
                "retainedSubscriptionsAfterUnmount": work.retained_subscriptions_after_unmount,
                "retainedSurfaceStatesAfterUnmount": work.retained_surface_states_after_unmount,
                "retainedSurfaceAppsAfterUnmount": work.retained_surface_apps_after_unmount,
            },
            "timings": {
                "stateAndReducer": Self::distribution(&self.state_and_reducer),
                "selectorSubscription": Self::distribution(&self.selector_subscription),
                "compatibleHotReload": Self::distribution(&self.compatible_hot_reload),
                "incompatibleHotReload": Self::distribution(&self.incompatible_hot_reload),
                "unmountChurn": Self::distribution(&self.unmount_churn),
                "total": Self::distribution(&self.total),
            }
        });
        eprintln!("nickel_release_admission={report}");
    }
}

fn lifecycle_identity(
    signature: &'static str,
) -> nickel_plugin_runtime::HotReloadIdentity<'static> {
    nickel_plugin_runtime::HotReloadIdentity {
        owner: "admission.shell",
        module: "src/Lifecycle.jsx",
        export: "App",
        signature,
    }
}

fn lifecycle_patch(
    runtime: &mut JsxRuntime,
    accepted: &mut Value,
    expression: &str,
) -> (NativePatchEnvelope, usize) {
    let ScheduledPatch::Patched {
        patch,
        transport_bytes,
        ..
    } = runtime.dispatch_patched(expression).unwrap()
    else {
        panic!("lifecycle update unexpectedly produced no patch");
    };
    apply_patch(accepted, &patch);
    runtime.finish_patch_render(true).unwrap();
    runtime.finish_event(true).unwrap();
    (patch, transport_bytes)
}

fn exercise_lifecycle() -> (LifecycleTimings, LifecycleWork) {
    let mut runtime = lifecycle_runtime();
    let identity = lifecycle_identity("state,reducer,external-store,memo,callback,effect");
    runtime
        .register_surface_entry_with_identity("lifecycle", LIFECYCLE_SOURCE_V1, &identity)
        .unwrap();
    runtime.select_surface("lifecycle").unwrap();
    runtime.set_data(&lifecycle_data(0, 0)).unwrap();
    let mut accepted = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();
    let mut cold_tree_transport_bytes = serde_json::to_vec(&accepted).unwrap().len() as u64;
    let initial_leaf = find_by_id(&accepted, "lifecycle-leaf").unwrap();
    let native_identity = initial_leaf["__nativeId"].clone();
    let handler_identity = initial_leaf["__handlerSlots"]["action"].clone();
    let action = initial_leaf["action"].as_u64().unwrap();

    let phase = Instant::now();
    let (state_patch, state_transport) = lifecycle_patch(
        &mut runtime,
        &mut accepted,
        &format!("__nickelDispatchBatchPatched([[{action},{{\"state\":3,\"reduced\":4}}]])"),
    );
    let state_and_reducer = phase.elapsed();
    assert_eq!(state_patch.operations.len(), 1);
    assert_eq!(state_patch.counters.nodes_visited, 2);

    let windows = json!([{"id":"window-1","title":"One"}]);
    let phase = Instant::now();
    assert!(runtime.set_windows_store(&windows).unwrap());
    let (store_patch, store_transport) = lifecycle_patch(
        &mut runtime,
        &mut accepted,
        "__nickelDispatchBatchPatched([])",
    );
    let selector_subscription = phase.elapsed();
    assert_eq!(store_patch.operations.len(), 1);
    assert_eq!(store_patch.counters.nodes_visited, 2);

    let mut oracle = cold_lifecycle(LIFECYCLE_SOURCE_V1, 3, 4, &windows);
    let mut canonical_accepted = accepted.clone();
    canonicalize_generation_local_actions(&mut canonical_accepted);
    canonicalize_generation_local_actions(&mut oracle);
    assert_eq!(
        canonical_accepted, oracle,
        "hook/store updates diverged from cold oracle"
    );
    let surviving_leaf = find_by_id(&accepted, "lifecycle-leaf").unwrap();
    assert_eq!(surviving_leaf["__nativeId"], native_identity);
    assert_eq!(surviving_leaf["__handlerSlots"]["action"], handler_identity);

    let phase = Instant::now();
    assert!(
        runtime
            .hot_replace_surface_entry("lifecycle", LIFECYCLE_SOURCE_V2, &identity)
            .unwrap()
    );
    accepted = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();
    cold_tree_transport_bytes += serde_json::to_vec(&accepted).unwrap().len() as u64;
    let compatible_hot_reload = phase.elapsed();
    let surviving_leaf = find_by_id(&accepted, "lifecycle-leaf").unwrap();
    assert_eq!(surviving_leaf["children"][0], "v2:3:4:7:1");
    assert_eq!(surviving_leaf["__nativeId"], native_identity);
    assert_eq!(surviving_leaf["__handlerSlots"]["action"], handler_identity);
    let mut oracle = cold_lifecycle(LIFECYCLE_SOURCE_V2, 3, 4, &windows);
    let mut canonical_accepted = accepted.clone();
    canonicalize_generation_local_actions(&mut canonical_accepted);
    canonicalize_generation_local_actions(&mut oracle);
    assert_eq!(
        canonical_accepted, oracle,
        "compatible reload diverged from cold oracle"
    );

    let incompatible = lifecycle_identity("state,effect");
    let phase = Instant::now();
    assert!(
        !runtime
            .hot_replace_surface_entry("lifecycle", LIFECYCLE_SOURCE_RESET, &incompatible)
            .unwrap()
    );
    let reset = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();
    cold_tree_transport_bytes += serde_json::to_vec(&reset).unwrap().len() as u64;
    let incompatible_hot_reload = phase.elapsed();
    assert_eq!(
        find_by_id(&reset, "lifecycle-reset").unwrap()["children"][0],
        "reset:9"
    );
    assert!(find_by_id(&reset, "lifecycle-leaf").is_none());
    let mut reset_oracle = cold_lifecycle(LIFECYCLE_SOURCE_RESET, 0, 0, &windows);
    let mut canonical_reset = reset.clone();
    canonicalize_generation_local_actions(&mut canonical_reset);
    canonicalize_generation_local_actions(&mut reset_oracle);
    assert_eq!(
        canonical_reset, reset_oracle,
        "incompatible reload diverged from cold oracle"
    );

    runtime.drop_surface("lifecycle").unwrap();
    let stale = runtime
        .dispatch_patched(&format!(
            "__nickelDispatchBatchPatched([[{action},{{\"state\":99,\"reduced\":99}}]])"
        ))
        .unwrap();
    assert!(matches!(stale, ScheduledPatch::Unchanged));
    runtime.finish_event(true).unwrap();

    let churn_started = Instant::now();
    for cycle in 0..LIFECYCLE_CHURN {
        let id = format!("churn-{cycle}");
        runtime
            .register_surface_entry_with_identity(&id, LIFECYCLE_SOURCE_RESET, &incompatible)
            .unwrap();
        runtime.select_surface(&id).unwrap();
        let rendered = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        cold_tree_transport_bytes += serde_json::to_vec(&rendered).unwrap().len() as u64;
        runtime.drop_surface(&id).unwrap();
    }
    let unmount_churn = churn_started.elapsed();

    let lifecycle: Value = runtime.eval_json("JSON.stringify(lifecycle)").unwrap();
    assert_eq!(lifecycle["memoRuns"], 2);
    assert_eq!(lifecycle["effectSetups"], (LIFECYCLE_CHURN + 4) as u64);
    assert_eq!(lifecycle["effectCleanups"], (LIFECYCLE_CHURN + 4) as u64);
    let diagnostics = runtime.runtime_diagnostics().unwrap();
    let retained: Value = runtime
        .eval_json(
            "JSON.stringify((()=>{let subscriptions=0;for(const hooks of [__componentHooks,...Array.from(__surfaceStates.values(),state=>state.hooks)])for(const slots of hooks.values())for(const entry of slots)if(entry?.kind==='sync-external-store')subscriptions++;return {hooks:__componentHooks.size,records:__componentRecords.size,handlers:__handlers.length,subscriptions,surfaces:__surfaceStates.size,apps:__surfaceApps.size}})())",
        )
        .unwrap();
    let work = LifecycleWork {
        patch_operations: state_patch.operations.len() + store_patch.operations.len(),
        patch_transport_bytes: state_transport + store_transport,
        patch_nodes_visited: state_patch.counters.nodes_visited
            + store_patch.counters.nodes_visited,
        patch_nodes_mutated: state_patch.counters.nodes_mutated
            + store_patch.counters.nodes_mutated,
        patch_local_materializations: state_patch.counters.local_materializations
            + store_patch.counters.local_materializations,
        patch_expansion_nodes: state_patch.counters.expansion_nodes
            + store_patch.counters.expansion_nodes,
        patch_complete_tree_bytes: state_patch.counters.tree_bytes
            + store_patch.counters.tree_bytes,
        cold_tree_transport_bytes,
        component_executions: diagnostics["counters"]["executed"].as_u64().unwrap(),
        lifecycle_commits: (LIFECYCLE_CHURN + 5) as u64,
        effects_scheduled: diagnostics["counters"]["effectsScheduled"]
            .as_u64()
            .unwrap(),
        effects_run: diagnostics["counters"]["effectsRun"].as_u64().unwrap(),
        cleanups: diagnostics["counters"]["cleanups"].as_u64().unwrap(),
        store_changes: diagnostics["counters"]["storeChanges"].as_u64().unwrap(),
        memo_factories: lifecycle["memoRuns"].as_u64().unwrap(),
        retained_hooks_after_unmount: retained["hooks"].as_u64().unwrap(),
        retained_records_after_unmount: retained["records"].as_u64().unwrap(),
        retained_handlers_after_unmount: retained["handlers"].as_u64().unwrap(),
        retained_subscriptions_after_unmount: retained["subscriptions"].as_u64().unwrap(),
        retained_surface_states_after_unmount: retained["surfaces"].as_u64().unwrap(),
        retained_surface_apps_after_unmount: retained["apps"].as_u64().unwrap(),
    };
    assert_eq!(work.patch_operations, 2);
    assert_eq!(work.patch_nodes_visited, 4);
    assert_eq!(work.patch_nodes_mutated, 2);
    assert_eq!(work.patch_local_materializations, 0);
    assert_eq!(work.patch_expansion_nodes, 0);
    assert_eq!(work.patch_complete_tree_bytes, 0);
    assert_eq!(work.lifecycle_commits, work.component_executions);
    assert_eq!(work.effects_scheduled, (LIFECYCLE_CHURN + 4) as u64);
    assert_eq!(work.effects_run, work.effects_scheduled);
    assert_eq!(work.cleanups, work.effects_run);
    // Selecting the mount publishes its surface authority once; the windows
    // publication is the second and only selector-bearing store change.
    assert_eq!(work.store_changes, 2);
    assert_eq!(work.retained_hooks_after_unmount, 0);
    assert_eq!(work.retained_records_after_unmount, 0);
    assert_eq!(work.retained_handlers_after_unmount, 0);
    assert_eq!(work.retained_subscriptions_after_unmount, 0);
    assert!(work.retained_surface_states_after_unmount <= 1);
    assert_eq!(work.retained_surface_apps_after_unmount, 0);

    (
        LifecycleTimings {
            state_and_reducer,
            selector_subscription,
            compatible_hot_reload,
            incompatible_hot_reload,
            unmount_churn,
            total: state_and_reducer
                + selector_subscription
                + compatible_hot_reload
                + incompatible_hot_reload
                + unmount_churn,
        },
        work,
    )
}

fn exercise(workload: Workload) -> (Duration, NativePatchEnvelope, usize) {
    let initial_items = (0..ITEM_COUNT as u64).collect::<Vec<_>>();
    let mut runtime =
        JsxRuntime::new(SOURCE, Some(&data(&initial_items, 0, 0).to_string())).unwrap();
    let mut accepted = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();

    let survivor = find_by_id(&accepted, "item-1").unwrap();
    let survivor_native_id = survivor["__nativeId"].clone();
    let survivor_handler_slot = survivor["__handlerSlots"]["action"].clone();

    let (action, payload, expected_items, leaf_state, leaf_reduced) = match workload {
        Workload::Insert => {
            let mut items = initial_items.clone();
            items.insert(ITEM_COUNT / 2, ITEM_COUNT as u64);
            let action = find_by_id(&accepted, "collection").unwrap()["action"]
                .as_u64()
                .unwrap();
            (action, json!({"items":items}), items, 0, 0)
        }
        Workload::Remove => {
            let mut items = initial_items.clone();
            items.remove(ITEM_COUNT / 2);
            let action = find_by_id(&accepted, "collection").unwrap()["action"]
                .as_u64()
                .unwrap();
            (action, json!({"items":items}), items, 0, 0)
        }
        Workload::Reorder => {
            let mut items = initial_items.clone();
            let last = items.pop().unwrap();
            items.insert(0, last);
            let action = find_by_id(&accepted, "collection").unwrap()["action"]
                .as_u64()
                .unwrap();
            (action, json!({"items":items}), items, 0, 0)
        }
        Workload::LeafHookReducer => {
            let action = find_by_id(&accepted, "item-0").unwrap()["action"]
                .as_u64()
                .unwrap();
            (action, json!({"state":1,"reduced":2}), initial_items, 1, 2)
        }
    };

    let expression = format!("__nickelDispatchBatchPatched([[{action},{payload}]])");
    let start = Instant::now();
    let outcome = runtime.dispatch_patched(&expression).unwrap();
    let elapsed = start.elapsed();
    let ScheduledPatch::Patched {
        patch,
        transport_bytes,
        ..
    } = outcome
    else {
        panic!("{} unexpectedly produced no patch", workload.name());
    };

    assert_eq!(patch.counters.local_materializations, 0);
    assert_eq!(patch.counters.expansion_nodes, 0);
    assert_eq!(patch.counters.tree_bytes, 0);
    assert_eq!(patch.operations.len(), 1, "one update needs one mutation");
    assert_eq!(
        patch.counters.nodes_visited,
        if matches!(workload, Workload::LeafHookReducer) {
            1
        } else {
            2
        },
        "unchanged keyed siblings must stay outside the native diff walk"
    );
    match workload {
        Workload::Insert => assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::InsertChild { .. }))
        ),
        Workload::Remove => assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::RemoveChild { .. }))
        ),
        Workload::Reorder => assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::MoveChild { .. }))
        ),
        Workload::LeafHookReducer => assert!(
            patch
                .operations
                .iter()
                .any(|operation| matches!(operation, NativePatchOperation::SetPrimitive { .. }))
        ),
    }

    apply_patch(&mut accepted, &patch);
    runtime.finish_patch_render(true).unwrap();
    runtime.finish_event(true).unwrap();
    let mut oracle = cold(&expected_items, leaf_state, leaf_reduced);
    canonicalize_generation_local_actions(&mut accepted);
    canonicalize_generation_local_actions(&mut oracle);
    assert_eq!(
        accepted,
        oracle,
        "{} diverged from cold oracle",
        workload.name()
    );

    let survivor = find_by_id(&accepted, "item-1").unwrap();
    assert_eq!(survivor["__nativeId"], survivor_native_id);
    assert_eq!(survivor["__handlerSlots"]["action"], survivor_handler_slot);

    (elapsed, patch, transport_bytes)
}

fn run_release_workload(workload: Workload) {
    let _serial = RELEASE_ADMISSION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    for _ in 0..WARMUP_ITERATIONS {
        exercise(workload);
    }
    let mut samples = Samples::default();
    for _ in 0..MEASURED_ITERATIONS {
        let (elapsed, patch, transport_bytes) = exercise(workload);
        samples.record(elapsed, &patch, transport_bytes);
    }
    samples.emit(workload);
}

#[test]
#[ignore = "release-profile admission workload"]
fn keyed_insert_emits_release_distribution() {
    run_release_workload(Workload::Insert);
}

#[test]
#[ignore = "release-profile admission workload"]
fn keyed_remove_emits_release_distribution() {
    run_release_workload(Workload::Remove);
}

#[test]
#[ignore = "release-profile admission workload"]
fn keyed_reorder_emits_release_distribution() {
    run_release_workload(Workload::Reorder);
}

#[test]
#[ignore = "release-profile admission workload"]
fn leaf_hook_reducer_emits_release_distribution() {
    run_release_workload(Workload::LeafHookReducer);
}

#[test]
#[ignore = "release-profile admission workload"]
fn hook_lifecycle_churn_emits_release_distribution() {
    let _serial = RELEASE_ADMISSION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    for _ in 0..WARMUP_ITERATIONS {
        exercise_lifecycle();
    }
    let mut samples = LifecycleSamples::default();
    for _ in 0..MEASURED_ITERATIONS {
        let (timings, work) = exercise_lifecycle();
        samples.record(timings, work);
    }
    samples.emit();
}

#[test]
fn percentile_selection_uses_nearest_rank_members() {
    let samples = (1..=100).map(Duration::from_nanos).collect::<Vec<_>>();
    let distribution = Distribution::new(&samples);
    assert_eq!(distribution.p50, Duration::from_nanos(50));
    assert_eq!(distribution.p95, Duration::from_nanos(95));
    assert_eq!(distribution.p99, Duration::from_nanos(99));
    assert_eq!(distribution.max, Duration::from_nanos(100));
}
