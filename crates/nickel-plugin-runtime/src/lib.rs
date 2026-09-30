//! Shared JavaScript evaluator for Nickel's native JSX hosts.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the Boa context and the bootstrap's render and event transactions.

use boa_engine::{Context, Source};
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
    settings_revision: u64,
    settings_data: Option<String>,
}

impl JsxRuntime {
    pub fn new(source: &str, data: Option<&str>) -> Result<Self, String> {
        let mut runtime = Self {
            context: Context::default(),
            settings_provider: None,
            settings_revision: 0,
            settings_data: None,
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
        self.context
            .eval(Source::from_bytes(source))
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn eval_json<T: DeserializeOwned>(&mut self, source: &str) -> Result<T, String> {
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
        self.eval(&format!("__nickelSetData({serialized_json})"))?;
        // Surface geometry does not change package setting values.
        let mut data: Value =
            serde_json::from_str(serialized_json).map_err(|error| error.to_string())?;
        if let Some(object) = data.as_object_mut() {
            object.remove("surface");
        }
        let data = data.to_string();
        if self.settings_data.as_ref() != Some(&data) {
            self.settings_revision = self.settings_revision.wrapping_add(1);
            self.settings_data = Some(data);
        }
        Ok(())
    }

    pub fn select_surface(&mut self, id: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelSelectSurface({id})"))
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
        let mut runtime = JsxRuntime::new_modules(&graph, Some(r#"{"plugins":{"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","name":"Example","enabled":true,"health":{"state":"running"},"grants":["windows-read"],"surfaces":[],"composition":[],"memory":{"jsHeapBytes":null,"nativeUiBytes":null,"textureBytes":null,"trackedPeakBytes":null,"timers":0,"subscriptions":0}}]}}"#)).unwrap();
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
    fn connectivity_clients_copy_snapshots_and_emit_revision_bound_effects() {
        let mut runtime = super::JsxRuntime::new("", Some(r#"{"wifi":{"available":true,"revision":"0123456789abcdef","operations":{"connect":true,"setEnabled":true},"networks":[{"id":"stable-profile","name":"SSID"}]},"bluetooth":{"available":true,"revision":"fedcba9876543210","operations":{"connect":true},"devices":[{"id":"stable-device"}]}}"#)).unwrap();
        runtime.eval("nickel.wifi.listNetworks()[0].name = 'mutated'; nickel.wifi.connect('stable-profile'); nickel.bluetooth.connect('stable-device');").unwrap();
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
                .eval("h(Slider,{min:1,max:1,value:1,onChange:()=>{}})")
                .is_err()
        );
        assert!(
            runtime
                .eval("h(Slider,{min:0,max:10,value:11,onChange:()=>{}})")
                .is_err()
        );
        assert!(
            runtime
                .eval("h(Slider,{value:0.5,step:0,onChange:()=>{}})")
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
