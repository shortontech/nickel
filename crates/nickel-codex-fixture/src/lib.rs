use std::{fs, path::Path};

use nickel_codex::ReplayScenario;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture contains forbidden field or value: {0}")]
    Secret(String),
    #[error("fixture validation failed: {0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn validate_file(path: &Path) -> Result<ReplayScenario, FixtureError> {
    let input = fs::read_to_string(path)?;
    validate_str(&input)
}

pub fn validate_str(input: &str) -> Result<ReplayScenario, FixtureError> {
    let value: Value = serde_json::from_str(input)?;
    scan(&value, "$")?;
    let scenario: ReplayScenario = serde_json::from_value(value)?;
    for (expected, event) in (1..).zip(&scenario.events) {
        if event.sequence != expected {
            return Err(FixtureError::Invalid(format!(
                "event sequence {} should be {expected}",
                event.sequence
            )));
        }
    }
    Ok(scenario)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Client,
    Server,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub sequence: u64,
    pub direction: Direction,
    #[serde(default)]
    pub advance_ms: u64,
    pub message: Value,
}

pub fn validate_transcript_str(input: &str) -> Result<Vec<TranscriptEntry>, FixtureError> {
    let value: Value = serde_json::from_str(input)?;
    scan(&value, "$")?;
    let entries: Vec<TranscriptEntry> = serde_json::from_value(value)?;
    for (index, entry) in entries.iter().enumerate() {
        let expected = index as u64 + 1;
        if entry.sequence != expected {
            return Err(FixtureError::Invalid(format!(
                "transcript sequence {} should be {expected}",
                entry.sequence
            )));
        }
        if entry.message.get("id").is_none()
            && entry
                .message
                .get("method")
                .and_then(Value::as_str)
                .is_none()
        {
            return Err(FixtureError::Invalid(format!(
                "transcript entry {expected} has neither id nor method"
            )));
        }
        if let Some(method) = entry.message.get("method").and_then(Value::as_str) {
            const ADMITTED: &[&str] = &[
                "initialize",
                "initialized",
                "account/read",
                "model/list",
                "thread/list",
                "thread/start",
                "thread/resume",
                "turn/start",
                "turn/interrupt",
                "thread/started",
                "turn/started",
                "turn/completed",
                "item/started",
                "item/completed",
                "item/agentMessage/delta",
                "item/commandExecution/outputDelta",
                "item/commandExecution/requestApproval",
                "item/fileChange/requestApproval",
                "item/tool/requestUserInput",
                "error",
            ];
            if !ADMITTED.contains(&method) {
                return Err(FixtureError::Invalid(format!(
                    "transcript entry {expected} uses unreviewed method {method}"
                )));
            }
        }
    }
    Ok(entries)
}

fn scan(value: &Value, path: &str) -> Result<(), FixtureError> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let lower = key.to_ascii_lowercase();
                if ["token", "authorization", "cookie", "password", "secret"]
                    .iter()
                    .any(|part| lower.contains(part))
                {
                    return Err(FixtureError::Secret(format!("{path}.{key}")));
                }
                scan(value, &format!("{path}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                scan(value, &format!("{path}[{index}]"))?;
            }
        }
        Value::String(string)
            if string.starts_with("/home/") || string.starts_with("C:\\Users\\") =>
        {
            return Err(FixtureError::Secret(path.into()));
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_secrets_and_paths_fail_closed() {
        assert!(matches!(
            validate_str(r#"{"name":"bad","access_token":"x"}"#),
            Err(FixtureError::Secret(path)) if path == "$.access_token"
        ));
        assert!(matches!(
            validate_str(
                r#"{"name":"bad","models":[{"id":"/home/alice/private","display_name":"x"}]}"#
            ),
            Err(FixtureError::Secret(path)) if path == "$.models[0].id"
        ));
    }

    #[test]
    fn sequence_gaps_are_rejected() {
        assert!(matches!(
            validate_str(
                r#"{"name":"bad","events":[{"sequence":2,"kind":{"type":"account_updated"}}]}"#
            ),
            Err(FixtureError::Invalid(message)) if message == "event sequence 2 should be 1"
        ));
    }

    #[test]
    fn checked_in_canonical_scenarios_validate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let mut scenario_count = 0;
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("transcript-")
            {
                validate_file(&path).unwrap();
                scenario_count += 1;
            }
        }
        assert_eq!(scenario_count, 3);
        let transcript =
            validate_transcript_str(include_str!("../fixtures/transcript-basic.json")).unwrap();
        assert_eq!(transcript.len(), 4);
        assert_eq!(transcript.last().unwrap().sequence, transcript.len() as u64);
    }

    #[test]
    fn transcript_sanitization_rejects_secret_bearing_protocol() {
        assert!(matches!(
            validate_transcript_str(
                r#"[{"sequence":1,"direction":"client","message":{"method":"initialize","params":{"authorization":"Bearer x"}}}]"#
            ),
            Err(FixtureError::Secret(path)) if path == "$[0].message.params.authorization"
        ));
    }
}
