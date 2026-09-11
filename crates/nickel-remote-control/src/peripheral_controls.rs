//! Scrubbed printer and removable-device diagnostics with a narrow control allowlist.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_PRINTERS: usize = 64;
pub const MAX_PRINT_JOBS: usize = 256;
pub const MAX_VOLUMES: usize = 64;
pub const MAX_OPAQUE_ID_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Ready,
    Busy,
    Offline,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrintJobState {
    Pending,
    Printing,
    Held,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VolumeState {
    Unmounted,
    Mounted,
    Busy,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct PrintJob {
    /// Observation-local opaque identity. It has meaning only with this generation.
    pub id: String,
    pub state: PrintJobState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Printer {
    /// Observation-local opaque identity. Native names and addresses are omitted.
    pub id: String,
    pub state: PrinterState,
    pub is_default: bool,
    pub jobs: Vec<PrintJob>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct RemovableVolume {
    /// Observation-local opaque identity. Device and mount paths are omitted.
    pub id: String,
    pub state: VolumeState,
    pub ejectable: bool,
    pub capacity_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_micros: u64,
    pub printers: Availability,
    pub removable_volumes: Availability,
    pub printer_controls: Availability,
    pub printer_entries: Vec<Printer>,
    pub removable_volume_entries: Vec<RemovableVolume>,
    pub omitted_printers: u32,
    pub omitted_print_jobs: u32,
    pub omitted_removable_volumes: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    SetDefaultPrinter {
        printer_id: String,
        prior_is_default: bool,
    },
    CancelPrintJob {
        printer_id: String,
        job_id: String,
        prior_state: PrintJobState,
    },
}

impl Action {
    pub fn valid(&self) -> bool {
        fn opaque(value: &str) -> bool {
            !value.is_empty()
                && value.len() <= MAX_OPAQUE_ID_BYTES
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }
        match self {
            Self::SetDefaultPrinter { printer_id, .. } => opaque(printer_id),
            Self::CancelPrintJob {
                printer_id, job_id, ..
            } => opaque(printer_id) && opaque(job_id),
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub action: Action,
}

impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation > 0 && self.action.valid()
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Outcome {
    pub completion: crate::semantics::SurfaceSemanticCompletion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_accepts_only_opaque_observed_targets_and_prior_state() {
        let transaction: Transaction = serde_json::from_value(serde_json::json!({
            "generation": 7,
            "action": {
                "kind": "cancel_print_job",
                "printer_id": "printer-7-1",
                "job_id": "job-7-2",
                "prior_state": "printing"
            }
        }))
        .unwrap();
        assert!(transaction.valid());

        for forbidden in ["address", "path", "credential", "printer_name"] {
            let mut value = serde_json::json!({
                "generation": 7,
                "action": {
                    "kind": "set_default_printer",
                    "printer_id": "printer-7-1",
                    "prior_is_default": false
                }
            });
            value["action"][forbidden] = serde_json::json!("private");
            assert!(serde_json::from_value::<Transaction>(value).is_err());
        }
        assert!(
            !Action::SetDefaultPrinter {
                printer_id: "/dev/private".into(),
                prior_is_default: false,
            }
            .valid()
        );
    }
}
