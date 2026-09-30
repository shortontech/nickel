//! Shared JavaScript evaluator for Nickel's native JSX hosts.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the Boa context and the bootstrap's render and event transactions.

use boa_engine::{Context, Source};
use serde::de::DeserializeOwned;
use serde_json::Value;

const BOOTSTRAP: &str = include_str!("../../../assets/plugin-runtime/bootstrap.js");
// Boa enforces this per JavaScript call frame. It bounds accidental infinite
// loops in plugin code without retaining an event or frame history.
const MAX_JS_LOOP_ITERATIONS: u64 = 100_000;

pub struct JsxRuntime {
    context: Context,
}

impl JsxRuntime {
    pub fn new(source: &str, data: Option<&str>) -> Result<Self, String> {
        let mut runtime = Self {
            context: Context::default(),
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
        self.eval(&format!("__nickelSetData({serialized_json})"))
    }

    pub fn select_surface(&mut self, id: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        self.eval(&format!("__nickelSelectSurface({id})"))
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
    use super::*;

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
