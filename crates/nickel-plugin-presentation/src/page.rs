//! Shared JSX render and event transaction for shell and Settings hosts.

use nickel_core::plugins::PluginManifest;
use nickel_plugin_runtime::JsxRuntime;
use serde_json::Value;

use crate::components::{PanelNode, render_panel};

pub const STALE_DATA: &str = "JSX page data changed; refresh before handling input";

pub struct JsxPage {
    runtime: JsxRuntime,
    manifest: PluginManifest,
    surface_id: Option<String>,
    data: Option<String>,
    node: Option<PanelNode>,
}

impl JsxPage {
    pub fn new(
        source: &str,
        manifest: PluginManifest,
        surface_id: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(source, None)?,
            manifest,
            surface_id,
            data: None,
            node: None,
        })
    }

    pub fn retained_bytes(&self) -> usize {
        self.data.as_ref().map_or(0, String::capacity)
            + self
                .node
                .as_ref()
                .map_or(0, |node| node.contribution_bytes() as usize)
    }

    pub fn node(&self) -> Option<&PanelNode> {
        self.node.as_ref()
    }

    pub fn render(&mut self, data: &Value) -> Result<&PanelNode, String> {
        let serialized = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.data.as_deref() != Some(&serialized) {
            self.runtime.set_data(&serialized)?;
            let node = render_panel(
                &mut self.runtime,
                &self.manifest,
                self.surface_id.as_deref(),
                "__nickelRender()",
            )?;
            self.node = Some(node);
            self.data = Some(serialized);
        }
        self.node.as_ref().ok_or("JSX page is unavailable".into())
    }

    /// Run a handler against the currently presented data. The host validates
    /// its single requested effect before the hook transaction is accepted.
    pub fn dispatch<T>(
        &mut self,
        action: usize,
        value: &Value,
        current_data: &Value,
        validate: impl FnOnce(Value) -> Result<T, String>,
    ) -> Result<T, String> {
        let serialized = serde_json::to_string(current_data).map_err(|error| error.to_string())?;
        if self.data.as_deref() != Some(&serialized) {
            return Err(STALE_DATA.into());
        }
        let rendered = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.surface_id.as_deref(),
            &format!("__nickelDispatch({action},{value})"),
        );
        let effects = if rendered.is_ok() {
            self.runtime.take_effects()
        } else {
            Ok(Vec::new())
        };
        let result: Result<(PanelNode, T), String> = (|| {
            let node = rendered?;
            let mut effects = effects?;
            if effects.len() != 1 {
                return Err("JSX action must request one operation".into());
            }
            Ok((node, validate(effects.remove(0))?))
        })();
        self.runtime.finish_event(result.is_ok())?;
        let (node, output) = result?;
        self.node = Some(node);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejected_action_restores_hooks_and_validated_action_updates_the_shared_tree() {
        let manifest =
            PluginManifest::from_json(include_str!("../../../assets/plugins/settings/plugin.json"))
                .unwrap();
        let source = "function App() { const [count, setCount] = useState(0); return h('div', {}, h(Button, {id: 'increment', onClick: () => { setCount(count + 1); nickel.request({type: 'increment', count: count + 1}); }}, String(count))); }";
        let mut page = JsxPage::new(source, manifest, None).unwrap();
        let data = json!({"generation": 1});
        let action = page
            .render(&data)
            .unwrap()
            .button_action("increment")
            .unwrap();
        assert!(
            page.dispatch(action, &Value::Null, &data, |_| -> Result<(), String> {
                Err("denied".into())
            })
            .is_err()
        );
        let PanelNode::Div { children, .. } = page.node().unwrap() else {
            panic!("expected ordinary div root")
        };
        assert!(matches!(&children[0], PanelNode::Button { label, .. } if label == "0"));
        assert!(
            page.dispatch(
                action,
                &Value::Null,
                &json!({"generation": 2}),
                Ok::<_, String>
            )
            .is_err()
        );
        let effect = page
            .dispatch(action, &Value::Null, &data, Ok::<_, String>)
            .unwrap();
        assert_eq!(effect, json!({"type": "increment", "count": 1}));
        let PanelNode::Div { children, .. } = page.node().unwrap() else {
            panic!("expected ordinary div root")
        };
        assert!(matches!(&children[0], PanelNode::Button { label, .. } if label == "1"));
    }

    #[test]
    fn shared_switch_rejects_handlers_while_disabled() {
        let manifest =
            PluginManifest::from_json(include_str!("../../../assets/plugins/settings/plugin.json"))
                .unwrap();
        let mut disabled = JsxPage::new(
            "function App() { return h(Switch, {id: 'integration', state: 'disabled-on', accessibilityLabel: 'Integration', onClick: () => nickel.request({type: 'toggle'})}); }",
            manifest.clone(),
            None,
        )
        .unwrap();
        assert!(disabled.render(&json!({})).is_err());
        let mut passive = JsxPage::new(
            "function App() { return h(Switch, {id: 'integration', state: 'disabled-on', accessibilityLabel: 'Integration'}); }",
            manifest,
            None,
        )
        .unwrap();
        assert!(matches!(
            passive.render(&json!({})),
            Ok(PanelNode::Switch { action: None, .. })
        ));
    }

    #[test]
    fn disabled_button_needs_no_handler_and_cannot_be_dispatched() {
        let manifest =
            PluginManifest::from_json(include_str!("../../../assets/plugins/settings/plugin.json"))
                .unwrap();
        let mut page = JsxPage::new(
            "function App() { return h(Button, {id: 'pending', disabled: true}, 'Pending'); }",
            manifest,
            None,
        )
        .unwrap();
        let node = page.render(&json!({})).unwrap();
        assert_eq!(node.button_action("pending"), None);
        assert!(matches!(node, PanelNode::Button { disabled: true, .. }));
    }
}
