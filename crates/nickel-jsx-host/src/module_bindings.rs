use crate::JsxModuleGraph;
use serde_json::Value;
pub trait NickelModuleBindings {
    fn with_nickel_component_bridge(
        self,
        contributions: Value,
        local: Value,
    ) -> Result<Self, String>
    where
        Self: Sized;
}
impl NickelModuleBindings for JsxModuleGraph {
    fn with_nickel_component_bridge(
        self,
        contributions: Value,
        local: Value,
    ) -> Result<Self, String> {
        self.with_component_bridge(contributions,local).with_host_prelude(
            include_str!("composition_bindings.js"),
            "const nickel = __nickelCompositionClient; const readPluginSettingsPages = __nickelCompositionSettingsPages; const readSettingsPages = __nickelCompositionSettingsPages;"
        )
    }
}
