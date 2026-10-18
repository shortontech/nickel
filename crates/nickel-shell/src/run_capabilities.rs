//! Presentation independent validation for parsed native Run execution.
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn revision(owner: &str, activation: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nickel.run.v1");
    digest.update(owner.as_bytes());
    digest.update(activation.to_le_bytes());
    format!("{:x}", digest.finalize())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Execute {
    pub command: String,
    pub revision: String,
}
impl Execute {
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        let command = value
            .get("command")
            .and_then(Value::as_str)
            .ok_or("Run command is missing")?
            .trim();
        if command.is_empty() || command.chars().count() > 4096 || command.contains('\0') {
            return Err("Run command is invalid");
        }
        let revision = value
            .get("revision")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or("Run revision is missing")?;
        Ok(Self {
            command: command.to_owned(),
            revision: revision.to_owned(),
        })
    }
    pub fn is_current(&self, current: &str) -> bool {
        self.revision == current
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_validation_preserves_native_argv_text_and_rejects_stale_revision() {
        let execute = Execute::parse(
            &serde_json::json!({"command":"  editor \"a b\"  ","revision":"owner:1"}),
        )
        .unwrap();
        assert_eq!(execute.command, "editor \"a b\"");
        assert!(execute.is_current("owner:1"));
        assert!(!execute.is_current("owner:2"));
        for command in [String::new(), "\0".into(), "x".repeat(4097)] {
            assert!(
                Execute::parse(&serde_json::json!({"command":command,"revision":"1"})).is_err()
            );
        }
    }
}
