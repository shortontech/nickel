//! Standalone native TSX preview through production admission and controls.
use serde_json::{Value, json};
use twinkle::{Application, View, ViewContext};
use twinkle_jsx_runtime::{JsxModuleGraph, JsxRuntime, ModuleSource, ScheduledPatch};
use twinkle_presentation::{
    components::{PluginImages, PluginMessage, RetainedPanelTree, render_retained_panel},
    css::StyleSheet,
};
use twinkle_protocol::SurfaceBounds;

pub struct JsxPreview {
    runtime: JsxRuntime,
    accepted: RetainedPanelTree,
    bounds: SurfaceBounds,
    stylesheet: StyleSheet,
    images: PluginImages,
    confirmations: Vec<String>,
    error: Option<String>,
}

impl JsxPreview {
    pub fn new() -> Result<Self, String> {
        Self::from_sources(
            twinkle_jsx_runtime::examples::STANDALONE_TSX,
            twinkle_jsx_runtime::examples::STANDALONE_CSS,
        )
    }
    pub fn from_sources(source: &str, css: &str) -> Result<Self, String> {
        let graph = JsxModuleGraph::new(
            "standalone.tsx",
            [
                ModuleSource {
                    path: "standalone.tsx",
                    source,
                },
                ModuleSource {
                    path: "standalone.css",
                    source: css,
                },
            ],
        )?;
        let stylesheet = StyleSheet::compile(&graph.stylesheet()?)?;
        let bounds = SurfaceBounds::from_json(
            r#"{"surfaces":[{"id":"main","kind":"window","width":720,"height":480}]}"#,
        )?;
        let mut runtime = JsxRuntime::new_modules(&graph, None)?;
        runtime.set_locale_store(&json!({"tag":"en-US","direction":"ltr","known":true}))?;
        let accepted =
            render_retained_panel(&mut runtime, &bounds, Some("main"), "__nickelRender()", 1)?;
        accepted.validate_candidate(accepted.source(), &bounds, Some("main"), &stylesheet)?;
        Ok(Self {
            runtime,
            accepted,
            bounds,
            stylesheet,
            images: Default::default(),
            confirmations: Vec::new(),
            error: None,
        })
    }

    fn dispatch(&mut self, action: usize, value: Value) -> Result<(), String> {
        self.dispatch_events(vec![json!([action, value])])
    }
    fn dispatch_events(&mut self, events: Vec<Value>) -> Result<(), String> {
        self.runtime.begin_transaction()?;
        let result: Result<(RetainedPanelTree, Vec<String>), String> = (|| {
            let mut candidate = self.accepted.clone();
            match self.runtime.dispatch_batch_patched(events, false)? {
                ScheduledPatch::Unchanged => {}
                ScheduledPatch::Patched {
                    patch,
                    transport_bytes,
                    ..
                } => {
                    let generation = candidate
                        .generation()
                        .checked_add(1)
                        .ok_or("preview generation exhausted")?;
                    candidate.apply_patch(
                        &patch,
                        &self.bounds,
                        Some("main"),
                        &self.stylesheet,
                        generation,
                        transport_bytes,
                    )?;
                    self.runtime.finish_patch_render(true)?;
                }
            }
            let effects = self.runtime.take_effects()?;
            if effects.len() > 1 {
                return Err("preview accepts at most one effect per event".into());
            }
            let mut confirmations = Vec::new();
            for effect in effects {
                let object = effect.as_object().ok_or("invalid preview effect")?;
                if object.len() != 2 || effect["type"] != "confirm" {
                    return Err("unknown preview operation".into());
                }
                let query = effect["query"]
                    .as_str()
                    .filter(|value| value.len() <= 4096)
                    .ok_or("invalid confirmation value")?;
                confirmations.push(query.to_owned());
            }
            self.runtime.finish_event(true)?;
            Ok((candidate, confirmations))
        })();
        self.runtime.finish_transaction(result.is_ok())?;
        let (candidate, confirmations) = result?;
        self.accepted = candidate;
        self.confirmations.extend(confirmations);
        if self.confirmations.len() > 128 {
            self.confirmations.drain(..self.confirmations.len() - 128);
        }
        Ok(())
    }

    pub fn set_locale(&mut self, snapshot: &Value) -> Result<bool, String> {
        let changed = self.runtime.set_locale_store(snapshot)?;
        if changed {
            self.dispatch_events(Vec::new())?;
        }
        Ok(changed)
    }
    pub fn set_theme(&mut self, snapshot: &Value) -> Result<bool, String> {
        let changed = self.runtime.set_theme_store(snapshot)?;
        if changed {
            self.dispatch_events(Vec::new())?;
        }
        Ok(changed)
    }
    pub fn last_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn confirmations(&self) -> &[String] {
        &self.confirmations
    }
}

impl Application for JsxPreview {
    type Message = PluginMessage;
    fn update(&mut self, message: PluginMessage) {
        let event = match message {
            PluginMessage::Click(action) | PluginMessage::Context(action) => {
                Some((action, Value::Null))
            }
            PluginMessage::Button { click, .. } => Some((click, Value::Null)),
            PluginMessage::Text(action, value) => Some((action, Value::String(value))),
            PluginMessage::Value(action, value) => Some((action, json!(value))),
            PluginMessage::Scroll => None,
            PluginMessage::Drag(..) | PluginMessage::Drop(..) => None,
        };
        if let Some((action, value)) = event {
            self.error = self.dispatch(action, value).err();
        }
    }
    fn view(&self, _: ViewContext) -> impl View<PluginMessage> {
        self.accepted.node().view(&self.images, &self.stylesheet)
    }
    fn title(&self) -> &str {
        "Twinkle standalone TSX"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twinkle::{ActionKind, SemanticAction, SemanticValueInput, UiHost};
    #[test]
    fn native_edit_and_controller_activation_commit_typed_patches() {
        let mut host = UiHost::new(JsxPreview::new().unwrap(), 720, 480);
        let field = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Filter rows"))
            .unwrap();
        let outcome = host.perform_semantic_action(
            field.id,
            SemanticAction::SetValue(SemanticValueInput::Text("Second".into())),
        );
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(host.application().last_error(), None);
        let confirm = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Confirm"))
            .unwrap();
        let outcome = host.perform_controller_semantic_action(
            confirm.id,
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(host.application().last_error(), None);
        assert_eq!(host.application().confirmations(), ["Second"]);
    }
    #[test]
    fn focused_keyboard_text_reaches_native_editor_and_jsx_callback() {
        use twinkle_input::{DeviceId, EventOrder, InputEvent, TextEvent};
        let mut host = UiHost::new(JsxPreview::new().unwrap(), 720, 480);
        let field = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Filter rows"))
            .unwrap();
        host.request_focus(field.id.clone());
        assert!(host.input_context().text_focused);
        host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(1),
                order: EventOrder(1),
                text: "Second".into(),
            }),
            None,
        );
        assert_eq!(host.application().last_error(), None);
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.id == field.id && node.focused)
        );
        let confirm = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Confirm"))
            .unwrap();
        host.perform_semantic_action(confirm.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(host.application().confirmations(), ["Second"]);
    }
    #[test]
    fn unknown_effect_rejects_provisional_native_tree_and_state() {
        let source = "export default function App(){const [count,setCount]=useState(0);return h(Window,{id:'main',width:720,height:480},h(Button,{onClick:()=>{setCount(count+1);twinkle.request({type:'unknown'})}},'Reject '+count))}";
        let mut host = UiHost::new(JsxPreview::from_sources(source, "").unwrap(), 720, 480);
        let generation = host.application().accepted.generation();
        for _ in 0..2 {
            let button = host
                .semantic_nodes()
                .into_iter()
                .find(|node| node.name.as_deref() == Some("Reject 0"))
                .unwrap();
            host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));
            assert_eq!(
                host.application().last_error(),
                Some("unknown preview operation")
            );
            assert_eq!(host.application().accepted.generation(), generation);
            assert!(host.application().confirmations().is_empty());
        }
    }
    #[test]
    fn selected_store_changes_preserve_native_editor_identity_and_focus() {
        let mut host = UiHost::new(JsxPreview::new().unwrap(), 720, 480);
        let field = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Filter rows"))
            .unwrap();
        host.request_focus(field.id.clone());
        assert!(
            host.application_mut()
                .set_locale(&json!({"tag":"fr-FR","direction":"ltr","known":true}))
                .unwrap()
        );
        assert!(
            host.application_mut()
                .set_theme(&json!({"mode":"light","reducedMotion":true}))
                .unwrap()
        );
        host.step(twinkle::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.id == field.id && node.focused)
        );
        assert!(
            !host
                .application_mut()
                .set_locale(&json!({"tag":"fr-FR","direction":"ltr","known":true}))
                .unwrap()
        );
        assert_eq!(host.application().last_error(), None);
    }
    #[test]
    fn controller_navigation_uses_native_targets_and_activates_callback() {
        use twinkle::ControllerAction;
        let mut host = UiHost::new(JsxPreview::new().unwrap(), 720, 480);
        host.handle_event(twinkle::UiEvent::FocusGained);
        host.handle_controller_action(ControllerAction::Down);
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.controller_selected)
        );
        host.handle_controller_action(ControllerAction::Down);
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.controller_selected && node.name.as_deref() == Some("Confirm"))
        );
        host.handle_controller_action(ControllerAction::Confirm);
        assert_eq!(host.application().last_error(), None);
        assert_eq!(host.application().confirmations(), [""]);
    }
    #[test]
    fn keyed_native_rows_preserve_component_state_when_reordered() {
        let mut host = UiHost::new(JsxPreview::new().unwrap(), 720, 480);
        for label in ["First: 0", "Reverse rows"] {
            let button = host
                .semantic_nodes()
                .into_iter()
                .find(|node| node.name.as_deref() == Some(label))
                .unwrap();
            host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));
            assert_eq!(host.application().last_error(), None);
        }
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.name.as_deref() == Some("First: 1"))
        );
    }
}
