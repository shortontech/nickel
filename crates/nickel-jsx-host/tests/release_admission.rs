//! Ignored release-profile admission workloads for Nickel desktop stores and lifecycle.
//! Generic keyed reconciliation workloads live upstream in Twinkle.
//!
//! Run with:
//! `cargo test --release -p nickel-jsx-host --test release_admission -- --ignored --nocapture`

use nickel_jsx_host::DomainStoreRuntimeExt;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use nickel_jsx_host::{
    JsxRuntime, NativePatchCounters, NativePatchEnvelope, NativePatchOperation, ScheduledPatch,
};
use serde_json::{Value, json};

const WARMUP_ITERATIONS: usize = 5;
const MEASURED_ITERATIONS: usize = 50;
static RELEASE_ADMISSION: OnceLock<Mutex<()>> = OnceLock::new();

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

const INDEPENDENT_BASE_SOURCE: &str = r#"
    globalThis.mountRuns={taskbar:0,launcher:0,settings:0};
    globalThis.storeSelections={taskbar:0,launcher:0,settings:0};
    function App(){return null}
"#;

const TASKBAR_SOURCE: &str = r#"
    const selectPinned=items=>{storeSelections.taskbar++;return items.filter(item=>item.pinned).length};
    function App(){
        mountRuns.taskbar++;
        const pinned=useApplications(selectPinned);
        const [local,setLocal]=useState(11);
        return h(Window,{id:'taskbar'},h(Button,{id:'taskbar-control',key:'control',onClick:()=>setLocal(value=>value+1)},
            `taskbar:${local}:${pinned}`));
    }
"#;

const LAUNCHER_SOURCE: &str = r#"
    const selectWindowCount=items=>{storeSelections.launcher++;return items.length};
    function App(){
        mountRuns.launcher++;
        const windows=useWindows(selectWindowCount);
        const [local,setLocal]=useState(22);
        return h(Window,{id:'launcher'},h(Button,{id:'launcher-control',key:'control',onClick:()=>setLocal(value=>value+1)},
            `launcher:${local}:${windows}`));
    }
"#;

const SETTINGS_SOURCE: &str = r#"
    const selectTheme=value=>{storeSelections.settings++;return value.mode};
    function App(){
        mountRuns.settings++;
        const mode=useTheme(selectTheme);
        const [local,setLocal]=useState(33);
        return h(Window,{id:'settings'},h(Button,{id:'settings-control',key:'control',onClick:()=>setLocal(value=>value+1)},
            `settings:${local}:${mode}`));
    }
"#;

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
    nickel_jsx_host::create_runtime(
        "globalThis.lifecycle={memoRuns:0,effectSetups:0,effectCleanups:0};function App(){return null}",
        Some(&lifecycle_data(0, 0)),
    )
    .unwrap()
}

fn cold_lifecycle(source: &str, state: u64, reduced: u64, windows: &Value) -> Value {
    let source =
        format!("globalThis.lifecycle={{memoRuns:0,effectSetups:0,effectCleanups:0}};{source}");
    let mut runtime =
        nickel_jsx_host::create_runtime(&source, Some(&lifecycle_data(state, reduced))).unwrap();
    runtime.set_windows_store(windows).unwrap();
    runtime
        .render("__twinkleRender()", |node| Ok(node.clone()))
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MountProfileWork {
    component_executions: u64,
    patches: u64,
    patch_operations: u64,
    patch_nodes_visited: u64,
    patch_transport_bytes: u64,
    typed_apply_attempts: u64,
    typed_apply_rejections: u64,
    store_selections: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndependentMountWork {
    mounts: BTreeMap<&'static str, MountProfileWork>,
    store_changes: u64,
    store_newly_dirty: BTreeMap<&'static str, u64>,
    local_materializations: u64,
    expansion_nodes: u64,
    complete_tree_bytes: u64,
}

struct IndependentMountTimings {
    applications: Duration,
    windows: Duration,
    theme_rejected: Duration,
    theme_retry: Duration,
    total: Duration,
}

#[derive(Default)]
struct IndependentMountSamples {
    applications: Vec<Duration>,
    windows: Vec<Duration>,
    theme_rejected: Vec<Duration>,
    theme_retry: Vec<Duration>,
    total: Vec<Duration>,
    exact: Option<IndependentMountWork>,
}

impl IndependentMountSamples {
    fn record(&mut self, timings: IndependentMountTimings, work: IndependentMountWork) {
        if let Some(expected) = &self.exact {
            assert_eq!(
                &work, expected,
                "deterministic independent-mount work changed"
            );
        } else {
            self.exact = Some(work);
        }
        self.applications.push(timings.applications);
        self.windows.push(timings.windows);
        self.theme_rejected.push(timings.theme_rejected);
        self.theme_retry.push(timings.theme_retry);
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
        let work = self.exact.as_ref().unwrap();
        let mounts = work
            .mounts
            .iter()
            .map(|(mount, work)| {
                (
                    (*mount).to_owned(),
                    json!({
                        "componentExecutionsPerIteration": work.component_executions,
                        "patchesPerIteration": work.patches,
                        "patchOperationsPerIteration": work.patch_operations,
                        "patchNodesVisitedPerIteration": work.patch_nodes_visited,
                        "patchEnvelopeTransportBytesPerIteration": work.patch_transport_bytes,
                        "typedPatchApplyAttemptsPerIteration": work.typed_apply_attempts,
                        "typedPatchApplyRejectionsPerIteration": work.typed_apply_rejections,
                        "storeSelectionsPerIteration": work.store_selections,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let report = json!({
            "schema": 1,
            "suite": "jsx_incremental",
            "workload": "independent_mount_store_revisions",
            "metadata": {
                "iterations": MEASURED_ITERATIONS,
                "warmupIterations": WARMUP_ITERATIONS,
                "mounts": ["taskbar", "launcher", "settings"],
            },
            "work": {
                "mounts": mounts,
                "storeChangesPerIteration": work.store_changes,
                "storeNewlyDirtyPerIteration": work.store_newly_dirty,
                "patchLocalMaterializationsPerIteration": work.local_materializations,
                "patchExpansionNodesPerIteration": work.expansion_nodes,
                "patchCompleteTreeBytesPerIteration": work.complete_tree_bytes,
            },
            "timings": {
                "applicationsRevision": Self::distribution(&self.applications),
                "windowsRevision": Self::distribution(&self.windows),
                "themeRejectedCommit": Self::distribution(&self.theme_rejected),
                "themeAcceptedRetry": Self::distribution(&self.theme_retry),
                "total": Self::distribution(&self.total),
            }
        });
        eprintln!("nickel_release_admission={report}");
    }
}

fn initial_applications() -> Value {
    json!([{"id":"editor","name":"Editor","icon":"application:1","pinned":true,
        "pinOrder":0,"recentOrder":0,"kind":"application","launchClass":"graphical"}])
}

fn changed_applications() -> Value {
    json!([
        {"id":"editor","name":"Editor","icon":"application:1","pinned":true,
            "pinOrder":0,"recentOrder":0,"kind":"application","launchClass":"graphical"},
        {"id":"terminal","name":"Terminal","icon":"application:2","pinned":true,
            "pinOrder":1,"recentOrder":1,"kind":"application","launchClass":"terminal"}
    ])
}

fn initial_windows() -> Value {
    json!([{"id":"window-1","title":"Editor","active":true,"canActivate":true}])
}

fn changed_windows() -> Value {
    json!([
        {"id":"window-1","title":"Editor","active":true,"canActivate":true},
        {"id":"window-2","title":"Terminal","active":false,"canActivate":true}
    ])
}

fn theme(mode: &str) -> Value {
    json!({"mode":mode,"reducedMotion":false,"reducedTransparency":false})
}

fn independent_runtime(applications: &Value, windows: &Value, theme_value: &Value) -> JsxRuntime {
    let mut runtime = nickel_jsx_host::create_runtime(INDEPENDENT_BASE_SOURCE, None).unwrap();
    runtime
        .register_surface_entry("taskbar", TASKBAR_SOURCE)
        .unwrap();
    runtime
        .register_surface_entry("launcher", LAUNCHER_SOURCE)
        .unwrap();
    runtime
        .register_surface_entry("settings", SETTINGS_SOURCE)
        .unwrap();
    runtime.set_applications_store(applications).unwrap();
    runtime.set_windows_store(windows).unwrap();
    runtime.set_theme_store(theme_value).unwrap();
    runtime
}

fn render_mount(runtime: &mut JsxRuntime, mount: &str) -> Value {
    runtime.select_surface(mount).unwrap();
    runtime
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap()
}

fn cold_independent_mount(mount: &str) -> Value {
    let mut runtime =
        independent_runtime(&changed_applications(), &changed_windows(), &theme("dark"));
    render_mount(&mut runtime, mount)
}

fn profile_map(diagnostics: &Value) -> BTreeMap<String, Value> {
    diagnostics["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|profile| {
            (
                profile["surface"].as_str().unwrap().to_owned(),
                profile.clone(),
            )
        })
        .collect()
}

fn profile_delta(before: &Value, after: &Value, store_selections: u64) -> MountProfileWork {
    let delta =
        |field: &str| after[field].as_u64().unwrap_or(0) - before[field].as_u64().unwrap_or(0);
    MountProfileWork {
        component_executions: delta("componentExecutions"),
        patches: delta("patches"),
        patch_operations: delta("patchOperations"),
        patch_nodes_visited: delta("patchNodesVisited"),
        patch_transport_bytes: delta("patchEnvelopeTransportBytes"),
        typed_apply_attempts: delta("typedPatchApplyAttempts"),
        typed_apply_rejections: delta("typedPatchApplyRejections"),
        store_selections,
    }
}

fn reconcile_store_patch(
    runtime: &mut JsxRuntime,
    mount: &str,
    accepted: &mut Value,
    accept: bool,
) -> (Duration, NativePatchEnvelope) {
    runtime.select_surface(mount).unwrap();
    let started = Instant::now();
    let ScheduledPatch::Patched { patch, .. } = runtime
        .dispatch_patched("__twinkleDispatchBatchPatched([])")
        .unwrap()
    else {
        panic!("{mount} store revision unexpectedly produced no patch");
    };
    let elapsed = started.elapsed();
    if accept {
        apply_patch(accepted, &patch);
    }
    runtime.finish_patch_render(accept).unwrap();
    runtime.report_typed_patch_apply(0, accept).unwrap();
    runtime.finish_event(accept).unwrap();
    (elapsed, patch)
}

fn exercise_independent_mounts() -> (IndependentMountTimings, IndependentMountWork) {
    let mut runtime =
        independent_runtime(&initial_applications(), &initial_windows(), &theme("light"));
    let mut accepted = BTreeMap::new();
    for mount in ["taskbar", "launcher", "settings"] {
        accepted.insert(mount, render_mount(&mut runtime, mount));
    }
    let before_diagnostics = runtime.runtime_diagnostics().unwrap();
    let before_profiles = profile_map(&before_diagnostics);
    let before_runs: Value = runtime.eval_json("JSON.stringify(mountRuns)").unwrap();
    let before_selections: Value = runtime
        .eval_json("JSON.stringify(storeSelections)")
        .unwrap();
    let identities = accepted
        .iter()
        .map(|(mount, tree)| {
            let control = find_by_id(tree, &format!("{mount}-control")).unwrap();
            (
                *mount,
                (
                    control["__nativeId"].clone(),
                    control["__handlerSlots"]["action"].clone(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();

    assert!(
        runtime
            .set_applications_store(&changed_applications())
            .unwrap()
    );
    let (applications, taskbar_patch) = reconcile_store_patch(
        &mut runtime,
        "taskbar",
        accepted.get_mut("taskbar").unwrap(),
        true,
    );
    assert!(runtime.set_windows_store(&changed_windows()).unwrap());
    let (windows, launcher_patch) = reconcile_store_patch(
        &mut runtime,
        "launcher",
        accepted.get_mut("launcher").unwrap(),
        true,
    );
    assert!(runtime.set_theme_store(&theme("dark")).unwrap());
    let (theme_rejected, rejected_patch) = reconcile_store_patch(
        &mut runtime,
        "settings",
        accepted.get_mut("settings").unwrap(),
        false,
    );
    let (theme_retry, settings_patch) = reconcile_store_patch(
        &mut runtime,
        "settings",
        accepted.get_mut("settings").unwrap(),
        true,
    );
    assert_eq!(
        rejected_patch, settings_patch,
        "rollback changed the retry patch"
    );

    let patches = [
        &taskbar_patch,
        &launcher_patch,
        &rejected_patch,
        &settings_patch,
    ];
    for patch in patches {
        assert_eq!(patch.operations.len(), 1);
        assert_eq!(patch.counters.nodes_visited, 2);
        assert_eq!(patch.counters.nodes_mutated, 1);
        assert_eq!(patch.counters.local_materializations, 0);
        assert_eq!(patch.counters.expansion_nodes, 0);
        assert_eq!(patch.counters.tree_bytes, 0);
        assert!(matches!(
            patch.operations[0],
            NativePatchOperation::SetPrimitive { .. }
        ));
    }

    for mount in ["taskbar", "launcher", "settings"] {
        let tree = accepted.get(mount).unwrap();
        let control = find_by_id(tree, &format!("{mount}-control")).unwrap();
        assert_eq!(control["__nativeId"], identities[mount].0);
        assert_eq!(control["__handlerSlots"]["action"], identities[mount].1);
        let expected_local = match mount {
            "taskbar" => 11,
            "launcher" => 22,
            "settings" => 33,
            _ => unreachable!(),
        };
        assert!(
            control["children"][0]
                .as_str()
                .unwrap()
                .contains(&format!(":{expected_local}:"))
        );
        let mut retained = tree.clone();
        let mut oracle = cold_independent_mount(mount);
        canonicalize_generation_local_actions(&mut retained);
        canonicalize_generation_local_actions(&mut oracle);
        assert_eq!(retained, oracle, "{mount} diverged from its cold oracle");
    }

    let diagnostics = runtime.runtime_diagnostics().unwrap();
    let profiles = profile_map(&diagnostics);
    let runs: Value = runtime.eval_json("JSON.stringify(mountRuns)").unwrap();
    let selections: Value = runtime
        .eval_json("JSON.stringify(storeSelections)")
        .unwrap();
    let mut mounts = BTreeMap::new();
    for mount in ["taskbar", "launcher", "settings"] {
        let execution_delta = runs[mount].as_u64().unwrap() - before_runs[mount].as_u64().unwrap();
        let selection_delta =
            selections[mount].as_u64().unwrap() - before_selections[mount].as_u64().unwrap();
        let work = profile_delta(&before_profiles[mount], &profiles[mount], selection_delta);
        assert_eq!(work.component_executions, execution_delta);
        mounts.insert(mount, work);
    }
    assert_eq!(mounts["taskbar"].component_executions, 1);
    assert_eq!(mounts["launcher"].component_executions, 1);
    assert_eq!(mounts["settings"].component_executions, 2);
    assert_eq!(mounts["taskbar"].patches, 1);
    assert_eq!(mounts["launcher"].patches, 1);
    assert_eq!(mounts["settings"].patches, 2);
    assert_eq!(mounts["taskbar"].store_selections, 2);
    assert_eq!(mounts["launcher"].store_selections, 2);
    assert_eq!(mounts["settings"].store_selections, 3);
    assert_eq!(mounts["taskbar"].typed_apply_attempts, 1);
    assert_eq!(mounts["launcher"].typed_apply_attempts, 1);
    assert_eq!(mounts["settings"].typed_apply_attempts, 2);
    assert_eq!(mounts["settings"].typed_apply_rejections, 1);
    assert_eq!(mounts["taskbar"].typed_apply_rejections, 0);
    assert_eq!(mounts["launcher"].typed_apply_rejections, 0);

    let store_changes = diagnostics["storeChanges"].as_array().unwrap();
    let measured_changes = &store_changes[store_changes.len() - 3..];
    let store_newly_dirty = measured_changes
        .iter()
        .map(|entry| {
            let store = match entry["store"].as_str().unwrap() {
                "applications" => "applications",
                "windows" => "windows",
                "theme" => "theme",
                other => panic!("unexpected measured store {other}"),
            };
            (store, entry["newlyDirty"].as_u64().unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(store_newly_dirty["applications"], 1);
    assert_eq!(store_newly_dirty["windows"], 1);
    assert_eq!(store_newly_dirty["theme"], 1);
    let local_materializations = patches
        .iter()
        .map(|patch| patch.counters.local_materializations)
        .sum();
    let expansion_nodes = patches
        .iter()
        .map(|patch| patch.counters.expansion_nodes)
        .sum();
    let complete_tree_bytes = patches.iter().map(|patch| patch.counters.tree_bytes).sum();
    let total = applications + windows + theme_rejected + theme_retry;
    (
        IndependentMountTimings {
            applications,
            windows,
            theme_rejected,
            theme_retry,
            total,
        },
        IndependentMountWork {
            mounts,
            store_changes: 3,
            store_newly_dirty,
            local_materializations,
            expansion_nodes,
            complete_tree_bytes,
        },
    )
}

fn lifecycle_identity(signature: &'static str) -> nickel_jsx_host::HotReloadIdentity<'static> {
    nickel_jsx_host::HotReloadIdentity {
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
        .render("__twinkleRender()", |node| Ok(node.clone()))
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
        &format!("__twinkleDispatchBatchPatched([[{action},{{\"state\":3,\"reduced\":4}}]])"),
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
        "__twinkleDispatchBatchPatched([])",
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
        .render("__twinkleRender()", |node| Ok(node.clone()))
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
        .render("__twinkleRender()", |node| Ok(node.clone()))
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
            "__twinkleDispatchBatchPatched([[{action},{{\"state\":99,\"reduced\":99}}]])"
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
            .render("__twinkleRender()", |node| Ok(node.clone()))
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
#[ignore = "release-profile admission workload"]
fn independent_mount_store_revisions_emit_release_distribution() {
    let _serial = RELEASE_ADMISSION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    for _ in 0..WARMUP_ITERATIONS {
        exercise_independent_mounts();
    }
    let mut samples = IndependentMountSamples::default();
    for _ in 0..MEASURED_ITERATIONS {
        let (timings, work) = exercise_independent_mounts();
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
