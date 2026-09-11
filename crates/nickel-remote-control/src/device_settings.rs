//! Bounded device controls. Native identifiers and network payloads stay private.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    Audio,
    Wifi,
    Bluetooth,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "domain", rename_all = "snake_case", deny_unknown_fields)]
pub enum Values {
    Audio { volume_percent: u8, muted: bool },
    Wifi { powered: bool },
    Bluetooth { powered: bool, discovering: bool },
}
impl Values {
    pub fn domain(&self) -> Domain {
        match self {
            Self::Audio { .. } => Domain::Audio,
            Self::Wifi { .. } => Domain::Wifi,
            Self::Bluetooth { .. } => Domain::Bluetooth,
        }
    }
    pub fn valid(&self) -> bool {
        !matches!(self, Self::Audio{volume_percent,..} if *volume_percent > 100)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Action {
    AudioVolume(u8),
    AudioMuted(bool),
    WifiPowered(bool),
    BluetoothPowered(bool),
    BluetoothDiscovery(bool),
}
impl Action {
    pub fn domain(&self) -> Domain {
        match self {
            Self::AudioVolume(_) | Self::AudioMuted(_) => Domain::Audio,
            Self::WifiPowered(_) => Domain::Wifi,
            _ => Domain::Bluetooth,
        }
    }
    pub fn valid(&self) -> bool {
        !matches!(self, Self::AudioVolume(v) if *v > 100)
    }
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Values,
    pub action: Action,
}
impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation > 0
            && self.prior.valid()
            && self.action.valid()
            && self.prior.domain() == self.action.domain()
    }
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_micros: u64,
    pub values: Values,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Outcome {
    pub completion: crate::semantics::SurfaceSemanticCompletion,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_device_schema_rejects_cross_domain_and_secret_payloads() {
        assert!(serde_json::from_value::<Transaction>(serde_json::json!({"generation":1,"prior":{"domain":"wifi","powered":true},"action":{"kind":"wifi_powered","value":false},"password":"secret"})).is_err());
        assert!(
            !Transaction {
                generation: 1,
                prior: Values::Wifi { powered: true },
                action: Action::AudioVolume(30)
            }
            .valid()
        );
        assert!(!Action::AudioVolume(101).valid());
    }
}
