//! Shared JavaScript evaluator for Nickel's native JSX hosts.
//!
//! Hosts own component validation and effect authority. This crate owns only
//! the Boa context and the bootstrap's render and event transactions.

use boa_engine::{Context, Source};
use serde::de::DeserializeOwned;
use serde_json::Value;

const BOOTSTRAP: &str = include_str!("../../../assets/plugin-runtime/bootstrap.js");

pub struct JsxRuntime {
    context: Context,
}

impl JsxRuntime {
    pub fn new(source: &str, data: Option<&str>) -> Result<Self, String> {
        let mut runtime = Self {
            context: Context::default(),
        };
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
        let source = "function App() { const [count, setCount] = useState(0); return h(Panel, {}, h(Button, {onClick: () => setCount(count + 1)}, String(count))); }";
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
}
