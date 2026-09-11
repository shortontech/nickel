//! Typed default-application association settings.
//!
//! Requests contain only bounded platform target and catalog identities. They
//! cannot carry executable names, command lines, or filesystem paths.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_TARGET_ID_BYTES: usize = 255;
pub const MAX_CATALOG_ID_BYTES: usize = 512;
pub const MAX_HANDLER_NAME_BYTES: usize = 256;
pub const MAX_HANDLERS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Extension,
    Mime,
    Scheme,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub kind: TargetKind,
    pub id: String,
}

impl Target {
    pub fn valid(&self) -> bool {
        let id = self.id.as_str();
        if id.is_empty()
            || id.len() > MAX_TARGET_ID_BYTES
            || id.chars().any(char::is_control)
            || id.contains(['/', '\\']) && self.kind != TargetKind::Mime
        {
            return false;
        }
        match self.kind {
            TargetKind::Extension => {
                id.len() <= 64
                    && id.starts_with('.')
                    && id[1..].chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '+')
                    })
            }
            TargetKind::Mime => {
                let Some((major, minor)) = id.split_once('/') else {
                    return false;
                };
                !major.is_empty()
                    && !minor.is_empty()
                    && !minor.contains('/')
                    && id.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(
                                character,
                                '/' | '!' | '#' | '$' | '&' | '^' | '_' | '.' | '+' | '-'
                            )
                    })
            }
            TargetKind::Scheme => {
                id.len() <= 64
                    && id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    && id.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
                    })
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Handler {
    /// Opaque identity selected from this exact observation's native catalog.
    pub catalog_id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    DirectUserChange,
    NativeConsent,
    ReadOnly,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    System,
    Policy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub target: Target,
    pub effective: Option<Handler>,
    pub handlers: Vec<Handler>,
    pub capability: Capability,
    pub scope: Scope,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub target: Target,
    pub prior_catalog_id: Option<String>,
    pub requested_catalog_id: String,
}

impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation != 0
            && self.target.valid()
            && valid_catalog_id(&self.requested_catalog_id)
            && self
                .prior_catalog_id
                .as_deref()
                .is_none_or(valid_catalog_id)
    }
}

pub fn valid_catalog_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_CATALOG_ID_BYTES && !id.chars().any(char::is_control)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TransactionOutcome {
    Confirmed { snapshot: Snapshot },
    NativeConsentRequired { generation: u64, detail: String },
    Rejected { generation: u64, detail: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_and_catalog_contracts_are_typed_and_bounded() {
        assert!(
            Target {
                kind: TargetKind::Mime,
                id: "text/plain".into()
            }
            .valid()
        );
        assert!(
            Target {
                kind: TargetKind::Extension,
                id: ".txt".into()
            }
            .valid()
        );
        assert!(
            Target {
                kind: TargetKind::Scheme,
                id: "https".into()
            }
            .valid()
        );
        assert!(
            !Target {
                kind: TargetKind::Extension,
                id: "/tmp/a".into()
            }
            .valid()
        );
        assert!(
            !Target {
                kind: TargetKind::Scheme,
                id: "file:/tmp".into()
            }
            .valid()
        );
        assert!(!valid_catalog_id("bad\nhandler"));
        assert!(!valid_catalog_id(&"x".repeat(MAX_CATALOG_ID_BYTES + 1)));
    }
}
