//! Public Settings metadata bridge; executable registrations stay in V8.
use nickel_core::settings_registry::{
    SettingRegistration, SettingsPageRegistration, SettingsRegistry,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub type SettingsValueSnapshot = BTreeMap<String, BTreeMap<String, Value>>;

use crate::JsxRuntime;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registrations {
    settings: Vec<SettingRegistration>,
    pages: Vec<SettingsPageRegistration>,
}

pub trait SettingsRuntimeExt {
    fn settings_revision(&self) -> u64;
    fn read_settings_values(
        &mut self,
        registry: &SettingsRegistry,
    ) -> Result<BTreeMap<String, Value>, String>;
    fn set_settings_values(&mut self, values: &SettingsValueSnapshot) -> Result<(), String>;
    fn publish_settings(
        &mut self,
        registry: &mut SettingsRegistry,
        provider: &str,
    ) -> Result<(), String>;
    fn set_settings_registry(&mut self, registry: &SettingsRegistry) -> Result<(), String>;
    fn has_registered_page(&self, id: &str) -> bool;
    fn invoke_setting(&mut self, provider: &str, id: &str, value: &Value) -> Result<(), String>;
    fn retire_settings(&mut self, registry: &mut SettingsRegistry) -> Result<(), String>;
}

impl SettingsRuntimeExt for JsxRuntime {
    fn settings_revision(&self) -> u64 {
        self.observation_revision()
    }

    /// Evaluates provider getters only when the host observes a changed revision.
    /// Snapshot publication is atomic; invalid getter values never replace cache.
    fn read_settings_values(
        &mut self,
        registry: &SettingsRegistry,
    ) -> Result<BTreeMap<String, Value>, String> {
        let provider = self
            .registration_owner()
            .ok_or("Settings provider is unavailable")?
            .to_owned();
        let registrations = registry.settings_snapshot();
        let metadata: Registrations = self.eval_json("__nickelSettingsMetadata()")?;
        if metadata.settings.iter().any(|setting| {
            !registrations.settings.iter().any(|entry| {
                entry.provider_package == provider && entry.registration.id == setting.id
            })
        }) {
            return Err("Settings provider is disabled or unknown".into());
        }
        let values: BTreeMap<String, Value> = self.eval_json("__nickelReadSettingsValues()")?;
        let bytes = serde_json::to_vec(&values)
            .map_err(|error| error.to_string())?
            .len();
        if bytes > nickel_core::settings_registry::MAX_PROVIDER_REGISTRATION_BYTES {
            return Err("Settings values exceed snapshot limit".into());
        }
        for (id, value) in &values {
            let registration = registrations
                .settings
                .iter()
                .find(|entry| entry.provider_package == provider && entry.registration.id == *id)
                .ok_or("Settings provider is disabled or unknown")?;
            if !registration.registration.accepts_value(value) {
                return Err("Settings getter value is outside registered bounds".into());
            }
        }
        Ok(values)
    }

    fn set_settings_values(&mut self, values: &SettingsValueSnapshot) -> Result<(), String> {
        let json = serde_json::to_string(values).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelSetSettingsValues({json})"))
    }

    /// Publishes this package's registrations atomically using host-owned identity.
    /// Call after module initialization, before rendering the package.
    fn publish_settings(
        &mut self,
        registry: &mut SettingsRegistry,
        provider: &str,
    ) -> Result<(), String> {
        if self
            .registration_owner()
            .is_some_and(|current| current != provider)
        {
            return Err("cannot change Settings provider identity".into());
        }
        let metadata: Registrations = self.eval_json("__nickelSettingsMetadata()")?;
        registry.replace_provider(provider, true, metadata.settings, metadata.pages)?;
        self.set_registration_owner(Some(provider.into()));
        self.set_settings_registry(registry)
    }

    /// Refreshes visible registrations after package activation/disable/retirement.
    fn set_settings_registry(&mut self, registry: &SettingsRegistry) -> Result<(), String> {
        let provider =
            serde_json::to_string(&self.registration_owner()).map_err(|error| error.to_string())?;
        let settings = serde_json::to_string(&registry.settings_snapshot())
            .map_err(|error| error.to_string())?;
        let pages = serde_json::to_string(&registry.settings_pages_snapshot())
            .map_err(|error| error.to_string())?;
        self.eval(&format!(
            "__nickelSetSettingsRegistry({provider}, {settings}, {pages})"
        ))?;
        self.set_registered_exports(
            registry
                .settings_pages_snapshot()
                .pages
                .into_iter()
                .filter(|entry| Some(entry.provider_package.as_str()) == self.registration_owner())
                .map(|entry| entry.registration.id)
                .collect(),
        );
        Ok(())
    }

    fn has_registered_page(&self, id: &str) -> bool {
        self.has_registered_export(id)
    }

    /// Invokes the retained callback. Effects still require ordinary host validation.
    fn invoke_setting(&mut self, provider: &str, id: &str, value: &Value) -> Result<(), String> {
        let metadata: Registrations = self.eval_json("__nickelSettingsMetadata()")?;
        let registration = metadata
            .settings
            .iter()
            .find(|registration| registration.id == id)
            .ok_or("unknown Settings registration")?;
        if !registration.accepts_value(value) {
            return Err("Settings value is outside its registered type or bounds".into());
        }
        let provider = serde_json::to_string(provider).map_err(|error| error.to_string())?;
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelInvokeSetting({provider}, {id}, {value})"))?;
        self.advance_observation_revision();
        Ok(())
    }

    fn retire_settings(&mut self, registry: &mut SettingsRegistry) -> Result<(), String> {
        if let Some(provider) = self.registration_owner().map(str::to_owned) {
            registry.retire_provider(&provider);
        }
        self.set_registration_owner(None);
        self.set_registered_exports(Default::default());
        self.eval("__nickelRetireSettings()")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_provider_reads_validated_current_values_without_polling_getters() {
        let mut provider = crate::create_runtime("let reads=0; registerSetting({id:'enabled',group:'Test',label:'Enabled',type:'switch',defaultValue:false,value:()=>{reads++; return nickel.data.enabled}})", Some("{\"enabled\":true}")).unwrap();
        let mut shell = crate::create_runtime("function App(){return h(Window,{},h(Text,null,String(readPluginSettings().settings[0].value())))}", None).unwrap();
        let mut registry = SettingsRegistry::default();
        provider
            .publish_settings(&mut registry, "org.nickel.provider")
            .unwrap();
        shell
            .publish_settings(&mut registry, "org.nickel.shell")
            .unwrap();
        let values = provider.read_settings_values(&registry).unwrap();
        shell
            .set_settings_values(&BTreeMap::from([("org.nickel.provider".into(), values)]))
            .unwrap();
        assert!(
            shell
                .eval_json::<bool>("readPluginSettings().settings[0].value()")
                .unwrap()
        );
        shell
            .eval("readPluginSettings();readPluginSettings()")
            .unwrap();
        assert_eq!(provider.eval_json::<usize>("reads").unwrap(), 1);
        let revision = provider.settings_revision();
        provider
            .set_data("{\"enabled\":true,\"surface\":{\"id\":\"other\"}}")
            .unwrap();
        assert_eq!(provider.settings_revision(), revision);
        provider.set_data("{\"enabled\":false}").unwrap();
        assert_ne!(provider.settings_revision(), revision);
        shell
            .set_settings_values(&BTreeMap::from([(
                "org.nickel.provider".into(),
                provider.read_settings_values(&registry).unwrap(),
            )]))
            .unwrap();
        assert!(
            !shell
                .eval_json::<bool>("readPluginSettings().settings[0].value()")
                .unwrap()
        );
        registry.retire_provider("org.nickel.provider");
        shell.set_settings_registry(&registry).unwrap();
        assert_eq!(
            shell
                .eval_json::<usize>("readPluginSettings().settings.length")
                .unwrap(),
            0
        );
    }

    #[test]
    fn getter_effects_and_invalid_values_do_not_escape_snapshot_reads() {
        let mut registry = SettingsRegistry::default();
        let mut runtime = crate::create_runtime("registerSetting({id:'enabled',group:'Test',label:'Enabled',type:'switch',defaultValue:false,value:()=>{nickel.request({type:'forbidden'});return true}});nickel.request({type:'existing'})", None).unwrap();
        runtime
            .publish_settings(&mut registry, "org.nickel.provider")
            .unwrap();
        assert!(runtime.read_settings_values(&registry).is_err());
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"existing"})]
        );
        let mut runtime = crate::create_runtime("registerSetting({id:'enabled',group:'Test',label:'Enabled',type:'switch',defaultValue:false,value:()=>12})", None).unwrap();
        runtime
            .publish_settings(&mut registry, "org.nickel.provider")
            .unwrap();
        assert!(runtime.read_settings_values(&registry).is_err());
    }

    #[test]
    fn rejected_hook_event_preserves_value_and_settings_revision() {
        let mut runtime = crate::create_runtime("let state;registerSetting({id:'enabled',group:'Test',label:'Enabled',type:'switch',defaultValue:false,value:()=>state.current});function App(){state=useRef(false);return h(Window,{},h(Button,{onClick:()=>state.current=true},'change'))}", None).unwrap();
        let mut registry = SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "org.nickel.provider")
            .unwrap();
        runtime
            .render("__nickelRender()", |node| Ok(node.clone()))
            .unwrap();
        let revision = runtime.settings_revision();
        runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(false).unwrap();
        assert_eq!(runtime.settings_revision(), revision);
        assert_eq!(
            runtime.read_settings_values(&registry).unwrap()["enabled"],
            Value::Bool(false)
        );
        runtime
            .render("__nickelDispatch(0)", |node| Ok(node.clone()))
            .unwrap();
        runtime.finish_event(true).unwrap();
        assert_ne!(runtime.settings_revision(), revision);
        assert_eq!(
            runtime.read_settings_values(&registry).unwrap()["enabled"],
            Value::Bool(true)
        );
    }

    #[test]
    fn public_registration_retains_callbacks_components_and_host_identity() {
        let mut runtime = crate::create_runtime(r#"
            let enabled = false;
            function Details() { return h(Text, null, enabled ? 'on' : 'off'); }
            registerSetting({id:'vpn.autoConnect', group:'VPN', label:'Automatic', type:'switch', defaultValue:false, value:()=>enabled, onChange:value=>{enabled=value; nickel.request({type:'vpn.setAutoConnect', value});}});
            registerSettingsPage({id:'vpn.details', group:'VPN', component:Details});
        "#, None).unwrap();
        let mut registry = SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "org.nickel.vpn")
            .unwrap();
        assert!(
            runtime
                .invoke_setting(
                    "org.nickel.vpn",
                    "vpn.autoConnect",
                    &serde_json::json!("yes")
                )
                .unwrap_err()
                .contains("bounds")
        );
        assert!(runtime.take_effects().unwrap().is_empty());
        assert_eq!(
            registry.settings_snapshot().settings[0].provider_package,
            "org.nickel.vpn"
        );
        assert!(
            runtime
                .eval_json::<bool>("readSettingsPages().pages[0].component === Details")
                .unwrap()
        );
        runtime
            .eval("readPluginSettings().settings[0].onChange(true)")
            .unwrap();
        assert!(
            runtime
                .eval_json::<bool>("readPluginSettings().settings[0].value()")
                .unwrap()
        );
        assert_eq!(
            runtime.take_effects().unwrap()[0]["type"],
            "vpn.setAutoConnect"
        );
        registry.set_provider_enabled("org.nickel.vpn", false);
        runtime.set_settings_registry(&registry).unwrap();
        assert_eq!(
            runtime
                .eval_json::<usize>("readPluginSettings().settings.length")
                .unwrap(),
            0
        );
        assert!(
            runtime
                .invoke_setting("org.nickel.vpn", "vpn.autoConnect", &Value::Bool(false))
                .is_err()
        );
        assert!(runtime.eval("savedChange(false)").is_err());
        runtime.retire_settings(&mut registry).unwrap();
        assert!(registry.settings_pages_snapshot().pages.is_empty());
    }

    #[test]
    fn static_fragment_children_do_not_require_list_keys() {
        let mut runtime = crate::create_runtime(
            "h(Column, null, h(Fragment, null, h(Text, null, 'one'), h(Text, null, 'two')))",
            None,
        )
        .unwrap();
        assert_eq!(
            runtime
                .eval_json::<usize>("__listKeyErrors.length")
                .unwrap(),
            0
        );
    }

    #[test]
    fn registration_is_bounded_and_closed_after_publication() {
        let mut runtime = crate::create_runtime(
            "registerSetting({id:'one',group:'Test',label:'One',type:'switch',defaultValue:false})",
            None,
        )
        .unwrap();
        let mut registry = SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "org.nickel.test")
            .unwrap();
        assert!(runtime.eval("registerSetting({id:'two',group:'Test',label:'Two',type:'switch',defaultValue:false})").is_err());
        assert!(crate::create_runtime("for(let i=0;i<129;i++) registerSetting({id:'s'+i,group:'Test',label:'Test',type:'switch',defaultValue:false})", None).is_err());
    }

    #[test]
    fn malformed_metadata_never_enters_registry() {
        let mut runtime = crate::create_runtime("registerSetting({id:'bad',group:'Test',label:'Bad',type:'slider',defaultValue:9,min:0,max:1});", None).unwrap();
        let mut registry = SettingsRegistry::default();
        assert!(
            runtime
                .publish_settings(&mut registry, "org.nickel.test")
                .is_err()
        );
        assert!(registry.settings_snapshot().settings.is_empty());
        assert!(
            crate::create_runtime(
                "registerSetting({id:'bad',providerPackage:'forged'});",
                None
            )
            .is_err()
        );
        assert!(
            crate::create_runtime(
                "registerSettingsPage({id:'bad',group:'Test',component:'serialized'});",
                None
            )
            .is_err()
        );
    }
}
