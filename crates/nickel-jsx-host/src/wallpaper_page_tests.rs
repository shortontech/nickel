use super::{JsxModuleGraph, ModuleSource};
use serde_json::{Value, json};

#[test]
fn wallpaper_page_consumes_native_labels_previews_and_path_free_chooser() {
    let graph = JsxModuleGraph::new("main.js", [
        ModuleSource { path: "main.js", source: "import {Appearance} from './Appearance.js';\nexport function App(){return h(Window,{id:'settings',width:1100,height:1000},h(Appearance,{}));}" },
        ModuleSource { path:"Appearance.js", source:include_str!("../../../assets/plugins/nickel-default/src/Appearance.js") },
        ModuleSource { path:"colors.js", source:include_str!("../../../assets/plugins/nickel-default/src/colors.js") },
        ModuleSource { path:"styles/appearance.css", source:include_str!("../../../assets/plugins/nickel-default/src/styles/appearance.css") },
    ]).unwrap();
    let data = json!({"appearance":{"available":true,"writable":true,"generation":1,"configured":{"theme":"system","accent_hue":null,"accent_intensity":null,"reduce_transparency":false,"animations":"normal"},"resolved":{"hue":210,"intensity":65}},"wallpaper":{"available":true,"writable":true,"generation":4,"configured":{"custom_image_configured":false,"position":"fill"},"images":[{"id":"opaque","label":"Mountain sunrise","previewAsset":"wallpaper:opaque","configured":false}],"chooser":{"available":true,"pending":false,"result":null}}});
    let mut runtime = crate::create_module_runtime(&graph, Some(&data.to_string())).unwrap();
    let mut tree: Value = runtime
        .render("__twinkleRender()", |value| Ok(value.clone()))
        .unwrap();
    fn node<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
        if value["id"] == id {
            return Some(value);
        }
        value["children"]
            .as_array()?
            .iter()
            .find_map(|child| node(child, id))
    }
    // This runtime-only fixture supplies the viewport acknowledgement normally
    // produced by native layout. Cold JSX must not construct every catalog row.
    assert!(!tree.to_string().contains("Mountain sunrise"));
    let collection = node(&tree, "appearance-wallpapers").unwrap();
    assert_eq!(collection["collection"]["count"], 1);
    let action = collection["action"].as_u64().unwrap();
    let feedback =
        serde_json::to_string(&json!({"start":0,"end":1,"source":1}).to_string()).unwrap();
    tree = runtime
        .render(
            &format!("__twinkleDispatch({action},{feedback})"),
            |value| Ok(value.clone()),
        )
        .unwrap();
    assert!(tree.to_string().contains("Mountain sunrise"));
    assert!(tree.to_string().contains("wallpaper:opaque"));
    let action = node(&tree, "appearance-wallpaper-choose").unwrap()["action"]
        .as_u64()
        .unwrap();
    runtime
        .render(&format!("__twinkleDispatch({action})"), |value| {
            Ok(value.clone())
        })
        .unwrap();
    let effects = runtime.take_effects().unwrap();
    assert_eq!(
        effects,
        vec![
            json!({"type":"wallpaper.chooseImage","transaction":{"generation":4,"prior":{"custom_image_configured":false,"position":"fill"}}})
        ]
    );
    let mut pending = data;
    pending["wallpaper"]["chooser"]["pending"] = true.into();
    pending["wallpaper"]["chooser"]["result"] =
        json!({"status":"failed","reason":"Native image chooser failed"});
    runtime.set_data(&pending.to_string()).unwrap();
    let tree: Value = runtime
        .render("__twinkleRender()", |value| Ok(value.clone()))
        .unwrap();
    assert_eq!(
        node(&tree, "appearance-wallpaper-choose").unwrap()["disabled"],
        true
    );
    assert!(tree.to_string().contains("Native image chooser failed"));
}
