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
    static RELEASE_ADMISSION: OnceLock<Mutex<()>> = OnceLock::new();
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
fn percentile_selection_uses_nearest_rank_members() {
    let samples = (1..=100).map(Duration::from_nanos).collect::<Vec<_>>();
    let distribution = Distribution::new(&samples);
    assert_eq!(distribution.p50, Duration::from_nanos(50));
    assert_eq!(distribution.p95, Duration::from_nanos(95));
    assert_eq!(distribution.p99, Duration::from_nanos(99));
    assert_eq!(distribution.max, Duration::from_nanos(100));
}
