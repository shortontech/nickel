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
    let insert = |declaration: &str, members: &mut std::collections::BTreeMap<String, String>| {
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
        "interface TwinkleExternalStore",
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
fn settings_schema_retirement_prunes_unmaterialized_nested_drafts() {
    let graph = super::JsxModuleGraph::new("entry.js", [
            super::ModuleSource { path: "entry.js", source: r#"
                import {draftSchema,pruneCollectionDrafts} from './SettingsDrafts.js';
                export function App(){return h(Column,{});}
                globalThis.check = () => {
                    const leaf = () => ({type:'text',state:{source:'stored',text:'draft'}});
                    const schema = [{id:'keep',type:'text'},
                        {id:'group',type:'group',fields:[{id:'keep',type:'text'}]},
                        {id:'nested',type:'repeated',fields:[{id:'keep',type:'text'}]}];
                    const makeRow = () => ({controls:{keep:leaf(),removed:leaf(),changed:leaf(),
                        group:{type:'group',state:{keep:leaf(),removed:leaf()}},
                        nested:{type:'repeated',state:{rows:[{controls:{keep:leaf(),removed:leaf()}}],next:1}}},
                        drafts:{'text:keep':{source:'stored',text:'draft'},'text:removed':{}},
                        collections:{removed:{rows:[],next:0},nested:{rows:[{drafts:{'text:keep':{},'text:removed':{}}}],next:1}}});
                    const identity = {rows:Array.from({length:1000},makeRow),next:1000};
                    const before = JSON.stringify(draftSchema(schema));
                    const relabeled = schema.map(field=>({...field,label:'New label'}));
                    if (JSON.stringify(draftSchema(relabeled))!==before) throw Error('label invalidates drafts');
                    pruneCollectionDrafts(identity,schema.concat({id:'changed',type:'number'}));
                    for (const row of identity.rows) {
                        if (Object.keys(row.controls).join(',')!=='keep,group,nested') throw Error('stale controls');
                        if (Object.keys(row.controls.group.state).join(',')!=='keep') throw Error('stale grouped draft');
                        if (Object.keys(row.controls.nested.state.rows[0].controls).join(',')!=='keep') throw Error('stale nested control');
                        if (Object.keys(row.drafts).join(',')!=='text:keep') throw Error('stale scoped draft');
                        if (Object.keys(row.collections).join(',')!=='nested') throw Error('stale collection');
                        if (Object.keys(row.collections.nested.rows[0].drafts).join(',')!=='text:keep') throw Error('stale nested scope');
                        if (row.controls.keep.state.text!=='draft') throw Error('compatible draft lost');
                    }
                    pruneCollectionDrafts(identity,[]);
                    return identity.rows.every(row=>!Object.keys(row.controls).length &&
                        !Object.keys(row.drafts).length && !Object.keys(row.collections).length);
                };
            "# },
            super::ModuleSource { path: "SettingsDrafts.js", source: include_str!("../../../assets/plugins/nickel-default/src/SettingsDrafts.js") },
        ]).unwrap();
    let mut runtime = crate::create_module_runtime(&graph, None).unwrap();
    assert!(
        runtime
            .eval_json::<bool>("JSON.stringify(check())")
            .unwrap()
    );
}

#[test]
fn repeated_setting_drafts_survive_control_unmount_but_not_source_or_schema_replacement() {
    let graph = super::JsxModuleGraph::new("entry.js",[
            super::ModuleSource {path:"entry.js",source:r#"
                import {SettingsDraftContext,draftPaths,rowDraftScope,useSettingDraft,useCollectionIdentity} from './SettingsDrafts.js';
                const row = {};
                const scope = rowDraftScope(row,'row/',draftPaths([{id:'name',type:'text'},{id:'nested',type:'repeated'}]));
                globalThis.draftCount=()=>Object.keys(scope.drafts).length;
                globalThis.pruneDrafts=()=>rowDraftScope(row,'row/',draftPaths([]));
                function Field({id='field',controlId='row/name'}){const [value,onChange]=useSettingDraft(nickel.data.value,controlId,'text');
                    return h(TextField,{id,value,onChange});}
                function Nested(){const identity=useCollectionIdentity('row/nested');
                    const child=identity.rows[0]||(identity.rows[0]={});
                    const nested=rowDraftScope(child,'row/nested/child/',draftPaths([{id:'name',type:'text'}]));
                    return h(SettingsDraftContext.Provider,{value:nested},
                        h(Field,{id:'nested-field',controlId:'row/nested/child/name'}));}
                export function App(){const [show,setShow]=useState(true);
                    return h(Column,{},h(Button,{id:'toggle',onClick:()=>setShow(!show)},'Toggle'),
                        h(SettingsDraftContext.Provider,{value:scope},show?h(Column,{},h(Field,{}),h(Nested,{})):null));}
            "#},
            super::ModuleSource {path:"SettingsDrafts.js",source:include_str!("../../../assets/plugins/nickel-default/src/SettingsDrafts.js")},
        ]).unwrap();
    fn find<'a>(tree: &'a serde_json::Value, id: &str) -> Option<&'a serde_json::Value> {
        if tree["id"] == id {
            return Some(tree);
        }
        tree["children"]
            .as_array()?
            .iter()
            .find_map(|child| find(child, id))
    }
    fn dispatch(
        runtime: &mut super::JsxRuntime,
        tree: &serde_json::Value,
        id: &str,
        value: serde_json::Value,
    ) -> serde_json::Value {
        let action = find(tree, id).unwrap()["action"].as_u64().unwrap();
        runtime
            .render(&format!("__twinkleDispatch({action},{value})"), |node| {
                Ok(node.clone())
            })
            .unwrap()
    }
    let mut runtime = crate::create_module_runtime(&graph, Some(r#"{"value":"stored"}"#)).unwrap();
    let mut tree = runtime
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap();
    tree = dispatch(&mut runtime, &tree, "field", serde_json::json!("unapplied"));
    tree = dispatch(
        &mut runtime,
        &tree,
        "nested-field",
        serde_json::json!("nested draft"),
    );
    assert_eq!(find(&tree, "field").unwrap()["value"], "unapplied");
    tree = dispatch(&mut runtime, &tree, "toggle", serde_json::Value::Null);
    assert!(find(&tree, "field").is_none());
    assert_eq!(
        runtime
            .eval_json::<u64>("JSON.stringify(draftCount())")
            .unwrap(),
        1
    );
    tree = dispatch(&mut runtime, &tree, "toggle", serde_json::Value::Null);
    assert_eq!(find(&tree, "field").unwrap()["value"], "unapplied");
    assert_eq!(
        find(&tree, "nested-field").unwrap()["value"],
        "nested draft"
    );
    for value in ["external", "stored"] {
        runtime
            .set_data(&serde_json::json!({"value":value}).to_string())
            .unwrap();
        tree = runtime
            .render("__twinkleRender()", |node| Ok(node.clone()))
            .unwrap();
        assert_eq!(find(&tree, "field").unwrap()["value"], value);
        assert_eq!(
            runtime
                .eval_json::<u64>("JSON.stringify(draftCount())")
                .unwrap(),
            0
        );
    }
    tree = dispatch(
        &mut runtime,
        &tree,
        "field",
        serde_json::json!("another draft"),
    );
    tree = dispatch(&mut runtime, &tree, "toggle", serde_json::Value::Null);
    runtime.eval("pruneDrafts()").unwrap();
    tree = dispatch(&mut runtime, &tree, "toggle", serde_json::Value::Null);
    assert_eq!(find(&tree, "field").unwrap()["value"], "stored");
    assert!(
        runtime.take_effects().unwrap().is_empty(),
        "draft edits must not write settings"
    );
}

#[test]
fn settings_collection_identity_matching_is_linear_and_preserves_duplicate_keys() {
    let graph = super::JsxModuleGraph::new("entry.js", [
            super::ModuleSource { path:"entry.js", source:r#"
                import {reconcileRows} from './settings-collection.js';
                export function App(){return h(Column,{});}
                globalThis.checkRows = count => {
                        const original = JSON.stringify;
                        const values = Array.from({length:count},(_,index)=>({name:'Row '+index,nested:{enabled:true}}));
                        const state = {rows:[],next:0};
                        reconcileRows(state,values);
                        let calls = 0;
                        JSON.stringify = (...args) => {calls++;return original(...args);};
                        reconcileRows(state,[...values].reverse());
                        const references = calls;
                        calls = 0;
                        reconcileRows(state,values.map(value=>({name:value.name,nested:{enabled:true}})));
                        JSON.stringify = original;
                        if (state.rows.some((row,index)=>row.key!=='row-'+index)) throw Error('reorder changed row identity');
                        const stable = state.rows;
                        if (reconcileRows(state,values.map(value=>({name:value.name,nested:{enabled:true}}))) !== stable)
                            throw Error('equal transport replaced the logical sequence');
                        if (reconcileRows(state,[...values].reverse()) === stable)
                            throw Error('reorder failed to replace the logical sequence');
                        return {count,references,calls};
                };
                globalThis.checkDuplicateRows = () => {
                    function oracle(state,values) {
                        const available=state.rows.slice();
                        const rows=values.map(value=>{
                            const encoded=JSON.stringify(value);
                            let index=available.findIndex(row=>row.value===value);
                            if(index<0) index=available.findIndex(row=>JSON.stringify(row.value)===encoded);
                            const row=index<0?{key:'row-'+state.next++,value}:available.splice(index,1)[0];
                            row.value=value;return row;
                        });state.rows=rows;return rows;
                    }
                    const a={v:1},b={v:1},c={v:2};
                    const actual={rows:[],next:0},expected={rows:[],next:0};
                    for (const values of [[a,b,c],[b,{v:1},c],[c,a,b],[{v:1},a],[{v:3},a,b],[],[a]]) {
                        const got=reconcileRows(actual,values).map(row=>row.key);
                        const want=oracle(expected,values).map(row=>row.key);
                        if(JSON.stringify(got)!==JSON.stringify(want)||actual.next!==expected.next)
                            throw Error('duplicate/remove/reinsert identity diverged');
                    }
                    return true;
                };
            "# },
            super::ModuleSource {path:"settings-collection.js",source:include_str!("../../../assets/plugins/nickel-default/src/settings-collection.js")},
        ]).unwrap();
    let mut runtime = crate::create_module_runtime(&graph, None).unwrap();
    for count in [100_u64, 1_000, 5_000] {
        // Keep each cardinality in its own bounded execution. The production
        // deadline protects every host call; this test checks algorithmic work
        // counts and must not combine independent workloads into one budget.
        let report: serde_json::Value = runtime
            .eval_json(&format!("JSON.stringify(globalThis.checkRows({count}))"))
            .unwrap();
        let count = report["count"].as_u64().unwrap();
        assert_eq!(report["references"], 0);
        assert_eq!(report["calls"], 2 * count);
    }
    assert!(runtime.eval_json::<bool>("checkDuplicateRows()").unwrap());
}

#[test]
fn ordinary_connectivity_pages_preserve_disconnect_pair_and_native_details() {
    let graph = super::JsxModuleGraph::new("entry.js",[
            super::ModuleSource {path:"entry.js",source:"import { Wifi } from './Wifi.js';\nimport { Bluetooth } from './Bluetooth.js';\nexport default function App(){return h(Column,{},h(Wifi,{}),h(Bluetooth,{}));}"},
            super::ModuleSource {path:"Wifi.js",source:include_str!("../../../assets/plugins/nickel-default/src/Wifi.js")},
            super::ModuleSource {path:"Bluetooth.js",source:include_str!("../../../assets/plugins/nickel-default/src/Bluetooth.js")},
            super::ModuleSource {path:"styles/connectivity.css",source:include_str!("../../../assets/plugins/nickel-default/src/styles/connectivity.css")},
        ]).unwrap();
    let mut runtime = crate::create_module_runtime(&graph,Some(r#"{"wifi":{"available":true,"enabled":true,"revision":"0123456789abcdef","operations":{"disconnect":true},"adaptersAvailable":true,"adapters":[{"id":"adapter-eth0","name":"eth0","description":"Ethernet","connected":true,"speedBitsPerSecond":null}],"networks":[{"id":"profile","name":"SSID","connected":true,"canDisconnect":true,"signalPercent":80}]},"bluetooth":{"available":true,"powered":true,"revision":"fedcba9876543210","adapterName":"Native radio","operations":{"pair":true},"devices":[{"id":"device","name":"Headset","paired":false,"connected":false,"batteryPercent":75,"signalDbm":-42,"kind":"audio-card"}]}}"#)).unwrap();
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
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap();
    let mut tree = tree;
    assert!(!tree.to_string().contains("Battery: 75%"));
    // Runtime-only viewport feedback; native integration is covered by the
    // production Settings host tests in nickel.
    for (index, id) in [
        "settings-wifi-networks",
        "settings-wifi-adapters",
        "settings-bluetooth-devices",
    ]
    .into_iter()
    .enumerate()
    {
        let collection = find(&tree, id).unwrap();
        assert_eq!(collection["collection"]["count"], 1);
        let action = collection["action"].as_u64().unwrap();
        let feedback = serde_json::to_string(
            &serde_json::json!({"start":0,"end":1,"source":index+1}).to_string(),
        )
        .unwrap();
        tree = runtime
            .render(
                &format!("__twinkleDispatch({action},{feedback})"),
                |value| Ok(value.clone()),
            )
            .unwrap();
    }
    assert!(tree.to_string().contains("Battery: 75%"));
    assert!(tree.to_string().contains("eth0"));
    let action = find(&tree, "settings-wifi-disconnect/profile").unwrap()["action"]
        .as_u64()
        .unwrap();
    let tree = runtime
        .render(&format!("__twinkleDispatch({action})"), |node| {
            Ok(node.clone())
        })
        .unwrap();
    let action = find(&tree, "settings-bluetooth-pair/device").unwrap()["action"]
        .as_u64()
        .unwrap();
    runtime
        .render(&format!("__twinkleDispatch({action})"), |node| {
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
fn plugin_declarations_match_the_bidirectional_runtime_surface() {
    let entrypoint = include_str!("../../../assets/plugin-runtime/index.d.ts");
    assert!(
        entrypoint.contains(r#"<reference path="../plugins/nickel-plugin.d.ts" />"#),
        "plugin-runtime/index.d.ts must route to the canonical ambient declarations"
    );
    let declarations = concat!(
        include_str!("../../../assets/plugins/twinkle.d.ts"),
        "\n",
        include_str!("../../../assets/plugins/nickel-plugin.d.ts")
    );
    let declared = declared_runtime_surface(declarations);
    let mut runtime =
        crate::create_runtime("function App(){return h(Text,null,'ok')}", None).unwrap();
    let runtime_globals = runtime
        .eval_json::<std::collections::BTreeSet<String>>(
            "JSON.stringify(__twinklePublicRuntimeGlobals)",
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
    for (namespace, declared_members) in declared.iter().filter(|(name, _)| *name != "globalThis") {
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
}

#[test]
fn bidirectional_surface_comparison_rejects_both_drift_directions() {
    let declared = std::collections::BTreeSet::from(["shared".into(), "typedOnly".into()]);
    let runtime = std::collections::BTreeSet::from(["shared".into(), "runtimeOnly".into()]);
    let (declaration_only, runtime_only) = surface_difference(&declared, &runtime);
    assert_eq!(declaration_only, ["typedOnly"]);
    assert_eq!(runtime_only, ["runtimeOnly"]);
}
