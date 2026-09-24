use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::platform::NotificationSource;
use image::{Rgba, RgbaImage};
use nickel_input::KeyCode;
use nickel_session_protocol::{
    AnchorSide, PointerInteraction, PreviewTargetAction, ScreenshotTargetAction, ShellRole,
    ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::{
    ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
    HostTelemetry, InputModality, OverlayAnchor, Point, Rect, SemanticAction, SemanticRole,
    SemanticSelector, SemanticValueInput, SemanticValueSnapshot, Shortcut, UiEvent, UiHost,
    ViewContext,
};
use nickel_ui_testkit::{Scenario, Selector};

use super::{
    ControlAction, HostRuntimeSamples, LiveShell, desktop_label_foreground, initial_wallpaper,
    panel_status_layout, panel_tray_icons,
    platform::{
        AudioStatus, BluetoothStatus, FeedState, FeedStatus, GlobalShortcut, NetworkStatus,
        SecureStorageState, SystemStatusUpdate,
    },
    preview_refresh_due, retain_unchanged_desktop_icons, secure_storage_status_label,
    semantic_theme_from_palette, session_feed_status_label, shortcut_capability_status,
    visible_tray_item, window_belongs_to_panel,
};

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");
include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");

#[test]
fn pointer_opened_control_center_does_not_paint_initial_keyboard_focus() {
    let mut shell = LiveShell::new().expect("live shell");
    assert_eq!(
        shell.control_host.inspect().modality,
        InputModality::Keyboard
    );
    assert!(
        shell
            .panel_host
            .adopt_input_modality(InputModality::Pointer)
    );

    shell.apply_panel_action(super::PanelAction::Control);

    assert_eq!(
        shell.control_host.inspect().modality,
        InputModality::Pointer
    );
}

#[test]
fn codex_approval_notification_revises_in_place_and_retires_on_resolution() {
    use nickel_codex::{ApprovalContext, ServerRequestId, ThreadId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_session_launcher_visibility(true);
    shell.launcher_host.step(HostBatch {
        surface_size: Some((920, 680)),
        ..HostBatch::default()
    });
    let typing_focus = shell.launcher_host.inspect().keyboard_focus.clone();
    assert!(typing_focus.is_some());
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let snapshot = |root: &str| CodexApprovalNotification {
        connection_generation: 1,
        request_revision: if root == "/safe" { 1 } else { 2 },
        thread_id: Some(ThreadId("thread".into())),
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("request".into()),
            approval_type: "item/fileChange/requestApproval".into(),
            summary: "Write files".into(),
            context: ApprovalContext {
                grant_root: Some(root.into()),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Change files".into(),
            scope: Some(root.into()),
            duration: None,
            warning: None,
            detail: Some(format!(
                "Requested files under {root}; Bearer fixture-private-token"
            )),
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, snapshot("/safe"))]);
    shell.refresh_fast();
    assert_eq!(shell.launcher_host.inspect().keyboard_focus, typing_focus);
    let first = shell
        .notification_feed
        .snapshot()
        .expect("pending notification");
    assert_eq!(first.actions.len(), 3);
    assert!(first.body.contains("/safe"));
    assert!(first.body.contains("Review the full operation in Codex"));
    assert!(!first.body.contains("fixture-private-token"));
    assert!(!first.body.contains("Requested files under /safe"));

    shell.sync_codex_approval_notifications(vec![(owner, snapshot("/broader"))]);
    let revised = shell
        .notification_feed
        .snapshot()
        .expect("revised notification");
    assert_eq!(revised.id, first.id);
    assert!(revised.body.contains("/broader"));
    assert_eq!(shell.codex_approval_notifications.len(), 1);

    shell.notification = Some(revised.clone());
    shell.sync_notification_host(420, 180);
    let cancel = shell
        .notification_host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Cancel".into(),
        })
        .expect("cancel decision");
    shell
        .notification_host
        .perform_semantic_action(cancel.id, SemanticAction::Invoke(ActionKind::Activate));
    shell.apply_notification_effects();
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(
            owner,
            snapshot("/broader"),
            nickel_codex_ui::CodexApprovalChoice::Cancel,
        )]
    );
    assert!(
        shell
            .notification_feed
            .history()
            .into_iter()
            .find(|item| item.id == revised.id)
            .unwrap()
            .actions
            .is_empty(),
        "a submitted action must not be offered again"
    );
    shell.dismiss_notification_transport(revised.id);
    assert!(shell.notification.is_none());
    assert!(
        shell
            .dismissed_codex_approval_notifications
            .contains(&revised.id)
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == revised.id)
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .all(|item| !item.body.contains("fixture-private-token"))
    );
    shell.refresh_fast();
    assert!(
        shell.notification.is_none(),
        "dismissal must not re-toast a pending request"
    );

    shell.sync_codex_approval_notifications(Vec::new());
    assert!(shell.codex_approval_notifications.is_empty());
    assert!(shell.dismissed_codex_approval_notifications.is_empty());
    assert!(shell.notification_feed.snapshot().is_none());
}

#[test]
fn codex_notification_reviews_large_source_decision_set_without_truncating_it() {
    use nickel_codex::{ApprovalContext, CommandDecision, ServerRequestId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let snapshot = CodexApprovalNotification {
        connection_generation: 1,
        request_revision: 1,
        thread_id: None,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("choices".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run a command".into(),
            context: ApprovalContext {
                available_decisions: Some(vec![
                    CommandDecision::Accept,
                    CommandDecision::AcceptForSession,
                    CommandDecision::Decline,
                    CommandDecision::Cancel,
                ]),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Run a command".into(),
            scope: Some("/projects/nickel".into()),
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, snapshot)]);
    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.actions.len(), 1);
    assert_eq!(notification.actions[0].key, "review");
    shell.notification = Some(notification);
    shell.sync_notification_host(420, 180);
    let review = shell
        .notification_host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Review in Codex".into(),
        })
        .unwrap();
    shell
        .notification_host
        .perform_semantic_action(review.id, SemanticAction::Invoke(ActionKind::Activate));
    shell.apply_notification_effects();
    assert_eq!(shell.take_codex_approval_reviews(), vec![owner]);
    assert!(shell.take_codex_approval_decisions().is_empty());
}

#[test]
fn simultaneous_codex_and_remote_approvals_keep_distinct_request_owners() {
    use nickel_codex::{ApprovalContext, ServerRequestId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_session_protocol::{
        RemoteLeaseRequest, RemoteLeaseRequestChanges, RemotePendingLease, RemoteResourceScope,
    };
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let remote = RemotePendingLease {
        pending_generation: 7,
        client_id: "remote-client".into(),
        client_label: "Remote controller".into(),
        request: RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::FullSession,
            duration_seconds: Some(600),
            allow_resumption: false,
            full_debug: false,
        },
        resource_label: None,
        changes: RemoteLeaseRequestChanges::default(),
    };
    shell.sync_remote_lease_notifications_from(vec![remote.clone()]);
    let remote_id = *shell.remote_lease_notifications.keys().next().unwrap();

    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let surface = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(surface);
    let codex = |scope: &str| CodexApprovalNotification {
        connection_generation: 2,
        request_revision: if scope == "/first" { 1 } else { 2 },
        thread_id: None,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("codex-request".into()),
            approval_type: "item/fileChange/requestApproval".into(),
            summary: "Change files".into(),
            context: ApprovalContext {
                grant_root: Some(scope.into()),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Change files".into(),
            scope: Some(scope.into()),
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, codex("/first"))]);
    assert_eq!(shell.remote_lease_notifications.len(), 1);
    assert_eq!(shell.codex_approval_notifications.len(), 1);
    let codex_id = *shell.codex_approval_notifications.keys().next().unwrap();
    assert_ne!(remote_id, codex_id);

    // A revision of one source must not replace or retire the other source's
    // independently pending authority request.
    shell.sync_codex_approval_notifications(vec![(owner, codex("/revised"))]);
    assert!(shell.remote_lease_notifications.contains_key(&remote_id));
    assert!(shell.codex_approval_notifications.contains_key(&codex_id));
    assert_eq!(shell.notification_feed.history().len(), 2);
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == codex_id && item.body.contains("/revised"))
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == remote_id && item.body.contains("full desktop"))
    );
}

#[test]
fn codex_approval_feed_overflow_queues_one_explicit_decline() {
    use nickel_codex::{ApprovalContext, CommandDecision, ServerRequestId, ThreadId};
    use nickel_codex_ui::{CodexApprovalChoice, CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let fixture_ids = (0..crate::notification::MAX_NOTIFICATIONS)
        .map(|_| {
            shell
                .notification_feed
                .notify_internal(crate::notification::NotificationRequest {
                    app_name: "Persistent fixture".into(),
                    summary: "Fixture".into(),
                    body: String::new(),
                    actions: Vec::new(),
                    expire_timeout_ms: 0,
                })
        })
        .collect::<Vec<_>>();
    assert!(fixture_ids.iter().all(|id| *id != 0));
    let pending = CodexApprovalNotification {
        connection_generation: 1,
        request_revision: 1,
        thread_id: Some(ThreadId("thread".into())),
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("request".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext::default(),
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Run a command".into(),
            scope: None,
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, pending.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, pending.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(owner, pending.clone(), CodexApprovalChoice::Decline)]
    );
    let revised = CodexApprovalNotification {
        request_revision: 2,
        ..pending.clone()
    };
    shell.sync_codex_approval_notifications(vec![(owner, revised.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, revised.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(owner, revised, CodexApprovalChoice::Decline)]
    );

    let with_decisions = |id: &str, decisions| CodexApprovalNotification {
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId(id.into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext {
                available_decisions: Some(decisions),
                ..Default::default()
            },
        },
        ..pending.clone()
    };
    let no_refusal = with_decisions("no-refusal", vec![CommandDecision::AcceptForSession]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    assert!(shell.take_codex_approval_decisions().is_empty());
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, no_refusal.clone(), false)]
    );
    shell.notification_feed.close_internal(fixture_ids[0]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, no_refusal, true)]
    );
    shell.sync_codex_approval_notifications(Vec::new());
    assert_eq!(
        shell.notification_feed.history().len(),
        crate::notification::MAX_NOTIFICATIONS - 1
    );
    assert_ne!(
        shell
            .notification_feed
            .notify_internal(crate::notification::NotificationRequest {
                app_name: "Persistent fixture".into(),
                summary: "Fixture".into(),
                body: String::new(),
                actions: Vec::new(),
                expire_timeout_ms: 0,
            }),
        0
    );
    let cancel = with_decisions("cancel-only", vec![CommandDecision::Cancel]);
    shell.sync_codex_approval_notifications(vec![(owner, cancel.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(
            owner,
            cancel,
            CodexApprovalChoice::Command(CommandDecision::Cancel),
        )]
    );
    let inactive = CodexApprovalNotification {
        actionable: false,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("disconnected".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext::default(),
        },
        ..pending
    };
    shell.sync_codex_approval_notifications(vec![(owner, inactive.clone())]);
    assert!(shell.take_codex_approval_decisions().is_empty());
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, inactive, false)]
    );
}

#[test]
fn in_process_system_feed_propagates_audio_network_and_bluetooth() {
    let mut shell = LiveShell::new().expect("live shell");
    let network = NetworkStatus {
        available: true,
        enabled: true,
        connected: true,
        name: "Nickel Lab".into(),
        signal_percent: 82,
        networks: Vec::new(),
    };
    let bluetooth = BluetoothStatus {
        available: true,
        powered: true,
        discovering: false,
        devices: Vec::new(),
    };
    let audio = AudioStatus {
        available: true,
        devices: Vec::new(),
        volume_percent: 64,
        muted: false,
    };

    assert!(shell.apply_system_status_update(SystemStatusUpdate::Network(network.clone())));
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Bluetooth(bluetooth.clone())));
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Audio(audio.clone())));
    assert_eq!(shell.network, network);
    assert_eq!(shell.bluetooth, bluetooth);
    assert_eq!(shell.audio, audio);
}

#[test]
fn unchanged_system_feed_events_are_idle_and_do_not_schedule_polling() {
    let mut shell = LiveShell::new().expect("live shell");
    let status = shell.audio.clone();
    let before = shell.next_host_deadline();

    assert!(!shell.apply_system_status_update(SystemStatusUpdate::Audio(status)));
    assert_eq!(shell.next_host_deadline(), before);
}

#[test]
fn launcher_focus_loss_dismisses_the_ephemeral_surface() {
    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_session_launcher_visibility(true);

    assert!(shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::Launcher));
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
    assert!(!shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::Launcher));
}

#[test]
fn control_center_focus_loss_dismisses_the_ephemeral_surface() {
    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_control_visibility(true);

    assert!(shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
}

#[test]
fn rejected_launcher_focus_request_does_not_project_internal_focus() {
    struct RejectingHost;

    impl crate::session_host::SessionHost for RejectingHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Err(crate::platform::SessionRequestError::Send)
        }
    }

    let mut shell = LiveShell::new_with_session_host(Arc::new(RejectingHost)).expect("live shell");

    assert!(!shell.request_launcher_toggle());
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
    assert!(shell.launcher_host.inspect().keyboard_focus.is_none());
}

#[test]
fn successful_launcher_retry_clears_transient_update_error() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecoveringHost {
        reject: AtomicBool,
    }

    impl crate::session_host::SessionHost for RecoveringHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            if self.reject.swap(false, Ordering::AcqRel) {
                Err(crate::platform::SessionRequestError::Send)
            } else {
                Ok(())
            }
        }
    }

    let mut shell = LiveShell::new_with_session_host(Arc::new(RecoveringHost {
        reject: AtomicBool::new(true),
    }))
    .expect("live shell");

    assert!(!shell.request_launcher_toggle());
    assert_eq!(
        shell.launcher_status.as_deref(),
        Some("Nickel could not update the launcher.")
    );

    assert!(shell.request_launcher_toggle());
    assert!(shell.launcher_status.is_none());
    assert!(shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
}

#[test]
fn pending_remote_lease_becomes_persistent_shell_notification() {
    use nickel_session_protocol::{
        RemoteLeaseRequest, RemoteLeaseRequestChanges, RemotePendingLease, RemoteResourceScope,
    };
    use std::sync::Mutex;

    struct PendingLeaseHost {
        pending: Mutex<Vec<RemotePendingLease>>,
        #[cfg(target_os = "linux")]
        decisions: Mutex<Vec<bool>>,
    }
    impl crate::session_host::SessionHost for PendingLeaseHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
        fn remote_pending_leases(&self) -> Vec<RemotePendingLease> {
            self.pending.lock().unwrap().clone()
        }
        fn decide_remote_lease(
            &self,
            _: &RemotePendingLease,
            allow: bool,
        ) -> Result<(), crate::platform::SessionRequestError> {
            #[cfg(target_os = "linux")]
            self.decisions.lock().unwrap().push(allow);
            #[cfg(not(target_os = "linux"))]
            let _ = allow;
            Ok(())
        }
    }

    let pending = RemotePendingLease {
        pending_generation: 4,
        client_id: "agent-1".into(),
        client_label: "Codex".into(),
        request: RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::FullSession,
            duration_seconds: Some(1_200),
            allow_resumption: false,
            full_debug: true,
        },
        resource_label: None,
        changes: RemoteLeaseRequestChanges::default(),
    };
    let host = Arc::new(PendingLeaseHost {
        pending: Mutex::new(vec![pending.clone()]),
        #[cfg(target_os = "linux")]
        decisions: Mutex::new(Vec::new()),
    });
    let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();

    shell.sync_remote_lease_notifications_from(host.pending.lock().unwrap().clone());

    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.summary, "Codex requests approval");
    for expected in [
        "Scope: the full desktop",
        "Duration: 20 minutes",
        "Requester label is self-reported",
        "Full Nickel debugging access is requested",
        "pointer and keyboard input",
        "Protected surfaces and clipboard or filesystem transfer are excluded",
    ] {
        assert!(notification.body.contains(expected), "{expected}");
    }
    assert_eq!(notification.actions[0].key, "deny");
    assert_eq!(notification.actions[1].key, "approve");
    assert_eq!(shell.remote_lease_notifications.len(), 1);

    let notification_id = notification.id;
    shell.notification = Some(notification);
    shell.sync_notification_host(420, 180);
    let approve = shell
        .notification_host
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Approve".into(),
        })
        .unwrap();
    let point = nickel_input::Point {
        x: f64::from(approve.bounds.origin.x + approve.bounds.size.width / 2.0),
        y: f64::from(approve.bounds.origin.y + approve.bounds.size.height / 2.0),
    };
    let event = |edge| {
        nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            device: nickel_input::DeviceId(1),
            order: nickel_input::EventOrder(1),
            position: Some(point),
            button: nickel_input::PointerButton::Primary,
            edge,
        })
    };
    shell.notification_host_input(event(nickel_input::KeyEdge::Pressed), 420, 180);
    shell.notification_host_input(event(nickel_input::KeyEdge::Released), 420, 180);
    #[cfg(target_os = "linux")]
    assert_eq!(*host.decisions.lock().unwrap(), vec![true]);
    #[cfg(target_os = "windows")]
    assert_eq!(
        shell.take_remote_lease_decisions(),
        vec![(pending.clone(), true)]
    );
    assert_eq!(
        shell.remote_lease_notifications.len(),
        1,
        "dispatch is not resolution"
    );
    assert!(shell.remote_lease_submitting.contains(&notification_id));
    let submitted = shell
        .notification_feed
        .history()
        .into_iter()
        .find(|item| item.id == notification_id)
        .unwrap();
    assert!(submitted.actions.is_empty());
    assert!(submitted.body.contains("unconfirmed"));
    shell.dismiss_notification_transport(notification_id);
    assert!(shell.notification.is_none());
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == notification_id)
    );
    shell.refresh_fast();
    assert!(
        shell.notification.is_none(),
        "dismissed pending request must not re-toast"
    );

    host.pending.lock().unwrap().clear();
    shell.sync_remote_lease_notifications_from(Vec::new());
    assert!(shell.remote_lease_notifications.is_empty());
    assert!(shell.remote_lease_submitting.is_empty());
    assert!(shell.dismissed_remote_lease_notifications.is_empty());
    assert!(shell.notification_feed.snapshot().is_none());

    let mut changed = pending.clone();
    changed.pending_generation = 5;
    changed.request.allow_resumption = true;
    changed.request.renewal = Some(nickel_session_protocol::RemoteLeaseRenewal {
        lease_id: 2,
        generation: 1,
    });
    changed.changes.access_changed = true;
    changed.changes.duration_increased = true;
    shell.sync_remote_lease_notifications_from(vec![changed]);
    let changed_body = shell.notification_feed.snapshot().unwrap().body;
    for warning in [
        "may resume after the client reconnects",
        "broadens access",
        "increases the duration",
        "renews an existing lease",
    ] {
        assert!(changed_body.contains(warning), "{warning}");
    }
    shell.sync_remote_lease_notifications_from(Vec::new());

    let mut oversized = pending;
    oversized.pending_generation = 5;
    oversized.client_label = "x".repeat(nickel_ui::approval::MAX_APPROVAL_PRESENTATION_BYTES);
    let mut overflow = oversized.clone();
    overflow.pending_generation = 6;
    shell.sync_remote_lease_notifications_from(vec![oversized]);
    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.actions.len(), 1);
    assert_eq!(notification.actions[0].key, "deny");
    assert!(notification.body.contains("Approval is unavailable"));

    shell.sync_remote_lease_notifications_from(Vec::new());
    for _ in 0..crate::notification::MAX_NOTIFICATIONS {
        assert_ne!(
            shell
                .notification_feed
                .notify_internal(crate::notification::NotificationRequest {
                    app_name: "Persistent fixture".into(),
                    summary: "Fixture".into(),
                    body: String::new(),
                    actions: Vec::new(),
                    expire_timeout_ms: 0,
                }),
            0
        );
    }
    shell.sync_remote_lease_notifications_from(vec![overflow.clone()]);
    shell.sync_remote_lease_notifications_from(vec![overflow]);
    #[cfg(target_os = "linux")]
    assert_eq!(*host.decisions.lock().unwrap(), vec![true, false]);
    #[cfg(target_os = "windows")]
    assert_eq!(shell.take_remote_lease_decisions().len(), 1);
    assert!(shell.remote_lease_notifications.is_empty());
    assert_eq!(shell.remote_lease_overflow_rejections.len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn queued_keyboard_auto_show_does_not_recursively_query_the_unacknowledged_snapshot() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct PendingKeyboardHost(AtomicUsize);
    impl crate::session_host::SessionHost for PendingKeyboardHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
        fn keyboard_snapshot(
            &self,
        ) -> Result<
            nickel_session_protocol::OnScreenKeyboardSnapshot,
            crate::platform::SessionRequestError,
        > {
            Ok(nickel_session_protocol::OnScreenKeyboardSnapshot {
                enabled: true,
                auto_show_requested: true,
                epoch: 19,
                generation: 1,
                height: 320,
                recipient: Some(nickel_session_protocol::WindowId(7)),
                ..Default::default()
            })
        }
        fn configure_keyboard(
            &self,
            _: bool,
            _: bool,
            _: u64,
            _: bool,
            _: bool,
            _: u32,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    let host = Arc::new(PendingKeyboardHost(AtomicUsize::new(0)));
    let mut shell = LiveShell::new().unwrap();
    shell.session_host = host.clone();
    shell.keyboard_override = nickel_core::on_screen_keyboard::KeyboardOverride::Enabled;
    assert!(shell.refresh_keyboard());
    assert!(shell.keyboard_enabled);
    assert!(shell.keyboard_visible);
    assert!(
        host.0.load(Ordering::SeqCst) <= 2,
        "no recursive auto-show requests before authority acknowledgement"
    );
}

#[test]
fn coalesced_audio_feedback_uses_latest_state_and_suppresses_reconnect_only_changes() {
    let mut shell = LiveShell::new().unwrap();
    let status = |available, volume_percent, muted| AudioStatus {
        available,
        volume_percent,
        muted,
        devices: Vec::new(),
    };
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status(true, 31, false)));
    let (sender, receiver) = crate::platform::status_mailbox::channel();
    for snapshot in [status(true, 36, false), status(true, 31, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    let updates = receiver.drain();
    assert_eq!(updates.len(), 1);
    assert!(shell.apply_system_status_update(updates.into_iter().next().unwrap()));
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    shell.volume_osd_scene(320, 88);
    assert!(
        shell
            .volume_osd_host
            .application()
            .label
            .starts_with("Volume 31%")
    );
    // A hidden unavailable/available transition must not turn a different device's
    // initial volume into apparent user feedback.
    for snapshot in [status(false, 0, false), status(true, 50, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    for update in receiver.drain() {
        shell.apply_system_status_update(update);
    }
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    for snapshot in [status(true, 50, true), status(true, 50, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    for update in receiver.drain() {
        assert!(shell.apply_system_status_update(update));
    }
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    assert!(!shell.audio.muted);
}

#[test]
fn native_audio_feedback_ignores_startup_metadata_and_reconnect_but_shows_value_changes() {
    let mut shell = LiveShell::new().unwrap();
    let mut status = AudioStatus {
        available: true,
        volume_percent: 31,
        muted: false,
        devices: Vec::new(),
    };
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.devices.push(crate::platform::AudioDeviceStatus {
        id: "sink".into(),
        name: "Speaker".into(),
        is_default: true,
    });
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.volume_percent = 36;
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone())));
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    shell.volume_osd_scene(320, 88);
    assert!(
        shell
            .volume_osd_host
            .application()
            .label
            .starts_with("Volume 36%")
    );
    let first = shell.volume_osd_until.unwrap();
    status.muted = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(shell.volume_osd_until.unwrap() >= first);
    let outcome = shell.poll_deadlines(Instant::now() + Duration::from_secs(2));
    assert!(outcome.visibility_changed);
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.available = false;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    status.available = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
}

#[test]
fn settings_transition_reprojects_light_and_dark_appearance() {
    use nickel_core::{shell_settings::ThemePreference, theme::ThemePalette};

    let mut shell = LiveShell::new().expect("live shell");
    let mut settings = nickel_core::shell_settings::ShellSettings {
        theme: ThemePreference::Light,
        ..Default::default()
    };
    assert!(shell.apply_shell_settings(settings.clone()));
    let light = shell.palette;
    assert_eq!(
        light,
        ThemePalette::from_appearance(settings.resolve_appearance(Default::default()))
    );

    settings.theme = ThemePreference::Dark;
    assert!(shell.apply_shell_settings(settings.clone()));
    assert_ne!(shell.palette, light);
    assert_eq!(
        shell.palette,
        ThemePalette::from_appearance(settings.resolve_appearance(Default::default()))
    );
}

#[test]
fn injected_session_host_receives_shell_commands_without_platform_transport() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingHost(AtomicUsize);

    impl crate::session_host::SessionHost for RecordingHost {
        fn dispatch(
            &self,
            _command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<SecureStorageState, crate::platform::SessionRequestError> {
            Ok(SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
    }

    let host = Arc::new(RecordingHost(AtomicUsize::new(0)));
    let shell = LiveShell::new_with_session_host(host.clone()).expect("live shell");

    assert!(shell.dispatch_session_command(
        "test-direct-session-host",
        crate::platform::ShellCommand::CreateWorkspace,
    ));
    assert_eq!(host.0.load(Ordering::Relaxed), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn active_window_screenshot_uses_in_process_capture_and_crops_before_copy() {
    use std::sync::Mutex;

    use crate::session_host::DesktopCapturePoll;
    use nickel_session_protocol::{
        Geometry, OutputSnapshot, OutputTransform, Snapshot, WindowId, WindowSnapshot, WorkspaceId,
    };

    #[derive(Default)]
    struct CaptureHost {
        outputs: Mutex<Vec<Option<String>>>,
        copied_sizes: Mutex<Vec<(u32, u32)>>,
    }

    impl crate::session_host::SessionHost for CaptureHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }

        fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
            self.outputs.lock().unwrap().push(output.map(str::to_owned));
            DesktopCapturePoll::Ready(Ok(crate::platform::DesktopCapture {
                image: RgbaImage::new(200, 100),
            }))
        }

        fn copy_image(&self, image: RgbaImage) -> Result<(), String> {
            self.copied_sizes.lock().unwrap().push(image.dimensions());
            Ok(())
        }
    }

    let host = Arc::new(CaptureHost::default());
    let mut shell = LiveShell::new_with_session_host(host.clone()).expect("live shell");
    shell.apply_internal_session_snapshot(Snapshot {
        outputs: vec![OutputSnapshot {
            name: "DP-1".into(),
            model: "Test".into(),
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            work_area: Geometry {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_120: 120,
            transform: OutputTransform::Normal,
            physical_width_mm: 1,
            physical_height_mm: 1,
            primary: true,
            enabled: true,
            modes: Vec::new(),
            current_mode: None,
        }],
        windows: vec![WindowSnapshot {
            id: WindowId(7),
            application_id: "test.app".into(),
            title: "Test".into(),
            active: true,
            minimized: false,
            maximized: false,
            fullscreen: false,
            geometry: Some(Geometry {
                x: 10,
                y: 5,
                width: 30,
                height: 20,
            }),
            workspace: WorkspaceId(1),
        }],
        focused: Some(WindowId(7)),
        ..Snapshot::default()
    });

    assert!(shell.global_shortcut(GlobalShortcut::Screenshot(
        crate::platform::ScreenshotAction::ActiveWindow
    )));
    assert!(!shell.capture_screenshot());
    assert_eq!(
        host.outputs.lock().unwrap().as_slice(),
        &[Some("DP-1".into())]
    );
    assert_eq!(host.copied_sizes.lock().unwrap().as_slice(), &[(60, 40)]);
    assert!(!shell.screenshot.visible());
}

#[test]
fn compositor_owned_shell_scenario_routes_focus_switching_and_files_without_transport() {
    use std::{path::PathBuf, sync::Mutex};

    use nickel_core::{
        hotkeys::HotkeyAction,
        task_switcher::{SwitchWindow, TaskSwitcher},
    };
    use nickel_file::{FileLaunch, FileWindowRequest};

    #[derive(Default)]
    struct RecordingHost(Mutex<Vec<crate::platform::ShellCommand>>);

    impl crate::session_host::SessionHost for RecordingHost {
        fn dispatch(
            &self,
            command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.lock().unwrap().push(command);
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, crate::platform::SessionRequestError>
        {
            Ok(crate::platform::SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingFiles(Mutex<Vec<FileWindowRequest>>);

    impl crate::file_window_host::FileWindowHost for RecordingFiles {
        fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
            self.0.lock().unwrap().push(request);
            Ok(())
        }
    }

    let session = Arc::new(RecordingHost::default());
    let files = Arc::new(RecordingFiles::default());
    let mut shell = LiveShell::new_with_hosts(session.clone(), files.clone()).expect("live shell");

    shell.apply_session_launcher_visibility(true);
    shell.launcher_host.step(HostBatch {
        surface_size: Some((920, 680)),
        ..HostBatch::default()
    });
    assert!(shell.launcher_host.inspect().keyboard_focus.is_some());

    shell.windows = vec![
        OpenWindow {
            id: WindowId(10),
            application_id: Some(ApplicationId::new("org.nickel.One")),
            active: true,
            title: "One".into(),
            state: crate::model::WindowState::default(),
        },
        OpenWindow {
            id: WindowId(20),
            application_id: Some(ApplicationId::new("org.nickel.Two")),
            active: false,
            title: "Two".into(),
            state: crate::model::WindowState::default(),
        },
    ];
    let switch_windows = shell
        .windows
        .iter()
        .map(|window| SwitchWindow {
            id: window.id,
            application_id: window.application_id.as_ref().unwrap().as_str().to_owned(),
            active: window.active,
        })
        .collect::<Vec<_>>();
    shell.task_switcher = TaskSwitcher::default();
    assert!(
        !shell
            .task_switcher
            .apply(HotkeyAction::SwitchNext, &switch_windows)
            .is_empty()
    );
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::SwitchNext));
    let preview_role = crate::winit_shell::SurfaceRole::WindowPreview;
    let _ = shell.scene(preview_role, 640, 240);
    let first_preview_token = shell
        .scene_change_token(preview_role)
        .expect("task switcher preview token");
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::SwitchNext));
    assert!(
        shell.preview_frame.is_some(),
        "consecutive switch steps must retain the preview host so its presentation token advances"
    );
    let _ = shell.scene(preview_role, 640, 240);
    assert_ne!(
        shell.scene_change_token(preview_role),
        Some(first_preview_token),
        "each visible task-switch selection must receive a distinct presentation token"
    );
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::CommitSwitch));
    assert!(session.0.lock().unwrap().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));

    // The Windows shortcut adapter forwards cancellation through this same
    // production shell action without committing the highlighted candidate.
    shell.task_switcher = TaskSwitcher::default();
    shell
        .task_switcher
        .apply(HotkeyAction::SwitchNext, &switch_windows);
    session.0.lock().unwrap().clear();
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::CancelSwitch));
    assert!(shell.task_switcher.session().is_none());
    assert!(!session.0.lock().unwrap().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));

    // Session actions share this platform-neutral shell path on Windows and
    // Linux. They must retire a pending switch (and its delayed peek) before
    // the platform begins locking, logging out, suspending, or restarting.
    shell.task_switcher = TaskSwitcher::default();
    shell
        .task_switcher
        .apply(HotkeyAction::SwitchNext, &switch_windows);
    session.0.lock().unwrap().clear();
    shell.apply_control_action(ControlAction::SessionAction(
        crate::platform::SessionAction::Lock,
    ));
    assert!(shell.task_switcher.session().is_none());
    let commands = session.0.lock().unwrap();
    assert!(commands.iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::SessionAction(crate::platform::SessionAction::Lock)
    )));
    assert!(!commands.iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));
    drop(commands);

    let path = PathBuf::from("/tmp/internal-file-scenario");
    shell.launch_application(crate::model::Application::new(
        "place:test".into(),
        "Test location".into(),
        None,
        None,
        Some(vec!["nickel-file".into(), path.display().to_string()]),
    ));
    assert_eq!(
        files.0.lock().unwrap().as_slice(),
        [FileWindowRequest::OpenOrFocus(FileLaunch::Browse(path))]
    );
}
