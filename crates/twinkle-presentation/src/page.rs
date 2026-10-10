//! Shared JSX render and event transaction for shell and Settings hosts.

use serde_json::Value;
use std::{cell::RefCell, rc::Rc};
use twinkle_jsx_runtime::JsxRuntime;
use twinkle_protocol::{SurfaceBounds, SurfaceBoundsProvider};

use crate::components::{PanelNode, render_panel};

pub const STALE_DATA: &str = "JSX page data changed; refresh before handling input";

pub struct JsxPage {
    runtime: Rc<RefCell<JsxRuntime>>,
    runtime_scope: Option<String>,
    manifest: SurfaceBounds,
    surface_id: Option<String>,
    data: Option<String>,
    node: Option<PanelNode>,
}

impl JsxPage {
    pub fn new(
        source: &str,
        manifest: impl SurfaceBoundsProvider,
        surface_id: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: Rc::new(RefCell::new(JsxRuntime::new(source, None)?)),
            runtime_scope: None,
            manifest: SurfaceBounds::snapshot(&manifest),
            surface_id,
            data: None,
            node: None,
        })
    }

    pub fn new_with_shared_runtime(
        source: &str,
        manifest: impl SurfaceBoundsProvider,
        surface_id: Option<String>,
        runtime_scope: &str,
        runtime: Rc<RefCell<JsxRuntime>>,
    ) -> Result<Self, String> {
        runtime
            .borrow_mut()
            .register_surface_entry(runtime_scope, source)?;
        Ok(Self {
            runtime,
            runtime_scope: Some(runtime_scope.to_owned()),
            manifest: SurfaceBounds::snapshot(&manifest),
            surface_id,
            data: None,
            node: None,
        })
    }

    fn select_runtime(&self) -> Result<std::cell::RefMut<'_, JsxRuntime>, String> {
        let mut runtime = self.runtime.borrow_mut();
        if let Some(scope) = &self.runtime_scope {
            runtime.select_surface(scope)?;
        }
        Ok(runtime)
    }

    pub fn retained_bytes(&self) -> usize {
        self.data.as_ref().map_or(0, String::capacity)
            + self
                .node
                .as_ref()
                .map_or(0, |node| node.retained_bytes() as usize)
    }

    pub fn node(&self) -> Option<&PanelNode> {
        self.node.as_ref()
    }

    /// Evaluate an auxiliary JSX document with host data. This invalidates the
    /// displayed tree so an event cannot run against a different data snapshot.
    pub fn evaluate_with_data<T>(
        &mut self,
        data: &Value,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        self.data = None;
        self.node = None;
        let serialized = serde_json::to_string(data).map_err(|error| error.to_string())?;
        let mut runtime = self.select_runtime()?;
        runtime.set_data(&serialized)?;
        runtime.render(expression, parse)
    }

    pub fn render(&mut self, data: &Value) -> Result<&PanelNode, String> {
        let serialized = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.data.as_deref() != Some(&serialized) {
            let node = {
                let mut runtime = self.select_runtime()?;
                runtime.set_data(&serialized)?;
                render_panel(
                    &mut runtime,
                    &self.manifest,
                    self.surface_id.as_deref(),
                    "__nickelRender()",
                )?
            };
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
        let mut runtime = self.select_runtime()?;
        let rendered = render_panel(
            &mut runtime,
            &self.manifest,
            self.surface_id.as_deref(),
            &format!("__nickelDispatch({action},{value})"),
        );
        let effects = if rendered.is_ok() {
            runtime.take_effects()
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
        runtime.finish_event(result.is_ok())?;
        drop(runtime);
        let (node, output) = result?;
        self.node = Some(node);
        Ok(output)
    }
}

impl Drop for JsxPage {
    fn drop(&mut self) {
        if let Some(scope) = &self.runtime_scope
            && let Ok(mut runtime) = self.runtime.try_borrow_mut()
        {
            let _ = runtime.drop_surface(scope);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        components::{PluginImages, PluginMessage},
        css::StyleSheet,
    };
    use serde_json::json;

    #[test]
    fn rejected_action_restores_hooks_and_validated_action_updates_the_shared_tree() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let source = "function App() { const [count, setCount] = useState(0); return h('div', {}, h(Button, {id: 'increment', onClick: () => { setCount(count + 1); twinkle.request({type: 'increment', count: count + 1}); }}, String(count))); }";
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
    fn scoped_pages_share_one_runtime_without_mixing_handlers_or_state() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let runtime = Rc::new(RefCell::new(JsxRuntime::new("", None).unwrap()));
        let source = |name: &str| {
            format!(
                "function App() {{ const [count, setCount] = useState(0); return h(Button, {{id: 'advance', onClick: () => {{ setCount(count + 1); twinkle.request({{type: '{name}'}}); }} }}, '{name} ' + count); }}"
            )
        };
        let mut first = JsxPage::new_with_shared_runtime(
            &source("first"),
            manifest.clone(),
            None,
            "first",
            runtime.clone(),
        )
        .unwrap();
        let mut second = JsxPage::new_with_shared_runtime(
            &source("second"),
            manifest,
            None,
            "second",
            runtime.clone(),
        )
        .unwrap();
        let data = json!({});
        let first_action = first
            .render(&data)
            .unwrap()
            .button_action("advance")
            .unwrap();
        let second_action = second
            .render(&data)
            .unwrap()
            .button_action("advance")
            .unwrap();
        assert_eq!(
            first
                .dispatch(first_action, &Value::Null, &data, Ok::<_, String>)
                .unwrap(),
            json!({"type": "first"})
        );
        assert!(
            matches!(first.node(), Some(PanelNode::Button { label, .. }) if label == "first 1")
        );
        assert!(
            matches!(second.node(), Some(PanelNode::Button { label, .. }) if label == "second 0")
        );
        assert_eq!(
            second
                .dispatch(second_action, &Value::Null, &data, Ok::<_, String>)
                .unwrap(),
            json!({"type": "second"})
        );
        drop(first);
        assert!(
            !runtime
                .borrow_mut()
                .eval_json::<bool>("__surfaceApps.has('first')")
                .unwrap()
        );
        assert!(
            runtime
                .borrow_mut()
                .eval_json::<bool>("__surfaceApps.has('second')")
                .unwrap()
        );
        assert!(
            matches!(second.node(), Some(PanelNode::Button { label, .. }) if label == "second 1")
        );
    }

    #[test]
    fn auxiliary_render_invalidates_handlers_until_the_window_renders_again() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let source = "function Metadata() { return h('metadata', {value: twinkle.data.label}); } function App() { return h(Button, {id: 'go', onClick: () => twinkle.request({type: 'go'})}, 'Go'); }";
        let mut page = JsxPage::new(source, manifest, None).unwrap();
        let data = json!({"label":"Settings"});
        let action = page.render(&data).unwrap().button_action("go").unwrap();
        let label = page
            .evaluate_with_data(&data, "__nickelRender(Metadata)", |value| {
                value["value"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("metadata label is missing".into())
            })
            .unwrap();
        assert_eq!(label, "Settings");
        assert!(page.node().is_none());
        assert!(
            page.dispatch(action, &Value::Null, &data, Ok::<_, String>)
                .is_err()
        );
        let action = page.render(&data).unwrap().button_action("go").unwrap();
        assert_eq!(
            page.dispatch(action, &Value::Null, &data, Ok::<_, String>)
                .unwrap(),
            json!({"type":"go"})
        );
    }

    #[test]
    fn shared_switch_rejects_handlers_while_disabled() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let mut disabled = JsxPage::new(
            "function App() { return h(Switch, {id: 'integration', state: 'disabled-on', accessibilityLabel: 'Integration', onClick: () => twinkle.request({type: 'toggle'})}); }",
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
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
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

    #[test]
    fn color_swatch_keeps_selection_and_dispatches_its_handler() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let source = "function App() { return h(Div, {}, h(ColorSwatch, {id: 'accent', color: '#336699', selected: true, accessibilityLabel: 'Blue accent', onClick: () => twinkle.request({type: 'accent', hue: 210})}), h(ColorSwatch, {accessibilityLabel: 'Custom color', onClick: () => twinkle.request({type: 'custom'})})); }";
        let mut page = JsxPage::new(source, manifest, None).unwrap();
        let node = page.render(&json!({})).unwrap();
        let PanelNode::Div { children, .. } = node else {
            panic!("expected color swatch container")
        };
        assert!(matches!(
            &children[0],
            PanelNode::ColorSwatch {
                color: Some(0xff336699),
                selected: true,
                ..
            }
        ));
        assert!(matches!(
            &children[1],
            PanelNode::ColorSwatch {
                color: None,
                selected: false,
                ..
            }
        ));
        let action = node.button_action("accent").unwrap();
        assert_eq!(
            page.dispatch(action, &Value::Null, &json!({}), Ok::<_, String>)
                .unwrap(),
            json!({"type": "accent", "hue": 210})
        );
    }

    #[test]
    fn select_preserves_options_and_dispatches_the_chosen_action() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let source = "function App() { return h(Select, {id: 'wallpaper-position', accessibilityLabel: 'Wallpaper position', value: 'Fill', open: true, onClick: () => twinkle.request({type: 'toggle'})}, h(Option, {id: 'fill', onClick: () => twinkle.request({type: 'position', value: 'fill'})}, 'Fill'), h(Option, {id: 'fit', onClick: () => twinkle.request({type: 'position', value: 'fit'})}, 'Fit')); }";
        let mut page = JsxPage::new(source, manifest, None).unwrap();
        let node = page.render(&json!({})).unwrap();
        assert!(
            matches!(node, PanelNode::Select { value, open: true, options, .. } if value == "Fill" && options.len() == 2)
        );
        let action = node.button_action("fit").unwrap();
        assert_eq!(
            page.dispatch(action, &Value::Null, &json!({}), Ok::<_, String>)
                .unwrap(),
            json!({"type": "position", "value": "fit"})
        );
    }

    #[test]
    fn clickable_div_requires_an_accessible_label() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let mut page = JsxPage::new(
            "function App() { return h(Div, {onClick: () => twinkle.request({type: 'activate'})}, h(Text, {}, 'Visual label')); }",
            manifest,
            None,
        )
        .unwrap();
        assert!(page.render(&json!({})).is_err());
    }

    #[test]
    fn window_slot_places_host_content_inside_the_jsx_layout() {
        let manifest =
            SurfaceBounds::from_json(include_str!("../tests/fixtures/window-surfaces.json"))
                .unwrap();
        let source = "function App() { return h(Window, {id: 'main', width: '100%', height: '100%'}, h(Div, {}, h(Slot, {id: 'content'}))); }";
        let mut page = JsxPage::new(source, manifest, Some("main".into())).unwrap();
        let node = page.render(&json!({})).unwrap();
        let mut called = false;
        let stylesheet = StyleSheet::compile("window { background: #232831; }").unwrap();
        let view = node.view_as_with_slots::<PluginMessage>(
            &PluginImages::new(),
            &stylesheet,
            None,
            &mut |id| {
                assert_eq!(id, "content");
                called = true;
                Some(twinkle::AnyView::new(
                    twinkle::Container::new()
                        .id("injected-content")
                        .child(twinkle::Text::new("Host content")),
                ))
            },
        );
        let frame = twinkle::UiFrame::layout(view, twinkle::Rect::new(0.0, 0.0, 1100.0, 800.0));
        assert!(called);
        assert!(
            frame
                .accessibility_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("injected-content"))
        );
    }
}
