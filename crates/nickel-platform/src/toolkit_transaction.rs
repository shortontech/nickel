//! Shared local/MCP scale policy transaction. Every external write has a durable
//! intent first; interrupted commands remain uncertain rather than losing ownership.
use crate::toolkit_scale::{
    ToolkitFamily, ToolkitRejection, ToolkitScaleBackend, ToolkitWriteError,
    canonical_toolkit_value,
};
use nickel_core::dpi::{ApplicationScalePolicy, ApplicationScaleSettings, ToolkitScaleIntent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitOutcomeKind {
    Unchanged,
    Confirmed,
    ExternalConflict,
    Unavailable,
    Failed,
    Uncertain,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolkitOutcome {
    pub family: ToolkitFamily,
    pub kind: ToolkitOutcomeKind,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScaleTransactionReport {
    pub outcomes: Vec<ToolkitOutcome>,
}
fn fields(
    settings: &mut ApplicationScaleSettings,
    family: ToolkitFamily,
) -> (
    &mut Option<String>,
    &mut Option<String>,
    &mut Option<ToolkitScaleIntent>,
) {
    match family {
        ToolkitFamily::Gtk => (
            &mut settings.owned_gtk_previous,
            &mut settings.owned_gtk_applied,
            &mut settings.pending_gtk,
        ),
        ToolkitFamily::Qt => (
            &mut settings.owned_qt_previous,
            &mut settings.owned_qt_applied,
            &mut settings.pending_qt,
        ),
    }
}

/// The caller owns serialization/CAS and cancellation. `persist` must stage and
/// authorize each config replacement; `backend.write` must authorize each setter.
/// No rollback or later write is attempted when either callback reports cancellation.
pub fn transact_application_scale(
    backend: &dyn ToolkitScaleBackend,
    settings: &mut ApplicationScaleSettings,
    requested: ApplicationScalePolicy,
    mut persist: impl FnMut(&ApplicationScaleSettings) -> Result<(), String>,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<ScaleTransactionReport, String> {
    let mut report = ScaleTransactionReport::default();
    check()?;
    for capability in backend.capabilities().into_iter().take(2) {
        check()?;
        let family = capability.family;
        let outcome = |kind| ToolkitOutcome {
            family,
            kind,
            restart_required: capability.restart_required,
        };
        if !capability.available {
            report
                .outcomes
                .push(outcome(ToolkitOutcomeKind::Unavailable));
            continue;
        }
        let current = match backend
            .read(family)
            .and_then(|value| canonical_toolkit_value(family, &value))
        {
            Ok(value) => value,
            Err(_) => {
                check()?;
                report.outcomes.push(outcome(ToolkitOutcomeKind::Failed));
                continue;
            }
        };
        check()?;
        let (previous, applied, pending) = fields(settings, family);
        if let Some(intent) = pending.clone() {
            // A previously accepted setter may still finish. Do not clear an
            // unresolved intent merely because the old value is visible now.
            if !intent.terminal || current != intent.requested {
                report.outcomes.push(outcome(ToolkitOutcomeKind::Uncertain));
                continue;
            }
            if intent.restoring {
                *previous = None;
                *applied = None;
            } else {
                if previous.is_none() {
                    *previous = Some(intent.previous);
                }
                *applied = Some(intent.requested);
            }
            *pending = None;
            persist(settings)?;
        }
        let (previous, applied, pending) = fields(settings, family);
        let target = match requested {
            ApplicationScalePolicy::Unchanged => None,
            ApplicationScalePolicy::FollowNickel => match (previous.as_ref(), applied.as_ref()) {
                (Some(previous), Some(applied)) => {
                    let applied = canonical_toolkit_value(family, applied)?;
                    if current != applied {
                        report
                            .outcomes
                            .push(outcome(ToolkitOutcomeKind::ExternalConflict));
                        continue;
                    }
                    Some(canonical_toolkit_value(family, previous)?)
                }
                _ => None,
            },
            ApplicationScalePolicy::Custom(scale) => {
                if let Some(applied) = applied.as_ref()
                    && current != canonical_toolkit_value(family, applied)?
                {
                    report
                        .outcomes
                        .push(outcome(ToolkitOutcomeKind::ExternalConflict));
                    continue;
                }
                Some(match family {
                    ToolkitFamily::Gtk => scale.integer_buffer_scale().to_string(),
                    ToolkitFamily::Qt => format!("{:.6}", scale.factor()),
                })
            }
        };
        let Some(target) = target else {
            report.outcomes.push(outcome(ToolkitOutcomeKind::Unchanged));
            continue;
        };
        if target == current && requested != ApplicationScalePolicy::FollowNickel {
            report.outcomes.push(outcome(ToolkitOutcomeKind::Unchanged));
            continue;
        }
        if !backend.writable(family).unwrap_or(false) {
            check()?;
            report
                .outcomes
                .push(outcome(ToolkitOutcomeKind::Unavailable));
            continue;
        }
        check()?;
        *pending = Some(ToolkitScaleIntent {
            previous: current.clone(),
            requested: target.clone(),
            restoring: requested == ApplicationScalePolicy::FollowNickel,
            terminal: false,
        });
        persist(settings)?;
        check()?;
        // Compare again after persistence; an external writer must not be
        // overwritten merely because it changed during journal staging.
        let observed_before_send = backend
            .read(family)
            .and_then(|value| canonical_toolkit_value(family, &value));
        let failure = match observed_before_send {
            Ok(observed) if observed != current => Some(ToolkitWriteError::NotAccepted(
                ToolkitRejection::ExternalConflict,
            )),
            Err(_) => Some(ToolkitWriteError::NotAccepted(ToolkitRejection::Failed)),
            Ok(_) => {
                check()?;
                backend.write_checked(family, &target, &current).err()
            }
        };
        if let Some(error) = failure {
            check()?;
            match error {
                ToolkitWriteError::NotAccepted(reason) => {
                    *fields(settings, family).2 = None;
                    persist(settings)?;
                    report.outcomes.push(outcome(match reason {
                        ToolkitRejection::Failed => ToolkitOutcomeKind::Failed,
                        ToolkitRejection::Unavailable => ToolkitOutcomeKind::Unavailable,
                        ToolkitRejection::ExternalConflict => ToolkitOutcomeKind::ExternalConflict,
                    }));
                }
                ToolkitWriteError::Uncertain => {
                    report.outcomes.push(outcome(ToolkitOutcomeKind::Uncertain))
                }
            }
            continue;
        }
        check()?;
        // A successful typed setter means native terminal completion, never
        // merely helper spawn, queue admission, socket send, or value equality.
        fields(settings, family).2.as_mut().unwrap().terminal = true;
        persist(settings)?;
        check()?;
        let observed = backend
            .read(family)
            .and_then(|value| canonical_toolkit_value(family, &value));
        check()?;
        if observed.as_ref().ok() != Some(&target) {
            report.outcomes.push(outcome(ToolkitOutcomeKind::Uncertain));
            continue;
        }
        let (previous, applied, pending) = fields(settings, family);
        if requested == ApplicationScalePolicy::FollowNickel {
            *previous = None;
            *applied = None;
        } else {
            if previous.is_none() {
                *previous = Some(current);
            }
            *applied = Some(target);
        }
        *pending = None;
        persist(settings)?;
        report.outcomes.push(outcome(ToolkitOutcomeKind::Confirmed));
    }
    check()?;
    settings.policy = requested;
    persist(settings)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolkitCapability;
    use nickel_core::dpi::Scale120;
    use std::cell::{Cell, RefCell};
    struct Backend {
        value: RefCell<String>,
        writes: Cell<usize>,
        cancel: Cell<bool>,
        cancel_on_write: bool,
        accept: bool,
    }
    impl Backend {
        fn new() -> Self {
            Self {
                value: RefCell::new("1.000000".into()),
                writes: Cell::new(0),
                cancel: Cell::new(false),
                cancel_on_write: false,
                accept: true,
            }
        }
    }
    impl ToolkitScaleBackend for Backend {
        fn capabilities(&self) -> Vec<ToolkitCapability> {
            vec![ToolkitCapability {
                family: ToolkitFamily::Qt,
                available: true,
                live: false,
                restart_required: true,
            }]
        }
        fn read(&self, _: ToolkitFamily) -> Result<String, String> {
            Ok(self.value.borrow().clone())
        }
        fn write(&self, _: ToolkitFamily, value: &str) -> Result<(), String> {
            self.writes.set(self.writes.get() + 1);
            if self.accept {
                *self.value.borrow_mut() = value.into();
            }
            if self.cancel_on_write {
                self.cancel.set(true);
            }
            Ok(())
        }
    }
    fn custom(value: u32) -> ApplicationScalePolicy {
        ApplicationScalePolicy::Custom(Scale120::new(value).unwrap())
    }
    fn run(
        backend: &Backend,
        settings: &mut ApplicationScaleSettings,
        policy: ApplicationScalePolicy,
    ) -> ScaleTransactionReport {
        transact_application_scale(backend, settings, policy, |_| Ok(()), || Ok(())).unwrap()
    }
    #[test]
    fn repeated_custom_writes_preserve_first_owned_prior_and_reset_it() {
        let backend = Backend::new();
        let mut settings = ApplicationScaleSettings::default();
        run(&backend, &mut settings, custom(150));
        run(&backend, &mut settings, custom(180));
        assert_eq!(settings.owned_qt_previous.as_deref(), Some("1.000000"));
        assert_eq!(backend.value.borrow().as_str(), "1.500000");
        run(
            &backend,
            &mut settings,
            ApplicationScalePolicy::FollowNickel,
        );
        assert_eq!(backend.value.borrow().as_str(), "1.000000");
        assert!(settings.owned_qt_applied.is_none());
    }
    #[test]
    fn reset_does_not_overwrite_external_change() {
        let backend = Backend::new();
        let mut settings = ApplicationScaleSettings::default();
        run(&backend, &mut settings, custom(150));
        *backend.value.borrow_mut() = "2.000000".into();
        let report = run(
            &backend,
            &mut settings,
            ApplicationScalePolicy::FollowNickel,
        );
        assert_eq!(
            report.outcomes[0].kind,
            ToolkitOutcomeKind::ExternalConflict
        );
        assert_eq!(backend.writes.get(), 1);
        assert!(settings.owned_qt_applied.is_some());
    }
    #[test]
    fn cancellation_after_setter_keeps_durable_intent_and_starts_no_later_write() {
        let mut backend = Backend::new();
        backend.cancel_on_write = true;
        let mut settings = ApplicationScaleSettings::default();
        let mut persisted = Vec::new();
        let result = transact_application_scale(
            &backend,
            &mut settings,
            custom(150),
            |settings| {
                persisted.push(settings.clone());
                Ok(())
            },
            || {
                if backend.cancel.get() {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(backend.writes.get(), 1);
        assert_eq!(persisted.len(), 1);
        let mut durable = persisted.pop().unwrap();
        assert!(durable.pending_qt.is_some());
        assert!(durable.owned_qt_applied.is_none());
        backend.cancel.set(false);
        let report = run(&backend, &mut durable, custom(150));
        assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Uncertain);
        assert!(durable.pending_qt.is_some());
        assert_eq!(backend.writes.get(), 1);
    }
    #[test]
    fn unresolved_accepted_write_never_claims_success_or_retries_setter() {
        let mut backend = Backend::new();
        backend.accept = false;
        let mut settings = ApplicationScaleSettings::default();
        let report = run(&backend, &mut settings, custom(150));
        assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Uncertain);
        assert!(settings.pending_qt.is_some());
        let report = run(&backend, &mut settings, custom(180));
        assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Uncertain);
        assert_eq!(backend.writes.get(), 1);
    }
    #[test]
    fn terminal_receipt_allows_later_reconciliation_without_reissuing_setter() {
        let mut backend = Backend::new();
        backend.accept = false;
        let mut settings = ApplicationScaleSettings::default();
        run(&backend, &mut settings, custom(150));
        assert!(settings.pending_qt.as_ref().unwrap().terminal);
        *backend.value.borrow_mut() = "1.250000".into();
        let report = run(&backend, &mut settings, custom(150));
        assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Unchanged);
        assert!(settings.pending_qt.is_none());
        assert_eq!(backend.writes.get(), 1);
    }

    #[test]
    fn failed_intent_persistence_prevents_external_setter() {
        let backend = Backend::new();
        let mut settings = ApplicationScaleSettings::default();
        assert!(
            transact_application_scale(
                &backend,
                &mut settings,
                custom(150),
                |_| Err("disk failure".into()),
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(backend.writes.get(), 0);
    }
    #[test]
    fn definitely_unsubmitted_failure_clears_intent_but_revocation_forbids_cleanup() {
        struct Rejected {
            cancel: Cell<bool>,
            revoke: bool,
        }
        impl ToolkitScaleBackend for Rejected {
            fn capabilities(&self) -> Vec<ToolkitCapability> {
                Backend::new().capabilities()
            }
            fn read(&self, _: ToolkitFamily) -> Result<String, String> {
                Ok("1".into())
            }
            fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
                unreachable!()
            }
            fn write_checked(
                &self,
                _: ToolkitFamily,
                _: &str,
                _: &str,
            ) -> Result<(), ToolkitWriteError> {
                self.cancel.set(self.revoke);
                Err(ToolkitWriteError::NotAccepted(
                    ToolkitRejection::Unavailable,
                ))
            }
        }
        for revoke in [false, true] {
            let backend = Rejected {
                cancel: Cell::new(false),
                revoke,
            };
            let mut settings = ApplicationScaleSettings::default();
            let mut durable = Vec::new();
            let result = transact_application_scale(
                &backend,
                &mut settings,
                custom(150),
                |value| {
                    durable.push(value.clone());
                    Ok(())
                },
                || {
                    if backend.cancel.get() {
                        Err("revoked".into())
                    } else {
                        Ok(())
                    }
                },
            );
            if revoke {
                assert!(result.is_err());
                assert_eq!(durable.len(), 1);
                assert!(durable[0].pending_qt.is_some());
            } else {
                assert_eq!(
                    result.unwrap().outcomes[0].kind,
                    ToolkitOutcomeKind::Unavailable
                );
                assert!(durable.last().unwrap().pending_qt.is_none());
            }
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_preparation_rejects_target_changed_after_engine_read() {
        struct Racing {
            path: std::path::PathBuf,
        }
        impl ToolkitScaleBackend for Racing {
            fn capabilities(&self) -> Vec<ToolkitCapability> {
                Backend::new().capabilities()
            }
            fn read(&self, _: ToolkitFamily) -> Result<String, String> {
                Ok("1".into())
            }
            fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
                unreachable!()
            }
            fn write_checked(
                &self,
                family: ToolkitFamily,
                value: &str,
                expected: &str,
            ) -> Result<(), ToolkitWriteError> {
                std::fs::write(&self.path, b"[KScreen]\nScaleFactor=2\nOther=keep\n").unwrap();
                crate::toolkit_native_write::PreparedToolkitWrite::prepare_file(
                    self.path.clone(),
                    family,
                    expected.into(),
                    "KScreen",
                    "ScaleFactor",
                    Some(value),
                )?
                .accept(|| Ok(()))?
                .wait(|| Ok(()))
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let backend = Racing {
            path: temp.path().join("kdeglobals"),
        };
        std::fs::write(&backend.path, b"[KScreen]\nScaleFactor=1\n").unwrap();
        let mut settings = ApplicationScaleSettings {
            owned_qt_previous: Some("0.750000".into()),
            owned_qt_applied: Some("1.000000".into()),
            ..Default::default()
        };
        let mut durable = Vec::new();
        let report = transact_application_scale(
            &backend,
            &mut settings,
            custom(150),
            |value| {
                durable.push(value.clone());
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            report.outcomes[0].kind,
            ToolkitOutcomeKind::ExternalConflict
        );
        assert_eq!(
            std::fs::read(&backend.path).unwrap(),
            b"[KScreen]\nScaleFactor=2\nOther=keep\n"
        );
        assert_eq!(settings.owned_qt_previous.as_deref(), Some("0.750000"));
        assert_eq!(settings.owned_qt_applied.as_deref(), Some("1.000000"));
        assert!(durable.last().unwrap().pending_qt.is_none());
    }
    #[test]
    fn post_journal_read_failure_is_retryable_without_native_submission() {
        struct ReadFailure {
            reads: Cell<usize>,
        }
        impl ToolkitScaleBackend for ReadFailure {
            fn capabilities(&self) -> Vec<ToolkitCapability> {
                Backend::new().capabilities()
            }
            fn read(&self, _: ToolkitFamily) -> Result<String, String> {
                let n = self.reads.get();
                self.reads.set(n + 1);
                if n == 1 {
                    Err("read unavailable".into())
                } else {
                    Ok("1".into())
                }
            }
            fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
                panic!("setter must not start")
            }
        }
        let mut settings = ApplicationScaleSettings::default();
        let report = transact_application_scale(
            &ReadFailure {
                reads: Cell::new(0),
            },
            &mut settings,
            custom(150),
            |_| Ok(()),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Failed);
        assert!(settings.pending_qt.is_none());
    }
    #[test]
    fn accepted_write_with_lost_receipt_keeps_unknown_intent_without_retry() {
        struct Lost {
            value: RefCell<String>,
            writes: Cell<usize>,
        }
        impl ToolkitScaleBackend for Lost {
            fn capabilities(&self) -> Vec<ToolkitCapability> {
                Backend::new().capabilities()
            }
            fn read(&self, _: ToolkitFamily) -> Result<String, String> {
                Ok(self.value.borrow().clone())
            }
            fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
                unreachable!()
            }
            fn write_checked(
                &self,
                _: ToolkitFamily,
                value: &str,
                _: &str,
            ) -> Result<(), ToolkitWriteError> {
                self.writes.set(self.writes.get() + 1);
                *self.value.borrow_mut() = value.into();
                Err(ToolkitWriteError::Uncertain)
            }
        }
        let backend = Lost {
            value: RefCell::new("1".into()),
            writes: Cell::new(0),
        };
        let mut settings = ApplicationScaleSettings::default();
        for _ in 0..2 {
            let report = transact_application_scale(
                &backend,
                &mut settings,
                custom(150),
                |_| Ok(()),
                || Ok(()),
            )
            .unwrap();
            assert_eq!(report.outcomes[0].kind, ToolkitOutcomeKind::Uncertain);
            assert!(!settings.pending_qt.as_ref().unwrap().terminal);
        }
        assert_eq!(backend.writes.get(), 1);
        assert!(settings.owned_qt_previous.is_none());
    }
}
