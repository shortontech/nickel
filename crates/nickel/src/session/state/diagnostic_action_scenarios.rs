use super::{PREVIEW_SESSION_TEST_LOCK, internal_shell_test_session};
use crate::session::state::{
    NickelSession, PreparedApplicationDiscovery, PreparedPlatformRefresh,
    PreparedPlatformRefreshData, RemoteDesktopRequest,
};
use nickel_remote_control::{
    ControlPlane, DesktopPermit, IssuedCapability,
    diagnostics::{DiagnosticAction, DiagnosticActionOutcome, PlatformRefreshDomain},
    leases::{ResourceId, ResourceScope},
};
use smithay::reexports::calloop::EventLoop;
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum PreparationAge {
    Fresh,
    Late,
}

struct DiagnosticOwnerScenario {
    _event_loop: EventLoop<'static, NickelSession>,
    session: NickelSession,
    control: Arc<Mutex<ControlPlane>>,
    identity: IssuedCapability,
    lease: u64,
}

impl DiagnosticOwnerScenario {
    fn new() -> Self {
        let (event_loop, mut session) = internal_shell_test_session();
        session.refresh_remote_output_identities();
        let control = session.remote_control.control();
        let now = Instant::now();
        let (identity, lease) = {
            let mut owner = control.lock().unwrap();
            owner.set_enabled(true);
            let identity = owner.connect_identity("diagnostic owner scenario").unwrap();
            let watch = owner
                .reserve_connection_watch(&identity.client_id, &identity.token, now)
                .unwrap();
            owner
                .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
                .unwrap();
            let lease = owner
                .leases_mut()
                .approve_local(
                    identity.client_id.clone(),
                    ResourceScope::FullSession,
                    now,
                    Some(now + Duration::from_secs(1200)),
                    false,
                    true,
                )
                .unwrap();
            (identity, lease)
        };
        Self {
            _event_loop: event_loop,
            session,
            control,
            identity,
            lease,
        }
    }

    fn permit(&self) -> DesktopPermit {
        DesktopPermit::from_active_lease(
            self.control.clone(),
            self.identity.client_id.clone(),
            self.identity.token.clone(),
            self.lease,
        )
        .unwrap()
    }

    fn output(&self) -> ResourceId {
        self.session
            .remote_output_identity("file-test".into())
            .unwrap()
    }

    fn actions(&self) -> Vec<DiagnosticAction> {
        let mut actions = vec![
            DiagnosticAction::Repaint,
            DiagnosticAction::RefreshScene,
            DiagnosticAction::RefreshApplicationInventory,
        ];
        actions.extend(
            platform_domains().map(|domain| DiagnosticAction::RefreshPlatformStatus { domain }),
        );
        actions.extend([
            DiagnosticAction::StartFrameTrace {
                duration_seconds: 1,
            },
            DiagnosticAction::StopFrameTrace,
            DiagnosticAction::IdentifyOutput {
                output: self.output(),
            },
        ]);
        for action in &actions {
            assert_matrix_variant_is_named(action);
        }
        actions
    }

    fn dispatch(
        &mut self,
        action: DiagnosticAction,
        age: PreparationAge,
    ) -> Result<DiagnosticActionOutcome, String> {
        let observed = match age {
            PreparationAge::Fresh => Instant::now(),
            PreparationAge::Late => Instant::now() - Duration::from_secs(3),
        };
        let observation_started = observed - Duration::from_millis(5);
        let application_discovery = matches!(action, DiagnosticAction::RefreshApplicationInventory)
            .then(|| PreparedApplicationDiscovery {
                discovery: crate::model::ApplicationDiscovery::ready(Vec::new()),
                observation_started,
                observed,
                preparation_duration_us: 5_000,
            });
        let platform_refresh = match action {
            DiagnosticAction::RefreshPlatformStatus { domain } => {
                Some(prepared_platform(domain, observation_started, observed))
            }
            _ => None,
        };
        let (reply, result) = mpsc::sync_channel(1);
        self.session
            .handle_remote_desktop_request(RemoteDesktopRequest::DiagnosticAction {
                permit: self.permit(),
                action,
                application_discovery,
                platform_refresh,
                reply,
            });
        result.recv_timeout(Duration::from_secs(1)).unwrap()
    }

    fn revoke(&self) {
        self.control.lock().unwrap().leases_mut().revoke(self.lease);
    }
}

fn assert_matrix_variant_is_named(action: &DiagnosticAction) {
    match action {
        DiagnosticAction::Repaint
        | DiagnosticAction::RefreshScene
        | DiagnosticAction::RefreshApplicationInventory
        | DiagnosticAction::RefreshPlatformStatus { .. }
        | DiagnosticAction::StartFrameTrace { .. }
        | DiagnosticAction::StopFrameTrace
        | DiagnosticAction::IdentifyOutput { .. } => {}
    }
}

fn platform_domains() -> impl Iterator<Item = PlatformRefreshDomain> {
    [
        PlatformRefreshDomain::Connectivity,
        PlatformRefreshDomain::Audio,
        PlatformRefreshDomain::Peripherals,
        PlatformRefreshDomain::Maintenance,
        PlatformRefreshDomain::DefaultAssociations,
    ]
    .into_iter()
}

fn prepared_platform(
    domain: PlatformRefreshDomain,
    observation_started: Instant,
    observed: Instant,
) -> PreparedPlatformRefresh {
    let data = match domain {
        PlatformRefreshDomain::Connectivity => {
            PreparedPlatformRefreshData::Connectivity(crate::platform::ConnectivityRefresh {
                network: crate::platform::NetworkStatus {
                    available: true,
                    ..Default::default()
                },
                bluetooth: crate::platform::BluetoothStatus {
                    available: true,
                    ..Default::default()
                },
                partial: true,
            })
        }
        PlatformRefreshDomain::Audio => {
            PreparedPlatformRefreshData::Audio(crate::platform::AudioRefresh {
                audio: crate::platform::AudioStatus {
                    available: true,
                    volume_percent: 37,
                    ..Default::default()
                },
                partial: false,
            })
        }
        PlatformRefreshDomain::Peripherals => {
            PreparedPlatformRefreshData::Peripherals(crate::platform::PeripheralRefresh {
                printers_available: true,
                volumes_available: true,
                filesystems_available: true,
                printer_count: 2,
                volume_count: 3,
                filesystem_count: 4,
                partial: true,
            })
        }
        PlatformRefreshDomain::Maintenance => {
            PreparedPlatformRefreshData::Maintenance(crate::platform::MaintenanceRefresh {
                maintenance_available: true,
                updates_available: Some(5),
                restart_required: Some(true),
                firewall_healthy: Some(true),
                malware_protection_healthy: Some(false),
                known_permission_states: 6,
                secure_storage_status_available: true,
                partial: true,
            })
        }
        PlatformRefreshDomain::DefaultAssociations => {
            PreparedPlatformRefreshData::DefaultAssociations(
                crate::platform::DefaultAssociationsRefresh {
                    associations_available: true,
                    targets_queried: 7,
                    effective_associations: 6,
                    directly_writable_associations: 2,
                    partial: true,
                },
            )
        }
    };
    PreparedPlatformRefresh {
        domain,
        data,
        observation_started,
        observed,
        preparation_duration_us: 5_000,
    }
}

#[test]
fn fresh_prepared_results_reconcile_every_platform_domain() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();

    for (expected_generation, domain) in (1_u64..).zip(platform_domains()) {
        let outcome = scenario
            .dispatch(
                DiagnosticAction::RefreshPlatformStatus { domain },
                PreparationAge::Fresh,
            )
            .unwrap();
        let refresh = outcome.platform_refresh.unwrap();
        assert_eq!(refresh.domain, domain);
        assert_eq!(refresh.generation, expected_generation);
        assert!(!refresh.stale);
        assert!(!outcome.presentation_confirmed);
        match domain {
            PlatformRefreshDomain::Connectivity => {
                assert!(refresh.network_available);
                assert!(refresh.bluetooth_available);
                assert!(refresh.reconciliation_confirmed);
            }
            PlatformRefreshDomain::Audio => {
                assert!(refresh.audio_available);
                assert!(refresh.reconciliation_confirmed);
            }
            PlatformRefreshDomain::Peripherals => {
                assert_eq!(
                    (
                        refresh.printer_count,
                        refresh.volume_count,
                        refresh.filesystem_count
                    ),
                    (2, 3, 4)
                );
                assert!(!refresh.reconciliation_confirmed);
            }
            PlatformRefreshDomain::Maintenance => {
                assert_eq!(refresh.updates_available, Some(5));
                assert_eq!(refresh.restart_required, Some(true));
                assert_eq!(refresh.known_permission_states, 6);
                assert!(!refresh.reconciliation_confirmed);
            }
            PlatformRefreshDomain::DefaultAssociations => {
                assert_eq!(refresh.association_targets_queried, 7);
                assert_eq!(refresh.effective_associations, 6);
                assert_eq!(refresh.directly_writable_associations, 2);
                assert!(!refresh.reconciliation_confirmed);
            }
        }
    }
    assert_eq!(scenario.session.remote_platform_refreshes.len(), 5);
}

#[test]
fn production_owner_exercises_every_non_native_action_variant() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();

    for action in [DiagnosticAction::Repaint, DiagnosticAction::RefreshScene] {
        let outcome = scenario.dispatch(action, PreparationAge::Fresh).unwrap();
        assert!(!outcome.presentation_confirmed);
    }

    let application = scenario
        .dispatch(
            DiagnosticAction::RefreshApplicationInventory,
            PreparationAge::Fresh,
        )
        .unwrap()
        .application_inventory_refresh
        .unwrap();
    assert_eq!(application.generation, 1);
    assert_eq!(application.applications, 0);
    assert!(application.reconciliation_confirmed);

    let stopped = scenario
        .dispatch(DiagnosticAction::StopFrameTrace, PreparationAge::Fresh)
        .unwrap();
    assert!(!stopped.presentation_confirmed);

    let output = scenario.output();
    let identified = scenario
        .dispatch(
            DiagnosticAction::IdentifyOutput { output },
            PreparationAge::Fresh,
        )
        .unwrap()
        .output_identification
        .unwrap();
    assert_eq!(identified.output.id, "file-test");
    assert_eq!(identified.duration_ms, 3_000);

    // This owner has no initialized native renderer. Accepting trace start here
    // would manufacture backend evidence; live nested acceptance owns success.
    assert_eq!(
        scenario
            .dispatch(
                DiagnosticAction::StartFrameTrace {
                    duration_seconds: 1,
                },
                PreparationAge::Fresh,
            )
            .unwrap_err(),
        "frame trace backend is unavailable"
    );
}

#[test]
fn shared_staging_rejects_every_refresh_kind_while_busy() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let scenario = DiagnosticOwnerScenario::new();
    let staging = scenario.session.remote_diagnostic_staging.clone();
    let _delayed = staging.acquire().unwrap();
    let mut actions = vec![DiagnosticAction::RefreshApplicationInventory];
    actions.extend(
        platform_domains().map(|domain| DiagnosticAction::RefreshPlatformStatus { domain }),
    );

    for action in actions {
        let error = scenario
            .session
            .remote_desktop_authority
            .diagnostic_action(scenario.permit(), action)
            .unwrap_err();
        assert_eq!(error, "background preparation is busy or unavailable");
    }
    assert_eq!(scenario.session.remote_application_inventory_generation, 0);
    assert_eq!(scenario.session.remote_platform_refresh_generation, 0);
}

#[test]
fn revoked_authority_rejects_every_action_before_owner_mutation() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();
    let actions = scenario.actions();
    let permits = actions
        .iter()
        .map(|_| scenario.permit())
        .collect::<Vec<_>>();
    scenario.revoke();

    for (action, permit) in actions.into_iter().zip(permits) {
        let observed = Instant::now();
        let application_discovery = matches!(action, DiagnosticAction::RefreshApplicationInventory)
            .then(|| PreparedApplicationDiscovery {
                discovery: crate::model::ApplicationDiscovery::ready(Vec::new()),
                observation_started: observed,
                observed,
                preparation_duration_us: 0,
            });
        let platform_refresh = match action {
            DiagnosticAction::RefreshPlatformStatus { domain } => {
                Some(prepared_platform(domain, observed, observed))
            }
            _ => None,
        };
        let generation = scenario.session.remote_observation_generation;
        let (reply, result) = mpsc::sync_channel(1);
        scenario
            .session
            .handle_remote_desktop_request(RemoteDesktopRequest::DiagnosticAction {
                permit,
                action,
                application_discovery,
                platform_refresh,
                reply,
            });
        assert!(
            result
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        assert_eq!(scenario.session.remote_observation_generation, generation);
        assert_eq!(scenario.session.remote_application_inventory_generation, 0);
        assert_eq!(scenario.session.remote_platform_refresh_generation, 0);
        assert!(scenario.session.remote_output_identification.is_none());
        assert!(scenario.session.remote_frame_trace.is_none());
    }
}

#[test]
fn expired_request_deadline_rejects_owner_commit() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();
    let permit = scenario.permit();
    std::thread::sleep(Duration::from_millis(2_050));
    let generation = scenario.session.remote_observation_generation;
    let (reply, result) = mpsc::sync_channel(1);
    scenario
        .session
        .handle_remote_desktop_request(RemoteDesktopRequest::DiagnosticAction {
            permit,
            action: DiagnosticAction::Repaint,
            application_discovery: None,
            platform_refresh: None,
            reply,
        });

    assert!(
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_err()
    );
    assert_eq!(scenario.session.remote_observation_generation, generation);
}

#[test]
fn late_prepared_results_cannot_replace_application_or_platform_state() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();

    let application_error = scenario
        .dispatch(
            DiagnosticAction::RefreshApplicationInventory,
            PreparationAge::Late,
        )
        .unwrap_err();
    assert_eq!(
        application_error,
        "diagnostic preparation expired before owner commit"
    );
    assert_eq!(scenario.session.remote_application_inventory_generation, 0);

    for domain in platform_domains() {
        let error = scenario
            .dispatch(
                DiagnosticAction::RefreshPlatformStatus { domain },
                PreparationAge::Late,
            )
            .unwrap_err();
        assert_eq!(error, "diagnostic preparation expired before owner commit");
    }
    assert_eq!(scenario.session.remote_platform_refresh_generation, 0);
    assert!(scenario.session.remote_platform_refreshes.is_empty());
}

#[test]
fn protected_state_rejects_every_action_variant() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut scenario = DiagnosticOwnerScenario::new();
    scenario.session.locked = true;

    for action in scenario.actions() {
        let generation = scenario.session.remote_observation_generation;
        assert!(scenario.dispatch(action, PreparationAge::Fresh).is_err());
        assert_eq!(scenario.session.remote_observation_generation, generation);
    }
    assert_eq!(scenario.session.remote_application_inventory_generation, 0);
    assert_eq!(scenario.session.remote_platform_refresh_generation, 0);
    assert!(scenario.session.remote_output_identification.is_none());
    assert!(scenario.session.remote_frame_trace.is_none());
}
