//! Public requests for the native keyboard service. Key meanings and recipient leases stay native.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct KeyboardRequest {
    pub operation: String,
    pub generation: u64,
    pub id: Option<String>,
    pub delta: Option<i32>,
}
impl KeyboardRequest {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let operation = value["type"].as_str().ok_or("missing keyboard operation")?;
        if !matches!(
            operation,
            "keyboard.press"
                | "keyboard.hide"
                | "keyboard.toggleDock"
                | "keyboard.holdModifiers"
                | "keyboard.resize"
        ) {
            return Err("unknown keyboard operation".into());
        }
        let id = if operation == "keyboard.press" {
            Some(
                value["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 64)
                    .ok_or("invalid keyboard key")?
                    .to_owned(),
            )
        } else {
            None
        };
        let delta = if operation == "keyboard.resize" {
            Some(
                value["delta"]
                    .as_i64()
                    .filter(|delta| matches!(delta, -32 | 32))
                    .ok_or("invalid keyboard resize")? as i32,
            )
        } else {
            None
        };
        Ok(Self {
            operation: operation.into(),
            generation: value["generation"]
                .as_u64()
                .ok_or("missing keyboard generation")?,
            id,
            delta,
        })
    }
    pub fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true || snapshot["generation"].as_u64() != Some(self.generation)
        {
            return Err("keyboard observation is stale or unavailable".into());
        }
        if let Some(id) = &self.id {
            if !snapshot["rows"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.as_array().is_some_and(|keys| {
                        keys.iter()
                            .any(|key| key["id"].as_str() == Some(id) && key["enabled"] == true)
                    })
                })
            }) {
                return Err("keyboard key is unavailable".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_keyboard_requests_require_current_enabled_key_identity() {
        let request = KeyboardRequest::parse(
            &serde_json::json!({"type":"keyboard.press","generation":7,"id":"key-a"}),
        )
        .unwrap();
        let snapshot = serde_json::json!({"available":true,"generation":7,"rows":[[{"id":"key-a","enabled":true}]]});
        assert!(request.validate(&snapshot).is_ok());
        let mut stale = snapshot.clone();
        stale["generation"] = 8.into();
        assert!(request.validate(&stale).is_err());
        stale = snapshot;
        stale["rows"][0][0]["enabled"] = false.into();
        assert!(request.validate(&stale).is_err());
        assert!(
            KeyboardRequest::parse(
                &serde_json::json!({"type":"keyboard.resize","generation":7,"delta":999})
            )
            .is_err()
        );
    }
}
