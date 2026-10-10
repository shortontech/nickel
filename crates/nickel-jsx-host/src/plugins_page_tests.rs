use crate::SettingsRuntimeExt;
#[test]
fn ordinary_plugins_page_registers_from_emitted_module_and_renders_inventory() {
    use super::{JsxModuleGraph, ModuleSource};
    let graph = JsxModuleGraph::new("entry.js", [
            ModuleSource {path:"entry.js",source:"import { Plugins } from './Plugins.js';\nexport default function App() { return h(Window,{id:'main',width:800,height:600},h(Plugins,{})); }"},
            ModuleSource {path:"Plugins.js",source:include_str!("../../../assets/plugins/nickel-default/src/Plugins.js")},
            ModuleSource {path:"styles/plugins.css",source:include_str!("../../../assets/plugins/nickel-default/src/styles/plugins.css")},
        ]).unwrap();
    let mut runtime = crate::create_module_runtime(&graph, Some(r#"{"plugins":{"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","name":"Example","enabled":false,"health":{"state":"running"},"grants":["windows-read"],"surfaces":[],"composition":[],"memory":{"jsHeapBytes":null,"nativeUiBytes":null,"textureBytes":null,"trackedPeakBytes":null,"timers":0,"subscriptions":0}}]}}"#)).unwrap();
    let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
    runtime
        .publish_settings(&mut registry, "nickel-default")
        .unwrap();
    assert_eq!(
        registry.settings_pages_snapshot().pages[0].registration.id,
        "plugins"
    );
    let mut tree = runtime
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap();
    // This runtime-only fixture supplies a one-card viewport; the native
    // Settings regression below owns clipping and range selection coverage.
    fn collection_action(node: &serde_json::Value) -> Option<u64> {
        if node["id"] == "settings-plugins" {
            return node["action"].as_u64();
        }
        node["children"]
            .as_array()?
            .iter()
            .find_map(collection_action)
    }
    let action = collection_action(&tree).unwrap();
    let feedback =
        serde_json::to_string(&serde_json::json!({"start":0,"end":1,"source":1}).to_string())
            .unwrap();
    tree = runtime
        .render(&format!("__twinkleDispatch({action},{feedback})"), |node| {
            Ok(node.clone())
        })
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
        .render(&format!("__twinkleDispatch({action})"), |node| {
            Ok(node.clone())
        })
        .unwrap();
    assert!(runtime.take_effects().unwrap().is_empty());
    let confirm = find(&reviewed, "plugin-review-confirm").unwrap();
    assert_ne!(confirm["disabled"], serde_json::json!(true));
    let action = confirm["action"].as_u64().unwrap();
    runtime
        .render(&format!("__twinkleDispatch({action})"), |node| {
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
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap();
    let action = find(&tree, "plugin-toggle-0").unwrap()["action"]
        .as_u64()
        .unwrap();
    runtime
        .render(&format!("__twinkleDispatch({action})"), |node| {
            Ok(node.clone())
        })
        .unwrap();
    let mut data: serde_json::Value = runtime.eval_json("JSON.stringify(nickel.data)").unwrap();
    data["plugins"]["revision"] = serde_json::json!("8");
    runtime.set_data(&data.to_string()).unwrap();
    let stale = runtime
        .render("__twinkleRender()", |node| Ok(node.clone()))
        .unwrap();
    assert_eq!(
        find(&stale, "plugin-review-confirm").unwrap()["disabled"],
        serde_json::json!(true)
    );
    assert!(runtime.take_effects().unwrap().is_empty());
}
