use crate::CapabilityRuntimeExt;
#[test]
fn capability_names_are_checked_against_the_rust_wire_enum() {
    for &name in crate::capabilities::PLUGIN_CAPABILITY_NAMES {
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
    let mut runtime = crate::create_runtime(source, None).unwrap();
    let grants = [PluginCapability::WindowsRead];
    assert!(
        runtime
            .set_capability_store(&grants, &serde_json::json!({"windows":[]}))
            .unwrap()
    );
    runtime.render("__twinkleRender()", |_| Ok(())).unwrap();
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
    runtime.render("__twinkleRender()", |_| Ok(())).unwrap();
    assert_eq!(
        runtime
            .eval_json::<serde_json::Value>("JSON.stringify(runs)")
            .unwrap(),
        serde_json::json!({"app":1,"windows":2,"audio":1})
    );
    assert!(runtime.eval("function Unknown(){useCapability('future-root')} __twinkleSetApp(Unknown); __twinkleRender()").is_err());
}

#[test]
fn rejected_capability_consumer_does_not_install_a_subscription() {
    use nickel_core::plugins::PluginCapability;
    let mut runtime = crate::create_runtime(
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
        .render("__twinkleRender()", |_| Err::<(), _>("reject".into()))
        .unwrap_err();
    runtime
        .set_capability_store(
            &[PluginCapability::AudioRead],
            &serde_json::json!({"audio":{"available":false}}),
        )
        .unwrap();
    assert!(
        !runtime
            .eval_json::<bool>("JSON.parse(__twinkleReconciliationRequest()).requested")
            .unwrap()
    );
}

#[test]
fn host_capability_schema_matches_shipped_declarations() {
    let declarations = include_str!("../../../assets/plugins/nickel-plugin.d.ts");
    let declared = declarations
        .split("type NickelCapability =")
        .nth(1)
        .unwrap()
        .split("interface NickelCapabilitySnapshot")
        .next()
        .unwrap();
    for name in crate::capabilities::PLUGIN_CAPABILITY_NAMES {
        assert!(declared.contains(&format!("\"{name}\"")));
    }
}
