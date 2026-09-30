//! Public Settings metadata bridge; executable registrations stay in Boa.
use nickel_core::settings_registry::{
    SettingRegistration, SettingsPageRegistration, SettingsRegistry,
};
use serde::Deserialize;
use serde_json::Value;

use crate::JsxRuntime;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registrations {
    settings: Vec<SettingRegistration>,
    pages: Vec<SettingsPageRegistration>,
}

impl JsxRuntime {
    /// Publishes this package's registrations atomically using host-owned identity.
    /// Call after module initialization, before rendering the package.
    pub fn publish_settings(
        &mut self,
        registry: &mut SettingsRegistry,
        provider: &str,
    ) -> Result<(), String> {
        if self
            .settings_provider
            .as_deref()
            .is_some_and(|current| current != provider)
        {
            return Err("cannot change Settings provider identity".into());
        }
        let metadata: Registrations = self.eval_json("__nickelSettingsMetadata()")?;
        registry.replace_provider(provider, true, metadata.settings, metadata.pages)?;
        self.settings_provider = Some(provider.into());
        self.set_settings_registry(registry)
    }

    /// Refreshes visible registrations after package activation/disable/retirement.
    pub fn set_settings_registry(&mut self, registry: &SettingsRegistry) -> Result<(), String> {
        let provider =
            serde_json::to_string(&self.settings_provider).map_err(|error| error.to_string())?;
        let settings = serde_json::to_string(&registry.settings_snapshot())
            .map_err(|error| error.to_string())?;
        let pages = serde_json::to_string(&registry.settings_pages_snapshot())
            .map_err(|error| error.to_string())?;
        self.eval(&format!(
            "__nickelSetSettingsRegistry({provider}, {settings}, {pages})"
        ))
    }

    /// Invokes the retained callback. Effects still require ordinary host validation.
    pub fn invoke_setting(
        &mut self,
        provider: &str,
        id: &str,
        value: &Value,
    ) -> Result<(), String> {
        let provider = serde_json::to_string(provider).map_err(|error| error.to_string())?;
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelInvokeSetting({provider}, {id}, {value})"))
    }

    pub fn retire_settings(&mut self, registry: &mut SettingsRegistry) -> Result<(), String> {
        if let Some(provider) = self.settings_provider.take() {
            registry.retire_provider(&provider);
        }
        self.eval("__nickelRetireSettings()")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_registration_retains_callbacks_components_and_host_identity() {
        let mut runtime = JsxRuntime::new(r#"
            let enabled = false;
            function Details() { return h(Text, null, enabled ? 'on' : 'off'); }
            registerSetting({id:'vpn.autoConnect', group:'VPN', label:'Automatic', type:'switch', defaultValue:false, value:()=>enabled, onChange:value=>{enabled=value; nickel.request({type:'vpn.setAutoConnect', value});}});
            registerSettingsPage({id:'vpn.details', group:'VPN', component:Details});
        "#, None).unwrap();
        let mut registry = SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "org.nickel.vpn")
            .unwrap();
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
        let mut runtime = JsxRuntime::new(
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
        let mut runtime = JsxRuntime::new(
            "registerSetting({id:'one',group:'Test',label:'One',type:'switch',defaultValue:false})",
            None,
        )
        .unwrap();
        let mut registry = SettingsRegistry::default();
        runtime
            .publish_settings(&mut registry, "org.nickel.test")
            .unwrap();
        assert!(runtime.eval("registerSetting({id:'two',group:'Test',label:'Two',type:'switch',defaultValue:false})").is_err());
        assert!(JsxRuntime::new("for(let i=0;i<129;i++) registerSetting({id:'s'+i,group:'Test',label:'Test',type:'switch',defaultValue:false})", None).is_err());
    }

    #[test]
    fn malformed_metadata_never_enters_registry() {
        let mut runtime = JsxRuntime::new("registerSetting({id:'bad',group:'Test',label:'Bad',type:'slider',defaultValue:9,min:0,max:1});", None).unwrap();
        let mut registry = SettingsRegistry::default();
        assert!(
            runtime
                .publish_settings(&mut registry, "org.nickel.test")
                .is_err()
        );
        assert!(registry.settings_snapshot().settings.is_empty());
        assert!(
            JsxRuntime::new(
                "registerSetting({id:'bad',providerPackage:'forged'});",
                None
            )
            .is_err()
        );
        assert!(
            JsxRuntime::new(
                "registerSettingsPage({id:'bad',group:'Test',component:'serialized'});",
                None
            )
            .is_err()
        );
    }
}
