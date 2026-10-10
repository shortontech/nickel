use crate::*;
use serde_json::Value;
#[test]
fn boundary_contains_selector_and_cleanup_failures() {
    let mut selector = crate::create_runtime(
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
    let mut cleanup = crate::create_runtime(source, None).unwrap();
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
fn developer_diagnostics_cover_identity_selectors_props_depth_and_loops() {
    fn kinds(runtime: &mut super::JsxRuntime) -> Vec<String> {
        runtime.runtime_diagnostics().unwrap()["developerDiagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    let mut identity = crate::create_runtime(
        "function App(){return h(Column,null,[h(Text,null,'unkeyed')])}",
        None,
    )
    .unwrap();
    identity.render("__nickelRender()", |_| Ok(())).unwrap();
    assert!(kinds(&mut identity).contains(&"positional-identity-churn".into()));

    let mut selector = crate::create_runtime(
            "function App(){const value=useWindows(items=>({count:items.length}));return h(Text,null,String(value.count))}",
            None,
        )
        .unwrap();
    selector.render("__nickelRender()", |_| Ok(())).unwrap();
    selector.render("__nickelRender()", |_| Ok(())).unwrap();
    assert!(kinds(&mut selector).contains(&"unstable-selector".into()));

    let mut props = crate::create_runtime(
            "function Bad(){return h(Slider,{min:1,max:0,value:0,onChange:()=>{}})} function App(){return h(ErrorBoundary,{fallback:h(Text,null,'fallback')},h(Bad))}",
            None,
        )
        .unwrap();
    props.render("__nickelRender()", |_| Ok(())).unwrap();
    assert!(kinds(&mut props).contains(&"invalid-props".into()));

    let mut depth = crate::create_runtime(
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

    let mut effect_loop = crate::create_runtime(
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
                const initial=twinkle.data.oracle;
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
        let mut runtime = crate::create_runtime(SOURCE, Some(&serialized)).unwrap();
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
        let mut runtime = crate::create_runtime(SOURCE, Some(&serialized)).unwrap();
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
fn windows_store_is_versioned_immutable_and_retains_unchanged_identity() {
    let source = r#"
            globalThis.observed=[];
            function App(){const windows=useWindows();observed.push(windows);return h(Text,null,String(windows.length))}
        "#;
    let mut runtime = crate::create_runtime(source, None).unwrap();
    let first = serde_json::json!([{"id":"1","title":"Editor","active":true,"canActivate":true}]);
    assert!(runtime.set_windows_store(&first).unwrap());
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    assert!(!runtime.set_windows_store(&first).unwrap());
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    assert!(
        runtime
            .eval_json::<bool>("observed[0] === observed[1] && observed[0][0] === observed[1][0]")
            .unwrap()
    );
    assert!(
        runtime
            .eval_json::<bool>("Object.isFrozen(observed[0]) && Object.isFrozen(observed[0][0])")
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
            .eval_json::<bool>("observed[1] !== observed[2] && observed[1][0] === observed[2][0]")
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
fn unchanged_mount_window_transport_is_bounded_and_explicit_updates_invalidate() {
    for count in [0, 16, 64, 128] {
        let mut runtime = crate::create_runtime(
            "function App(){return h(Text,null,JSON.stringify(useWindows()))}",
            None,
        )
        .unwrap();
        let base = std::rc::Rc::new(serde_json::json!({"windows":
                (0..count).map(|id| serde_json::json!({"id":id.to_string(),"title":"Original"})).collect::<Vec<_>>() }));
        let props = serde_json::json!({});
        runtime.set_mount_data(&base, None, &props).unwrap();
        let original = runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let before = runtime.bridge_value_count();
        runtime.set_mount_data(&base, None, &props).unwrap();
        assert_eq!(runtime.bridge_value_count() - before, 5);
        assert!(!runtime.reconciliation_requested().unwrap());
        runtime
            .set_windows_store(&serde_json::json!([{"id":"independent","title":"Changed"}]))
            .unwrap();
        assert_ne!(
            runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap(),
            original
        );
        runtime.set_mount_data(&base, None, &props).unwrap();
        assert_eq!(
            runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap(),
            original
        );
        runtime.begin_transaction().unwrap();
        runtime.set_windows_store(&serde_json::json!([])).unwrap();
        runtime.finish_transaction(false).unwrap();
        runtime.set_mount_data(&base, None, &props).unwrap();
        assert_eq!(
            runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap(),
            original
        );
    }
}

#[test]
fn unchanged_mount_window_publication_still_rejects_pending_render() {
    let mut runtime = crate::create_runtime(
        "function App(){return h(Text,null,String(useWindows().length))}",
        None,
    )
    .unwrap();
    let base = std::rc::Rc::new(serde_json::json!({"windows":[{"id":"one","title":"One"}]}));
    let props = serde_json::json!({});
    runtime.set_mount_data(&base, None, &props).unwrap();
    let initial: Value = runtime.eval_json("__nickelRender()").unwrap();
    // Unlike unchanged applications, even unchanged windows publication
    // is forbidden while a native render candidate awaits admission.
    let error = runtime.set_mount_data(&base, None, &props).unwrap_err();
    assert!(error.contains("cannot publish windows store"), "{error}");
    runtime.finish_patch_render(false).unwrap();
    runtime.set_mount_data(&base, None, &props).unwrap();
    assert_eq!(
        runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap(),
        initial
    );
}

#[test]
fn unchanged_mount_application_transport_is_independent_of_catalog_size() {
    for count in [0, 16, 128, 256] {
        let mut runtime = crate::create_runtime(
            "function App(){return h(Text,null,String(useApplications().length))}",
            None,
        )
        .unwrap();
        let mut base = std::rc::Rc::new(serde_json::json!({"applications":
                (0..count).map(|id| serde_json::json!({"id":id.to_string(),
                    "name":"Original","icon":"application:1","kind":"application",
                    "launchClass":"graphical"})).collect::<Vec<_>>() }));
        let props = serde_json::json!({});
        runtime.set_mount_data(&base, None, &props).unwrap();
        runtime.render("__nickelRender()", |_| Ok(())).unwrap();
        let before = runtime.bridge_value_count();
        runtime.set_mount_data(&base, None, &props).unwrap();
        assert_eq!(
            runtime.bridge_value_count() - before,
            5,
            "unchanged {count}-entry catalog crossed the bridge"
        );
        assert!(!runtime.reconciliation_requested().unwrap());
        if count > 0 {
            std::rc::Rc::make_mut(&mut base)["applications"][0]["name"] =
                Value::String("Changed".into());
            runtime.set_mount_data(&base, None, &props).unwrap();
            assert_eq!(
                runtime
                    .eval_json::<String>("JSON.stringify(__applicationsStore.snapshot[0].name)")
                    .unwrap(),
                "Changed"
            );
            assert!(runtime.reconciliation_requested().unwrap());
        }
    }
}

#[test]
fn unchanged_mount_republishes_independently_updated_application_store() {
    let mut runtime = crate::create_runtime(
        "function App(){return h(Text,null,useApplications()[0]?.name ?? 'empty')}",
        None,
    )
    .unwrap();
    let original = serde_json::json!([{"id":"editor","name":"Original",
            "icon":"application:1","kind":"application","launchClass":"graphical"}]);
    let base = std::rc::Rc::new(serde_json::json!({"applications":original}));
    let props = serde_json::json!({});
    runtime.set_mount_data(&base, None, &props).unwrap();
    let initial = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();
    let replacement = serde_json::json!([{"id":"editor","name":"Replacement",
            "icon":"application:1","kind":"application","launchClass":"graphical"}]);
    runtime.set_applications_store(&replacement).unwrap();
    let changed = runtime
        .render("__nickelRender()", |node| Ok(node.clone()))
        .unwrap();
    assert_ne!(changed, initial);
    // The immutable mount owner did not change, but its public store did.
    // Mount reuse must not retain the independently published replacement.
    runtime.set_mount_data(&base, None, &props).unwrap();
    assert_eq!(
        runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap(),
        initial
    );
    runtime.begin_transaction().unwrap();
    runtime.set_applications_store(&replacement).unwrap();
    runtime.finish_transaction(false).unwrap();
    runtime.set_mount_data(&base, None, &props).unwrap();
    assert_eq!(
        runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap(),
        initial
    );
}

#[test]
fn preview_and_window_menu_hooks_reconcile_only_their_consumers() {
    let source = r#"
            globalThis.runs={app:0,previews:0,menu:0,sibling:0};
            function Previews(){runs.previews++;return h(Text,null,String(useWindowPreviews().windows.length));}
            function MenuReader(){runs.menu++;return h(Text,null,String(useWindowMenu().targetId));}
            function Sibling(){runs.sibling++;return h(Text,null,'stable');}
            function App(){runs.app++;return h(Window,null,h(Previews),h(MenuReader),h(Sibling));}
        "#;
    let mut runtime = crate::create_runtime(source, None).unwrap();
    runtime
        .set_data_value(serde_json::json!({
            "windowPreviews":{"available":true,"windows":[]},
            "windowMenu":{"targetId":null}
        }))
        .unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    runtime
        .set_data_value(serde_json::json!({
            "windowPreviews":{"available":true,"windows":[{"id":"7"}]},
            "windowMenu":{"targetId":null}
        }))
        .unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    assert_eq!(
        runtime
            .eval_json::<serde_json::Value>("JSON.stringify(runs)")
            .unwrap(),
        serde_json::json!({"app":1,"previews":2,"menu":1,"sibling":1})
    );
}

#[test]
fn active_window_follows_public_store_order_and_updates() {
    let source = r#"
            globalThis.seen=[];
            function App(){const active=useActiveWindow();seen.push(active?.id ?? null);return h(Text,null,active?.title ?? 'none')}
        "#;
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut runtime = crate::create_runtime(
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
    let mut runtime = crate::create_runtime(
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut runtime = crate::create_runtime(
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
    let mut runtime = crate::create_runtime(
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    runtime.set_notifications_store(&serde_json::json!({"notification":{"id":1,"appName":"App","summary":"One","body":"","actions":[]},"history":[]})).unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    assert_eq!(
        runtime
            .eval_json::<serde_json::Value>("JSON.stringify(runs)")
            .unwrap(),
        serde_json::json!({"app":1,"notifications":2,"applications":1,"windows":1,"sibling":1})
    );

    let mut rejected = crate::create_runtime(
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
    let mut runtime=crate::create_runtime("globalThis.seen=[];function App(){const all=useWorkspaces();const active=useWorkspace();seen.push({all,active});return h(Text,null,active?.id??'none')}",None).unwrap();
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    runtime.set_workspaces_store(&serde_json::json!({"available":true,"revision":"one","workspaces":[{"id":"1","active":true}],"operations":{}})).unwrap();
    runtime.render("__nickelRender()", |_| Ok(())).unwrap();
    assert_eq!(
        runtime
            .eval_json::<serde_json::Value>("JSON.stringify(runs)")
            .unwrap(),
        serde_json::json!({"app":1,"workspace":2,"windows":1,"sibling":1})
    );
    let mut rejected = crate::create_runtime(
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
    let mut runtime=crate::create_runtime("globalThis.seen=[];function App(){const all=useOutputs();const current=useOutput();seen.push({all,current});return h(Text,null,current?.name??'none')}",None).unwrap();
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut rejected = crate::create_runtime(
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
fn locale_updates_are_isolated_and_rejected_subscriptions_roll_back() {
    let source = "globalThis.runs={app:0,locale:0,windows:0,sibling:0};function Locale(){runs.locale++;return h(Text,null,useLocale().tag)}function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}function Sibling(){runs.sibling++;return h(Text,null,'stable')}function App(){runs.app++;return h(Window,{},h(Locale),h(Windows),h(Sibling))}";
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
        crate::create_runtime("function App(){return h(Text,null,useLocale().tag)}", None).unwrap();
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut runtime=crate::create_runtime("function App(){return h(Text,null,useSyncExternalStore(NickelStores.locale.subscribe,NickelStores.locale.getSnapshot).tag)}",None).unwrap();
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
fn theme_store_dirties_only_changed_theme_selections() {
    let source = r#"
            globalThis.runs={app:0,mode:0,motion:0,windows:0,sibling:0};
            function Mode(){runs.mode++;return h(Text,null,useTheme(theme=>theme.mode))}
            function Motion(){runs.motion++;return h(Text,null,String(useReducedMotion()))}
            function Windows(){runs.windows++;return h(Text,null,String(useWindows().length))}
            function Sibling(){runs.sibling++;return h(Text,null,'stable')}
            function App(){runs.app++;return h(Window,{},h(Mode),h(Motion),h(Windows),h(Sibling))}
        "#;
    let mut runtime = crate::create_runtime(source, None).unwrap();
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
    let mut runtime = crate::create_runtime(
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
