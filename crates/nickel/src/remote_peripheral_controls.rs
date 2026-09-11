//! Production-owner state for the scrubbed MCP peripheral projection.
use nickel_platform::{
    PeripheralOutcome, PeripheralSnapshot, PrintJobState as NativeJobState,
    PrinterState as NativePrinterState, RemotePeripheralControl, VolumeState as NativeVolumeState,
};
use nickel_remote_control::peripheral_controls as wire;
use nickel_remote_control::semantics::SurfaceSemanticCompletion as Completion;
use std::collections::{BTreeMap, VecDeque};

const MAX_OBSERVATIONS: usize = 16;

#[derive(Default)]
pub(crate) struct State {
    next_generation: u64,
    observations: VecDeque<Observation>,
}

struct Observation {
    generation: u64,
    printers: BTreeMap<String, ObservedPrinter>,
}

struct ObservedPrinter {
    native_id: String,
    is_default: bool,
    jobs: BTreeMap<String, ObservedJob>,
}

struct ObservedJob {
    native_id: String,
    state: wire::PrintJobState,
}

pub(crate) struct Prepared {
    pub action: RemotePeripheralControl,
    expected: Expected,
}

enum Expected {
    DefaultPrinter {
        native_printer: String,
    },
    CancelledJob {
        native_printer: String,
        native_job: String,
    },
}

impl State {
    pub(crate) fn invalidate(&mut self) {
        self.observations.clear();
    }

    pub(crate) fn observe(
        &mut self,
        native: PeripheralSnapshot,
        observed_at_micros: u64,
        printer_controls: bool,
    ) -> Result<wire::Snapshot, String> {
        self.project(Some(native), observed_at_micros, printer_controls)
    }

    fn project(
        &mut self,
        native: Option<PeripheralSnapshot>,
        observed_at_micros: u64,
        printer_controls: bool,
    ) -> Result<wire::Snapshot, String> {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or("peripheral observation generation exhausted")?;
        let generation = self.next_generation;
        let mut observed_printers = BTreeMap::new();
        let mut printer_entries = Vec::new();
        let mut volume_entries = Vec::new();
        let mut omitted_printers = 0_u32;
        let mut omitted_jobs = 0_u32;
        let mut omitted_volumes = 0_u32;
        let (printers, removable_volumes) = if let Some(native) = native {
            omitted_printers = native.omitted_printers.min(u32::MAX as usize) as u32;
            omitted_jobs = native.omitted_jobs.min(u32::MAX as usize) as u32;
            omitted_volumes = native.omitted_volumes.min(u32::MAX as usize) as u32;
            let printers = match native.printers {
                Ok(printers) => {
                    omitted_printers = omitted_printers.saturating_add(
                        printers
                            .len()
                            .saturating_sub(wire::MAX_PRINTERS)
                            .min(u32::MAX as usize) as u32,
                    );
                    let mut retained_jobs = 0_usize;
                    for (printer_index, printer) in
                        printers.into_iter().take(wire::MAX_PRINTERS).enumerate()
                    {
                        let opaque = format!("printer-{generation}-{printer_index}");
                        let state = printer_state(printer.state);
                        let remaining = wire::MAX_PRINT_JOBS.saturating_sub(retained_jobs);
                        omitted_jobs = omitted_jobs.saturating_add(
                            printer
                                .jobs
                                .len()
                                .saturating_sub(remaining)
                                .min(u32::MAX as usize) as u32,
                        );
                        let mut jobs = BTreeMap::new();
                        let mut projected_jobs = Vec::new();
                        for (job_index, job) in printer.jobs.into_iter().take(remaining).enumerate()
                        {
                            let job_opaque =
                                format!("job-{generation}-{printer_index}-{job_index}");
                            let job_state = job_state(job.state);
                            jobs.insert(
                                job_opaque.clone(),
                                ObservedJob {
                                    native_id: job.id,
                                    state: job_state,
                                },
                            );
                            projected_jobs.push(wire::PrintJob {
                                id: job_opaque,
                                state: job_state,
                            });
                            retained_jobs += 1;
                        }
                        observed_printers.insert(
                            opaque.clone(),
                            ObservedPrinter {
                                native_id: printer.id,
                                is_default: printer.is_default,
                                jobs,
                            },
                        );
                        printer_entries.push(wire::Printer {
                            id: opaque,
                            state,
                            is_default: printer.is_default,
                            jobs: projected_jobs,
                        });
                    }
                    wire::Availability::Available
                }
                Err(_) => wire::Availability::Unavailable,
            };
            let volumes = match native.volumes {
                Ok(volumes) => {
                    omitted_volumes = omitted_volumes.saturating_add(
                        volumes
                            .len()
                            .saturating_sub(wire::MAX_VOLUMES)
                            .min(u32::MAX as usize) as u32,
                    );
                    for (index, volume) in volumes.into_iter().take(wire::MAX_VOLUMES).enumerate() {
                        volume_entries.push(wire::RemovableVolume {
                            id: format!("volume-{generation}-{index}"),
                            state: volume_state(volume.state),
                            ejectable: volume.ejectable,
                            capacity_bytes: volume.capacity_bytes,
                            available_bytes: volume
                                .available_bytes
                                .zip(volume.capacity_bytes)
                                .map(|(available, capacity)| available.min(capacity)),
                        });
                    }
                    wire::Availability::Available
                }
                Err(_) => wire::Availability::Unavailable,
            };
            (printers, volumes)
        } else {
            (
                wire::Availability::Unavailable,
                wire::Availability::Unavailable,
            )
        };
        if self.observations.len() == MAX_OBSERVATIONS {
            self.observations.pop_front();
        }
        self.observations.push_back(Observation {
            generation,
            printers: observed_printers,
        });
        Ok(wire::Snapshot {
            generation,
            observed_at_micros,
            printers,
            removable_volumes,
            printer_controls: if printer_controls {
                wire::Availability::Available
            } else {
                wire::Availability::Unavailable
            },
            printer_entries,
            removable_volume_entries: volume_entries,
            omitted_printers,
            omitted_print_jobs: omitted_jobs,
            omitted_removable_volumes: omitted_volumes,
        })
    }

    pub(crate) fn prepare(&mut self, transaction: wire::Transaction) -> Result<Prepared, String> {
        if !transaction.valid() {
            return Err("invalid peripheral transaction".into());
        }
        let index = self
            .observations
            .iter()
            .position(|observation| observation.generation == transaction.generation)
            .ok_or("peripheral observation is stale; read again")?;
        let observation = self
            .observations
            .remove(index)
            .ok_or("peripheral observation is stale; read again")?;
        match transaction.action {
            wire::Action::SetDefaultPrinter {
                printer_id,
                prior_is_default,
            } => {
                let printer = observation
                    .printers
                    .get(&printer_id)
                    .ok_or("opaque printer identity is stale or unavailable")?;
                if printer.is_default != prior_is_default {
                    return Err("printer prior state changed; read again".into());
                }
                Ok(Prepared {
                    action: RemotePeripheralControl::SetDefaultPrinter {
                        printer_id: printer.native_id.clone(),
                        prior_is_default,
                    },
                    expected: Expected::DefaultPrinter {
                        native_printer: printer.native_id.clone(),
                    },
                })
            }
            wire::Action::CancelPrintJob {
                printer_id,
                job_id,
                prior_state,
            } => {
                let printer = observation
                    .printers
                    .get(&printer_id)
                    .ok_or("opaque printer identity is stale or unavailable")?;
                let job = printer
                    .jobs
                    .get(&job_id)
                    .ok_or("opaque print-job identity is stale or unavailable")?;
                if job.state != prior_state {
                    return Err("print-job prior state changed; read again".into());
                }
                Ok(Prepared {
                    action: RemotePeripheralControl::CancelPrintJob {
                        printer_id: printer.native_id.clone(),
                        job_id: job.native_id.clone(),
                        prior_state: native_job_state(prior_state),
                    },
                    expected: Expected::CancelledJob {
                        native_printer: printer.native_id.clone(),
                        native_job: job.native_id.clone(),
                    },
                })
            }
        }
    }
}

impl Prepared {
    pub(crate) fn completion(
        &self,
        outcome: PeripheralOutcome,
        refreshed: Option<&PeripheralSnapshot>,
    ) -> Completion {
        match outcome {
            PeripheralOutcome::Cancelled => Completion::Cancelled,
            PeripheralOutcome::Uncertain => Completion::Uncertain,
            PeripheralOutcome::Unsupported { .. } => Completion::Unavailable,
            PeripheralOutcome::Busy { .. }
            | PeripheralOutcome::AuthorizationRequired { .. }
            | PeripheralOutcome::Rejected { .. } => Completion::Unavailable,
            PeripheralOutcome::Accepted => {
                let Some(refreshed) = refreshed else {
                    return Completion::Requested;
                };
                let Ok(printers) = &refreshed.printers else {
                    return Completion::Requested;
                };
                let confirmed = match &self.expected {
                    Expected::DefaultPrinter { native_printer } => printers
                        .iter()
                        .any(|printer| printer.id == *native_printer && printer.is_default),
                    Expected::CancelledJob {
                        native_printer,
                        native_job,
                    } => printers
                        .iter()
                        .find(|printer| printer.id == *native_printer)
                        .is_some_and(|printer| {
                            printer.jobs.iter().all(|job| job.id != *native_job)
                        }),
                };
                if confirmed {
                    Completion::Confirmed
                } else {
                    Completion::Requested
                }
            }
        }
    }
}

fn printer_state(state: NativePrinterState) -> wire::PrinterState {
    match state {
        NativePrinterState::Ready => wire::PrinterState::Ready,
        NativePrinterState::Busy => wire::PrinterState::Busy,
        NativePrinterState::Offline => wire::PrinterState::Offline,
        NativePrinterState::Error => wire::PrinterState::Error,
    }
}

fn job_state(state: NativeJobState) -> wire::PrintJobState {
    match state {
        NativeJobState::Pending => wire::PrintJobState::Pending,
        NativeJobState::Printing => wire::PrintJobState::Printing,
        NativeJobState::Held => wire::PrintJobState::Held,
        NativeJobState::Failed => wire::PrintJobState::Failed,
    }
}

fn native_job_state(state: wire::PrintJobState) -> NativeJobState {
    match state {
        wire::PrintJobState::Pending => NativeJobState::Pending,
        wire::PrintJobState::Printing => NativeJobState::Printing,
        wire::PrintJobState::Held => NativeJobState::Held,
        wire::PrintJobState::Failed => NativeJobState::Failed,
    }
}

fn volume_state(state: NativeVolumeState) -> wire::VolumeState {
    match state {
        NativeVolumeState::Unmounted => wire::VolumeState::Unmounted,
        NativeVolumeState::Mounted => wire::VolumeState::Mounted,
        NativeVolumeState::Busy => wire::VolumeState::Busy,
        NativeVolumeState::Error => wire::VolumeState::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_platform::{
        FilesystemUsage, PeripheralProvider, PrintJob, Printer, RemovableVolume,
    };

    fn raw() -> PeripheralSnapshot {
        PeripheralSnapshot {
            provider: PeripheralProvider::Unsupported {
                platform: "fixture".into(),
            },
            printers: Ok(vec![Printer {
                id: "private-native-printer".into(),
                name: "Secret office printer".into(),
                is_default: false,
                state: NativePrinterState::Ready,
                jobs: vec![PrintJob {
                    id: "private-native-job".into(),
                    name: "Secret document".into(),
                    state: NativeJobState::Printing,
                }],
            }]),
            volumes: Ok(vec![RemovableVolume {
                id: "/dev/private".into(),
                name: "Private volume".into(),
                capacity_bytes: Some(100),
                available_bytes: Some(120),
                mount_path: Some("/private/path".into()),
                state: NativeVolumeState::Mounted,
                ejectable: true,
                detail: Some("secret".into()),
            }]),
            filesystems: Ok(vec![FilesystemUsage {
                id: "private".into(),
                name: "private".into(),
                mount_path: "/private/path".into(),
                capacity_bytes: 100,
                available_bytes: 50,
            }]),
            omitted_printers: 0,
            omitted_jobs: 0,
            omitted_volumes: 0,
            omitted_filesystems: 0,
        }
    }

    #[test]
    fn projection_scrubs_native_text_paths_and_clamps_capacity() {
        let mut state = State::default();
        let snapshot = state.observe(raw(), 42, true).unwrap();
        let json = serde_json::to_string(&snapshot).unwrap();
        for secret in [
            "private-native-printer",
            "Secret office printer",
            "private-native-job",
            "Secret document",
            "/dev/private",
            "/private/path",
        ] {
            assert!(!json.contains(secret));
        }
        assert_eq!(
            snapshot.removable_volume_entries[0].available_bytes,
            Some(100)
        );
        assert_eq!(state.observations.len(), 1);
    }

    #[test]
    fn observation_is_consumed_and_prior_state_is_required() {
        let mut state = State::default();
        let snapshot = state.observe(raw(), 42, true).unwrap();
        let printer = &snapshot.printer_entries[0];
        let job = &printer.jobs[0];
        let transaction = wire::Transaction {
            generation: snapshot.generation,
            action: wire::Action::CancelPrintJob {
                printer_id: printer.id.clone(),
                job_id: job.id.clone(),
                prior_state: wire::PrintJobState::Printing,
            },
        };
        let prepared = state.prepare(transaction).unwrap();
        assert!(matches!(
            prepared.action,
            RemotePeripheralControl::CancelPrintJob { .. }
        ));
        assert!(
            state
                .prepare(wire::Transaction {
                    generation: snapshot.generation,
                    action: wire::Action::SetDefaultPrinter {
                        printer_id: printer.id.clone(),
                        prior_is_default: false,
                    },
                })
                .is_err()
        );
    }
}
