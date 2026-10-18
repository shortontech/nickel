use std::time::{Duration, Instant};

use nickel_codex::{ReplayBackend, Thread, ThreadId};
use twinkle_testkit::Scenario;
use twinkle_ui::{HostBatch, HostEvent, Rect, UiFrame, UiHost};

use super::*;

#[test]
fn resumed_medium_transcript_has_no_estimated_blank_spacer() {
    let mut state = ChatState::default();
    state.conversation_pinned = true;
    state.conversation_viewport_height = 650.0;
    for index in 0..22 {
        state.items.push_back(ChatItem {
            id: format!("resumed-{index}"),
            kind: if index % 2 == 0 {
                ChatItemKind::User
            } else {
                ChatItemKind::Agent
            },
            // The estimate treats this as tall, while rich Markdown can
            // lay it out much shorter. Every row must still be present.
            text: "A long resumed message ".repeat(100),
            complete: true,
        });
    }

    let window = transcript_window(&state);
    assert_eq!(window.range, 0..22);
    assert_eq!(window.leading, 0.0);
    assert_eq!(window.trailing, 0.0);

    for index in 22..=TRANSCRIPT_VIRTUALIZATION_THRESHOLD {
        state.items.push_back(ChatItem {
            id: format!("resumed-{index}"),
            kind: ChatItemKind::Agent,
            text: "Short".into(),
            complete: true,
        });
    }
    let large_window = transcript_window(&state);
    assert!(large_window.range.start > 0);
    assert_eq!(large_window.range.end, state.items.len());
}

#[test]
fn pinned_large_transcript_mounts_bounded_tail_despite_height_overestimates() {
    let mut state = ChatState::default();
    state.conversation_pinned = true;
    state.conversation_viewport_height = 650.0;
    for index in 0..200 {
        state.items.push_back(ChatItem {
            id: format!("long-{index}"),
            kind: ChatItemKind::Agent,
            text: "Markdown source much taller than its rendered card ".repeat(300),
            complete: true,
        });
    }

    let window = transcript_window(&state);
    assert_eq!(window.range, 136..200);
    assert_eq!(window.trailing, 0.0);
    assert!(window.leading > 0.0);

    state.conversation_pinned = false;
    state.conversation_scroll = 0.0;
    let unpinned = transcript_window(&state);
    assert_eq!(unpinned.range.start, 0);
    assert!(unpinned.range.end < 200);
}

#[test]
fn run_settings_close_icon_has_a_legible_raster_footprint() {
    let icon = close_icon_pixels(0x00ff_ffff);
    assert_eq!(icon.dimensions(), (24, 24));
    let painted = icon.pixels().filter(|pixel| pixel[3] != 0).count();
    assert!(
        painted >= 80,
        "close strokes should not collapse into a tiny glyph"
    );
}

#[test]
fn shell_approval_snapshot_rejects_scope_revision_and_repeated_action() {
    let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    let request_id = ServerRequestId("request".into());
    let request = |sequence, root: &str| {
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: request_id.clone(),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/fileChange/requestApproval".into(),
                summary: Some("Write files".into()),
                context: nickel_codex::ApprovalContext {
                    grant_root: Some(root.into()),
                    ..Default::default()
                },
            },
        })
    };
    app.state.apply(app.state.generation, request(1, "/safe"));
    let old = app.approval_notifications().remove(0);
    // A duplicate authority fact must not create a fresh action identity or toast.
    app.state.apply(app.state.generation, request(2, "/safe"));
    assert_eq!(app.approval_notifications(), vec![old.clone()]);
    assert!(app.report_approval_notification_delivery(&old, false));
    assert!(app.state.approval_notification_unavailable(&request_id));
    app.state
        .apply(app.state.generation, request(3, "/broader"));
    assert!(!app.state.approval_notification_unavailable(&request_id));
    assert!(!app.report_approval_notification_delivery(&old, false));
    assert!(!app.respond_approval_notification(&old, CodexApprovalChoice::Approve));

    let broader = app.approval_notifications().remove(0);
    assert_eq!(
        broader.presentation.scope.as_deref(),
        Some("Requested write root: /broader")
    );
    app.state.apply(app.state.generation, request(4, "/safe"));
    assert!(!app.respond_approval_notification(&old, CodexApprovalChoice::Approve));
    assert!(!app.respond_approval_notification(&broader, CodexApprovalChoice::Approve));
    let current = app.approval_notifications().remove(0);
    assert!(current.request_revision > old.request_revision);
    assert!(app.report_approval_notification_delivery(&current, false));
    assert!(app.state.approval_notification_unavailable(&request_id));
    assert!(app.report_approval_notification_delivery(&current, true));
    assert!(!app.state.approval_notification_unavailable(&request_id));
    assert!(app.respond_approval_notification(&current, CodexApprovalChoice::Cancel));
    assert!(!app.respond_approval_notification(&current, CodexApprovalChoice::Approve));
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 5,
            kind: nickel_codex::EventKind::ServerRequestResolved {
                thread_id: ThreadId("thread".into()),
                request_id: request_id.clone(),
            },
        }),
    );
    app.state.apply(app.state.generation, request(6, "/new"));
    let reused = app.approval_notifications().remove(0);
    assert!(reused.request_revision > current.request_revision);
    assert!(!app.respond_approval_notification(&current, CodexApprovalChoice::Cancel));
    assert!(app.respond_approval_notification(&reused, CodexApprovalChoice::Cancel));
}

#[test]
fn unsupported_approval_cannot_claim_a_notification_response() {
    let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    let request_id = ServerRequestId("unsupported".into());
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: request_id.clone(),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/unknown/requestApproval".into(),
                summary: Some("Unknown decision".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        }),
    );
    let notification = app.approval_notifications().remove(0);
    assert!(!notification.actionable);
    assert!(!app.respond_approval_notification(&notification, CodexApprovalChoice::Approve));
    assert!(!app.state.interaction_submission_pending(&request_id));
}

#[test]
fn offered_command_decisions_gate_both_notification_and_inline_actions() {
    let backend = ReplayBackend::from_json(r#"{"name":"choices","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    let request_id = ServerRequestId("decision-set".into());
    let decisions = vec![CommandDecision::AcceptForSession, CommandDecision::Decline];
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: request_id.clone(),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run command".into()),
                context: nickel_codex::ApprovalContext {
                    available_decisions: Some(decisions),
                    ..Default::default()
                },
            },
        }),
    );
    let notification = app.approval_notifications().remove(0);
    assert!(notification.needs_review());
    assert!(notification.notification_actions().is_empty());
    assert!(
        notification
            .presentation
            .detail
            .as_deref()
            .unwrap()
            .contains("future matching prompts in this session")
    );
    assert!(!app.respond_approval_notification(&notification, CodexApprovalChoice::Approve));
    app.update(ChatMessage::RespondApproval(
        request_id.clone(),
        "item/commandExecution/requestApproval".into(),
        CodexApprovalChoice::Cancel,
    ));
    assert!(!app.state.interaction_submission_pending(&request_id));
    assert!(app.respond_approval_notification(
        &notification,
        CodexApprovalChoice::Command(CommandDecision::AcceptForSession),
    ));
    assert!(app.state.interaction_submission_pending(&request_id));
    assert!(matches!(
        commands.recv_timeout(std::time::Duration::from_millis(10)),
        Ok(ControllerCommand::CommandApproval {
            request_id: delivered,
            decision: CommandDecision::AcceptForSession,
        }) if delivered == request_id
    ));
}

#[test]
fn compact_four_decision_approval_scrolls_to_last_action() {
    let backend = ReplayBackend::from_json(r#"{"name":"choices","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: ServerRequestId("four-choices".into()),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run command".into()),
                context: nickel_codex::ApprovalContext {
                    available_decisions: Some(vec![
                        CommandDecision::Accept,
                        CommandDecision::AcceptForSession,
                        CommandDecision::Decline,
                        CommandDecision::Cancel,
                    ]),
                    ..Default::default()
                },
            },
        }),
    );
    let mut scenario = Scenario::new(app, 640, 480);
    let pending = scenario
        .host()
        .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Approve".into(),
        })
        .expect("first approval action");
    let point = twinkle_ui::Point {
        x: pending.bounds.origin.x + pending.bounds.size.width / 2.0,
        y: pending.bounds.origin.y + pending.bounds.size.height / 2.0,
    };
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::Scroll {
            point,
            delta_y: 400.0,
        });
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::Button,
            "Cancel turn",
        ))
        .expect("last offered decision remains reachable after scrolling");
}

#[test]
fn approval_copy_uses_structured_scope_without_exposing_protocol_ids() {
    let context = nickel_codex::ApprovalContext {
        item_id: Some("internal-item-id".into()),
        approval_id: Some("internal-callback-id".into()),
        command: Some("cargo test".into()),
        cwd: Some("/projects/nickel".into()),
        asks_network_access: true,
        proposes_session_rule: true,
        ..Default::default()
    };
    let copy = codex_approval_presentation(
        "item/commandExecution/requestApproval",
        "Run tests",
        &context,
    );
    assert!(copy.scope_line().contains("/projects/nickel"));
    assert!(copy.warning.as_deref().unwrap().contains("future commands"));
    assert!(copy.detail.as_deref().unwrap().contains("cargo test"));
    assert!(!copy.notification_body().contains("internal-item-id"));
    assert!(!copy.notification_body().contains("internal-callback-id"));
    let missing = codex_approval_presentation(
        "item/fileChange/requestApproval",
        "Edit files",
        &nickel_codex::ApprovalContext::default(),
    );
    assert!(missing.scope_line().contains("not supplied"));
}

#[test]
fn command_approval_shows_command_outside_bounded_secondary_details() {
    let command = "cargo test -p nickel-codex-ui --lib";
    let interaction = PendingInteraction::Approval {
        request_id: ServerRequestId("approval".into()),
        approval_type: "item/commandExecution/requestApproval".into(),
        summary: "Run a command".into(),
        context: nickel_codex::ApprovalContext {
            command: Some(command.into()),
            cwd: Some("/projects/nickel".into()),
            ..Default::default()
        },
    };
    let frame = UiFrame::layout(
        ui! { <InteractionCard interaction={&interaction} answer={""} actionable={true}
        submitting={false} delivery={InteractionDeliveryStatus {
            response_unconfirmed: false,
            notification_unavailable: false,
        }} theme={semantic_theme()} compact_approval_details={true} /> },
        Rect::new(0.0, 0.0, 520.0, 240.0),
    );
    assert!(has_accessible_text(&frame, command));
    assert!(has_accessible_text(&frame, "Approve"));
    let nodes = frame.resolved_layout().nodes();
    let command_node = nodes
        .iter()
        .find(|node| node.id.as_str().ends_with("approval-command"))
        .expect("prominent command node");
    let detail_node = nodes
        .iter()
        .find(|node| node.id.as_str().ends_with("approval-details"))
        .expect("secondary detail node");
    assert!(command_node.allocated.origin.y < detail_node.allocated.origin.y);
}

fn alternate_theme() -> SemanticTheme {
    SemanticTheme::from_tokens(twinkle_ui::SemanticTokenSet::standard(
        0xf4f6f8, 0xe8edf4, 0xffffff, 0xd6dce5, 0xcbd2dc, 0x171a20, 0x4d5664, 0x075ca8, 0xc9e5ff,
        0x6c3fa0, 0xefe4ff,
    ))
}

#[test]
fn backend_diagnostics_are_reachable_but_not_permanent_composer_copy() {
    let backend = ReplayBackend::from_json(r#"{"name":"diagnostics","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.status = ConnectionStatus::Disconnected;
    app.state.account.authenticated = true;
    app.state.provenance = "Installed Codex 9.9.9".into();
    app.state.backend_source = Some(nickel_codex::CandidateSource::Bundled);
    app.state.fallback_reason = Some("installed schema incompatible".into());
    app.state.report_diagnostic("Transport interrupted");
    let normal = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            Rect::new(0.0, 0.0, 900.0, 640.0),
            twinkle_ui::InputModality::Keyboard,
        )),
        Rect::new(0.0, 0.0, 900.0, 640.0),
    );
    assert!(has_accessible_text(&normal, "Codex is disconnected"));
    assert!(!has_accessible_text(&normal, "Installed Codex 9.9.9"));
    app.update(ChatMessage::ToggleDiagnostics);
    let summary = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            Rect::new(0.0, 0.0, 900.0, 640.0),
            twinkle_ui::InputModality::Keyboard,
        )),
        Rect::new(0.0, 0.0, 900.0, 640.0),
    );
    assert!(has_accessible_text(
        &summary,
        "The Codex connection was interrupted."
    ));
    assert!(has_accessible_text(&summary, "Copy safe summary"));
    assert!(has_accessible_text(&summary, "Show technical details"));
    assert!(!has_accessible_text(&summary, "Installed Codex 9.9.9"));
    app.update(ChatMessage::ToggleDiagnosticDetails);
    let details = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            Rect::new(0.0, 0.0, 900.0, 640.0),
            twinkle_ui::InputModality::Keyboard,
        )),
        Rect::new(0.0, 0.0, 900.0, 640.0),
    );
    assert!(has_accessible_text(&details, "Installed Codex 9.9.9"));
    assert!(has_accessible_text(&details, "Transport interrupted"));
    assert!(has_accessible_text(
        &details,
        "Backend fallback: installed schema incompatible"
    ));
    assert!(has_accessible_text(&details, "Reconnect"));
}

#[test]
fn diagnostic_surface_distinguishes_backend_transport_auth_and_turn_failures() {
    for (status, authenticated, turn_failed, cause, reconnect) in [
        (
            ConnectionStatus::Unavailable,
            true,
            false,
            "Codex is not available.",
            true,
        ),
        (
            ConnectionStatus::Disconnected,
            true,
            false,
            "The Codex connection was interrupted.",
            true,
        ),
        (
            ConnectionStatus::Incompatible,
            true,
            false,
            "The Codex backend is incompatible with this version of Nickel.",
            true,
        ),
        (
            ConnectionStatus::Ready,
            false,
            false,
            "Codex needs sign-in.",
            false,
        ),
        (
            ConnectionStatus::Ready,
            true,
            true,
            "The last turn failed, but the connection is healthy.",
            false,
        ),
    ] {
        let backend = ReplayBackend::from_json(r#"{"name":"failure-state","events":[]}"#)
            .expect("offline backend");
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = status;
        app.state.account.authenticated = authenticated;
        if turn_failed {
            app.state.last_turn_error = Some((
                nickel_codex::TurnId("turn".into()),
                "The command failed".into(),
                false,
            ));
        }
        app.update(ChatMessage::ToggleDiagnostics);
        let area = Rect::new(0.0, 0.0, 640.0, 480.0);
        let frame = UiFrame::layout(
            app.view(twinkle_ui::ViewContext::new(
                area,
                twinkle_ui::InputModality::Keyboard,
            )),
            area,
        );
        assert!(has_accessible_text(&frame, cause), "missing {cause}");
        assert_eq!(
            has_accessible_text(&frame, "Reconnect"),
            reconnect,
            "wrong recovery action for {cause}"
        );
    }
}

#[test]
fn unauthenticated_ready_transport_shows_sign_in_not_send() {
    let backend = ReplayBackend::from_json(r#"{"name":"sign-in","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.status = ConnectionStatus::Ready;
    app.state.draft = "Do not send".into();
    let frame = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            Rect::new(0.0, 0.0, 640.0, 480.0),
            twinkle_ui::InputModality::Keyboard,
        )),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&frame, "Sign in to Codex"));
    assert!(
        !frame
            .resolved_layout()
            .nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("send-button"))
    );
}

#[test]
fn copied_diagnostic_summary_excludes_source_payloads_and_secrets() {
    let backend = ReplayBackend::from_json(r#"{"name":"safe-copy","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.status = ConnectionStatus::Disconnected;
    app.state.provenance = "token=super-secret".into();
    app.state.fallback_reason = Some("fallback secret".into());
    app.state
        .report_diagnostic("prompt content /private/file pairing-secret");
    app.state.draft = "private draft".into();
    app.state.selected_thread = Some(ThreadId("private-thread-id".into()));
    app.update(ChatMessage::ToggleDiagnostics);
    app.update(ChatMessage::CopyDiagnosticSummary);
    let copied = app.clipboard_write.take().unwrap();
    assert!(copied.contains("Disconnected"));
    for secret in [
        "super-secret",
        "fallback secret",
        "prompt content",
        "/private/file",
        "pairing-secret",
        "private draft",
        "private-thread-id",
    ] {
        assert!(!copied.contains(secret), "copied summary included {secret}");
    }
    assert!(app.diagnostic_copy_result.is_none());
    assert!(app.clipboard_write_completed(Ok(())));
    assert!(matches!(app.diagnostic_copy_result, Some(Ok(()))));
    let area = Rect::new(0.0, 0.0, 640.0, 480.0);
    let copied = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            area,
            twinkle_ui::InputModality::Keyboard,
        )),
        area,
    );
    assert!(has_accessible_text(
        &copied,
        "Safe diagnostic summary copied to clipboard"
    ));

    app.update(ChatMessage::CopyDiagnosticSummary);
    let _ = app.clipboard_write.take().unwrap();
    assert!(app.clipboard_write_completed(Err("selection unavailable".into())));
    let failed = UiFrame::layout(
        app.view(twinkle_ui::ViewContext::new(
            area,
            twinkle_ui::InputModality::Keyboard,
        )),
        area,
    );
    assert!(has_accessible_text(
        &failed,
        "Safe diagnostic summary could not be copied"
    ));
}

#[test]
fn authentication_and_phone_permission_views_protect_remote_access() {
    let backend = ReplayBackend::from_json(r#"{"name":"protection","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    assert!(
        app.remote_access_protected(),
        "initial sign-in view must be protected"
    );
    app.state.account.authenticated = true;
    assert!(!app.remote_access_protected());
    app.update(ChatMessage::OpenRemoteControl);
    assert!(app.remote_access_protected());
    app.update(ChatMessage::CloseRemoteControl);
    assert!(!app.remote_access_protected());
    app.update(ChatMessage::ManageRemoteHosts);
    assert!(app.remote_access_protected());
    app.update(ChatMessage::CloseRemoteHosts);
    assert!(!app.remote_access_protected());
    app.update(ChatMessage::AddRemoteHost);
    assert!(app.remote_access_protected());
    app.update(ChatMessage::CloseRemoteHosts);
    assert!(!app.remote_access_protected());
    app.state.apply(
        app.state.generation,
        ControllerEvent::LoginStarted(nickel_codex::LoginChallenge::DeviceCode {
            login_id: "fixture".into(),
            user_code: "PRIVATE-CODE".into(),
            verification_url: "https://example.test/device".into(),
        }),
    );
    assert!(
        app.remote_access_protected(),
        "device code is ordinary text but remains protected"
    );
    app.state.apply(
        app.state.generation,
        ControllerEvent::LoginCancelled("fixture".into()),
    );
    assert!(!app.remote_access_protected());
}

#[test]
fn escape_backs_out_of_phone_and_host_panels_before_interrupting_chat() {
    let backend = ReplayBackend::from_json(r#"{"name":"panel-back","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.active_turn = Some(nickel_codex::TurnId("running".into()));
    app.update(ChatMessage::OpenRemoteControl);
    app.shortcut_outcome(Shortcut::Escape);
    assert!(!app.remote_control_open);
    assert_eq!(
        app.state.active_turn,
        Some(nickel_codex::TurnId("running".into()))
    );

    app.update(ChatMessage::ManageRemoteHosts);
    app.update(ChatMessage::AddRemoteHost);
    app.shortcut_outcome(Shortcut::Escape);
    assert!(app.managing_hosts);
    assert!(app.host_editor.is_none());
    app.shortcut_outcome(Shortcut::Escape);
    assert!(!app.managing_hosts);
}

#[test]
fn leaving_phone_panel_stops_hidden_pairing_attempt() {
    let backend = ReplayBackend::from_json(r#"{"name":"pair-back","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.remote_control_open = true;
    app.state.remote_pairing = Some(nickel_codex::RemotePairingChallenge {
        environment_id: "environment".into(),
        expires_at: 42,
        pairing_code: "opaque".into(),
        manual_pairing_code: Some("12345678".into()),
    });

    app.update(ChatMessage::CloseRemoteControl);

    assert!(!app.remote_control_open);
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::CancelRemotePairing)
    ));
}

#[test]
fn empty_controller_polls_back_off_without_rebuilding() {
    let backend = ReplayBackend::from_json(r#"{"name":"idle","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    let started = Instant::now();
    let mut host = UiHost::new_at(app, 800, 600, started);

    let first_due = started + CONTROLLER_POLL_MIN;
    let first = host.step(HostBatch {
        now: Some(first_due),
        events: vec![HostEvent::Poll],
        ..HostBatch::default()
    });
    assert!(!first.changed);
    assert!(!first.telemetry.rebuilt);
    assert_eq!(
        first.next_deadline,
        Some(first_due + Duration::from_millis(32))
    );

    let second_due = first.next_deadline.unwrap();
    let second = host.step(HostBatch {
        now: Some(second_due),
        events: vec![HostEvent::Poll],
        ..HostBatch::default()
    });
    assert!(!second.changed);
    assert!(!second.telemetry.rebuilt);
    assert_eq!(
        second.next_deadline,
        Some(second_due + Duration::from_millis(64))
    );
}

#[test]
fn user_activity_restores_low_latency_controller_polling() {
    let backend = ReplayBackend::from_json(r#"{"name":"idle","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    for _ in 0..4 {
        assert!(!app.poll_controller());
    }
    assert_eq!(Application::poll_interval(&app), Some(CONTROLLER_POLL_MAX));

    app.update(ChatMessage::DraftChanged("hello".into()));

    assert_eq!(Application::poll_interval(&app), Some(CONTROLLER_POLL_MIN));
}

#[test]
fn embedded_theme_contract_is_live_and_idempotent() {
    let backend = ReplayBackend::from_json(r#"{"name":"theme","events":[]}"#).unwrap();
    let app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let light = alternate_theme();

    let mut scenario = Scenario::new(app, 900, 640);
    let dark_commands = scenario.host().commands().to_vec();
    assert!(scenario.host_mut().application_mut().set_theme(light));
    scenario.host_mut().step(HostBatch {
        events: vec![HostEvent::Poll],
        ..HostBatch::default()
    });

    assert_eq!(scenario.host().application().theme, light);
    assert_ne!(scenario.host().commands(), dark_commands);
    assert!(!scenario.host_mut().application_mut().set_theme(light));
}

#[test]
fn production_codex_and_task_switcher_views_reject_literal_colors() {
    fn assert_no_literal_colors(source: &str, label: &str) {
        let offenders = source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let color_sink = [
                    "background=",
                    "color=",
                    "Border::new(",
                    "foreground:",
                    "muted:",
                    "accent:",
                    "surface:",
                    "border:",
                    "code:",
                ]
                .iter()
                .any(|sink| line.contains(sink));
                color_sink
                    && line.as_bytes().windows(2).any(|prefix| prefix == b"0x")
                    && !line.trim_start().starts_with("//")
            })
            .map(|(line, text)| format!("{}: {}", line + 1, text.trim()))
            .collect::<Vec<_>>();
        assert!(offenders.is_empty(), "{label}: {offenders:?}");
    }

    let codex = include_str!("../view.rs").replace("\r\n", "\n");
    let fallback_start = codex.find("fn semantic_theme()").unwrap();
    let fallback_end = codex[fallback_start..]
        .find("\n}\n\n")
        .map(|offset| fallback_start + offset + 3)
        .unwrap();
    let mut production = codex[..fallback_start].to_owned();
    production.push_str(&codex[fallback_end..codex.find("#[cfg(test)]\nmod tests").unwrap()]);
    assert_no_literal_colors(&production, "Codex UI");
    for (label, source) in [
        ("application", include_str!("application.rs")),
        ("transcript", include_str!("transcript.rs")),
        ("navigation", include_str!("navigation.rs")),
        ("composer", include_str!("composer.rs")),
        ("diagnostics", include_str!("diagnostics.rs")),
        ("connection", include_str!("connection.rs")),
    ] {
        assert_no_literal_colors(source, label);
    }
    assert_no_literal_colors(
        include_str!("../../../nickel-shell/src/window_preview.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap(),
        "task switcher",
    );
}

fn has_accessible_text<Message: Clone>(frame: &UiFrame<Message>, needle: &str) -> bool {
    frame.accessibility_nodes().iter().any(|node| {
        node.label
            .as_deref()
            .is_some_and(|label| label.contains(needle))
            || node
                .description
                .as_deref()
                .is_some_and(|description| description.contains(needle))
    })
}

#[test]
fn interaction_fixture_has_a_deterministic_activity_projection() {
    let fixture: nickel_codex::ReplayScenario = serde_json::from_str(include_str!(
        "../../../nickel-codex-fixture/fixtures/interactions.json"
    ))
    .expect("offline interaction fixture");
    let project = || {
        let mut state = ChatState::default();
        for event in fixture.events.iter().cloned() {
            state.apply(state.generation, ControllerEvent::Protocol(event));
        }
        assert_eq!(state.pending.len(), 3);
        state
            .items
            .iter()
            .map(|item| {
                (
                    item.id.clone(),
                    item_label(&item.kind).to_owned(),
                    if is_collapsible_activity(&item.kind) {
                        activity_summary(
                            item,
                            state.activity_outcomes.get(&item.id),
                            state
                                .file_change_summaries
                                .get(&item.id)
                                .map(String::as_str),
                        )
                    } else {
                        item.text.clone()
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let first = project();
    assert_eq!(
        first,
        project(),
        "replaying the same fixture must not reorder cards"
    );
    assert_eq!(
        first,
        vec![
            (
                "local:request:1:1".into(),
                "Question from Codex".into(),
                "Which implementation should Codex use?\nAnswer in the request panel below.".into(),
            ),
            (
                "command-1".into(),
                "Command".into(),
                "Running command".into()
            ),
            (
                "patch-1".into(),
                "File change".into(),
                "Running file change".into(),
            ),
            ("plan-1".into(), "Plan".into(), "fixture plan".into()),
            (
                "reasoning-1".into(),
                "Reasoning summary".into(),
                "fixture summary".into(),
            ),
        ]
    );
}

#[test]
fn file_change_card_never_promotes_diff_prose_to_a_target() {
    let item = ChatItem {
        id: "file".into(),
        kind: ChatItemKind::FileChange,
        text: "+/not/an/authoritative/path\n-older line".into(),
        complete: false,
    };
    assert_eq!(activity_summary(&item, None, None), "Running file change");
    assert_eq!(
        activity_summary(&item, None, Some("update: /project/real.rs")),
        "Running: update: /project/real.rs"
    );
}

#[test]
fn primary_chat_regions_remain_reachable_across_required_viewports() {
    let mut state = ChatState::default();
    state.account.authenticated = true;
    state.status = ConnectionStatus::Ready;
    // These are logical client areas, not physical output dimensions. The
    // scaled-output case is represented by its usable 960 × 540 area; the
    // 1120-unit case matches the nested project's default chat window.
    for draft in ["", "A message ready to send"] {
        state.draft = draft.into();
        for (width, height) in [
            (1920.0, 1080.0),
            (1366.0, 768.0),
            (1280.0, 720.0),
            (1120.0, 760.0),
            (960.0, 540.0),
            (640.0, 480.0),
            (640.0, 960.0),
        ] {
            let frame = UiFrame::layout(
                configured_chat_view(
                    &state,
                    &DEFAULT_CODEX_SETTINGS,
                    ChatHostPanel::default(),
                    ChatOverlays::default(),
                    semantic_theme(),
                    width,
                ),
                Rect::new(0.0, 0.0, width, height),
            );
            let nodes = frame.resolved_layout().nodes();
            for suffix in [
                "conversation",
                "composer-viewport",
                "composer-status",
                "send-button",
            ] {
                let bounds = nodes
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .unwrap_or_else(|| panic!("{suffix} missing at {width}×{height}"))
                    .allocated;
                assert!(
                    bounds.size.width > 0.0 && bounds.size.height > 0.0,
                    "{suffix} collapsed at {width}×{height}: {bounds:?}"
                );
                assert!(
                    bounds.origin.x >= 0.0 && bounds.origin.y >= 0.0,
                    "{suffix} starts outside {width}×{height}: {bounds:?}"
                );
                assert!(
                    bounds.origin.x + bounds.size.width <= width + 0.5
                        && bounds.origin.y + bounds.size.height <= height + 0.5,
                    "{suffix} extends outside {width}×{height}: {bounds:?}"
                );
            }
            assert!(
                nodes
                    .iter()
                    .any(|node| node.id.as_str().ends_with("codex-menu"))
            );
        }
    }
}

#[test]
fn long_content_keeps_composer_and_transcript_within_compact_client_bounds() {
    let cases = [
        (ChatItemKind::User, "A long user request. ".repeat(180)),
        (
            ChatItemKind::Agent,
            "A long assistant explanation. ".repeat(220),
        ),
        (
            ChatItemKind::FileChange,
            format!("/project/{}", "deep/nested/".repeat(180)),
        ),
        (
            ChatItemKind::Command,
            format!("$ cargo test {}", "--all-targets ".repeat(240)),
        ),
        (
            ChatItemKind::FileChange,
            format!("@@ -1 +1 @@\n{}", "+changed line\n".repeat(240)),
        ),
        (
            ChatItemKind::Error,
            "Backend failure with context. ".repeat(180),
        ),
        (
            ChatItemKind::Unknown("future/event".into()),
            "Unrecognized event. ".repeat(180),
        ),
    ];
    for (index, (kind, text)) in cases.into_iter().enumerate() {
        let mut state = ChatState::default();
        state.account.authenticated = true;
        state.status = ConnectionStatus::Ready;
        state.draft = "A long pending prompt. ".repeat(120);
        state.items.push_back(ChatItem {
            id: format!("stress-{index}"),
            kind,
            text,
            complete: true,
        });
        if matches!(
            state.items.back().map(|item| &item.kind),
            Some(ChatItemKind::Command | ChatItemKind::FileChange)
        ) {
            state.expanded_items.insert(format!("stress-{index}"));
        }
        for (width, height) in [(640.0, 480.0), (960.0, 540.0)] {
            let frame = UiFrame::layout(
                configured_chat_view(
                    &state,
                    &DEFAULT_CODEX_SETTINGS,
                    ChatHostPanel::default(),
                    ChatOverlays::default(),
                    semantic_theme(),
                    width,
                ),
                Rect::new(0.0, 0.0, width, height),
            );
            let nodes = frame.resolved_layout().nodes();
            let region = |suffix: &str| {
                nodes
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .unwrap_or_else(|| panic!("{suffix} missing in case {index}"))
                    .allocated
            };
            let conversation = region("conversation");
            let editor = region("composer-viewport");
            let submit = region("send-button");
            let transcript = region("transcript-column");
            assert!(
                conversation.size.height >= 120.0,
                "case {index} at {width}×{height}"
            );
            assert!((52.0..=140.0).contains(&editor.size.height));
            let horizontal_padding = if width < 720.0 { 16.0 } else { 36.0 };
            assert!(
                transcript.size.width >= conversation.size.width - horizontal_padding - 0.5,
                "case {index} transcript does not use available width at {width}×{height}: {transcript:?}"
            );
            // Transcript content intentionally extends inside its clipped,
            // independently scrollable conversation viewport.
            assert!(transcript.origin.y >= conversation.origin.y);
            for (name, bounds) in [
                ("conversation", conversation),
                ("editor", editor),
                ("submit", submit),
            ] {
                assert!(
                    bounds.origin.x >= 0.0 && bounds.origin.y >= 0.0,
                    "{name}: {bounds:?}"
                );
                assert!(
                    bounds.origin.x + bounds.size.width <= width + 0.5,
                    "{name}: {bounds:?}"
                );
                assert!(
                    bounds.origin.y + bounds.size.height <= height + 0.5,
                    "{name}: {bounds:?}"
                );
            }
        }
    }
}

#[test]
fn compact_composer_bounds_attachment_previews_without_hiding_editor() {
    let mut state = ChatState::default();
    state.account.authenticated = true;
    for index in 0..8 {
        state.attachments.push(
            crate::PendingAttachment::from_rgba(
                crate::AttachmentId(index),
                1,
                1,
                &[255, 0, 0, 255],
                crate::AttachmentLimits::default(),
            )
            .expect("fixture image"),
        );
    }
    let frame = UiFrame::layout(
        configured_chat_view(
            &state,
            &DEFAULT_CODEX_SETTINGS,
            ChatHostPanel::default(),
            ChatOverlays::default(),
            semantic_theme(),
            640.0,
        ),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    let nodes = frame.resolved_layout().nodes();
    let attachments = nodes
        .iter()
        .find(|node| node.id.as_str().ends_with("attachment-list"))
        .expect("bounded attachments");
    let editor = nodes
        .iter()
        .find(|node| node.id.as_str().ends_with("composer-viewport"))
        .expect("composer editor");
    assert!(attachments.allocated.size.height <= 96.0);
    assert!(editor.allocated.origin.y + editor.allocated.size.height <= 480.0);
    assert!(has_accessible_text(&frame, "Remove image attachment 7"));
}

#[test]
fn compact_pending_approval_preserves_transcript_and_actions() {
    let mut state = ChatState::default();
    state.account.authenticated = true;
    state.status = ConnectionStatus::Ready;
    state.apply(
        state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: ServerRequestId("compact-approval".into()),
                thread_id: Some(nickel_codex::ThreadId("thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run tests".into()),
                context: nickel_codex::ApprovalContext {
                    command: Some("cargo test ".to_owned() + &"--all ".repeat(100)),
                    cwd: Some("/projects/nickel".into()),
                    ..Default::default()
                },
            },
        }),
    );
    // A saturated shell feed must leave the inline request actionable.
    state.mark_approval_notification_unavailable(&ServerRequestId("compact-approval".into()));
    let frame = UiFrame::layout(
        configured_chat_view(
            &state,
            &DEFAULT_CODEX_SETTINGS,
            ChatHostPanel::default(),
            ChatOverlays::default(),
            semantic_theme(),
            640.0,
        ),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    let nodes = frame.resolved_layout().nodes();
    let bounds = |suffix: &str| {
        nodes
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .expect("Codex layout region")
            .allocated
    };
    assert!(bounds("conversation").size.height >= 120.0);
    assert!(bounds("pending-interactions").size.height <= 146.0);
    assert!(bounds("approval-details").size.height <= 18.0);
    let editor = bounds("composer-viewport");
    assert!(editor.origin.y + editor.size.height <= 480.0);
    assert!(has_accessible_text(&frame, "Approve"));
    assert!(has_accessible_text(
        &frame,
        "Shell notification unavailable"
    ));
    let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state = state;
    let mut scenario = Scenario::new(app, 640, 480);
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::Button,
            "Approve",
        ))
        .expect("approval action remains clickable at minimum size");
}

#[test]
fn pending_approval_survives_live_run_settings_collapse_and_expansion() {
    let backend = ReplayBackend::from_json(r#"{"name":"resize","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::ApprovalRequested {
                request_id: ServerRequestId("resize-approval".into()),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("Run tests".into()),
                context: nickel_codex::ApprovalContext::default(),
            },
        }),
    );
    let mut scenario = Scenario::new(app, 1280, 720);
    // Run settings replace the three inline selectors below 1280 logical
    // units; the source-owned approval must not disappear in either mode.
    for (width, height) in [(1279, 720), (960, 540), (640, 480), (1280, 720)] {
        scenario.resize(width, height, 1.0).expect("live resize");
        let approve = scenario
            .host()
            .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Approve".into(),
            })
            .expect("approval decision after resize");
        assert!(approve.bounds.origin.x + approve.bounds.size.width <= width as f32);
        assert!(approve.bounds.origin.y + approve.bounds.size.height <= height as f32);
        if width < 1280 {
            scenario
                .pointer_activate(&twinkle_testkit::Selector::id("root/menu-bar/codex-menu"))
                .expect("Codex menu opens");
            scenario
                .pointer_activate(&twinkle_testkit::Selector::role_name(
                    SemanticRole::MenuItem,
                    "Run settings",
                ))
                .expect("menu-owned run settings remains operable");
            assert!(
                scenario
                    .host()
                    .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Approval policy selector".into(),
                    })
                    .is_ok()
            );
            scenario
                .pointer_activate(&twinkle_testkit::Selector::id("root/menu-bar/codex-menu"))
                .expect("Codex menu reopens");
            scenario
                .pointer_activate(&twinkle_testkit::Selector::role_name(
                    SemanticRole::MenuItem,
                    "Run settings",
                ))
                .expect("menu-owned run settings closes");
        }
    }
    scenario
        .resize(960, 540, 2.0)
        .expect("high-scale logical client");
    scenario.host_mut().set_scale_factor(2.0);
    scenario
        .pointer_activate(&twinkle_testkit::Selector::id("root/menu-bar/codex-menu"))
        .expect("scaled Codex menu hit region");
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::MenuItem,
            "Run settings",
        ))
        .expect("scaled menu-owned settings hit region");
    assert!(
        scenario
            .host()
            .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Approval policy selector".into(),
            })
            .is_ok()
    );
}

#[test]
fn streamed_content_does_not_repin_a_reader_scrolled_away() {
    let backend = ReplayBackend::from_json(r#"{"name":"scroll","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.update(ChatMessage::ConversationScrolled(
        twinkle_ui::ScrollExtent {
            viewport: twinkle_ui::Size {
                width: 640.0,
                height: 240.0,
            },
            content: twinkle_ui::Size {
                width: 640.0,
                height: 1200.0,
            },
            offset_x: 0.0,
            offset: 120.0,
        },
    ));
    assert!(!app.state.conversation_pinned);
    let mut scenario = Scenario::new(app, 1280, 720);
    scenario
        .resize(1279, 720, 1.0)
        .expect("collapse before streaming");
    let generation = scenario.host().application().state.generation;
    scenario.host_mut().application_mut().state.apply(
        generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::AgentMessageDelta {
                item_id: "stream".into(),
                delta: "New text arrived while reading history".into(),
            },
        }),
    );
    scenario.host_mut().step(HostBatch {
        application_changed: true,
        ..HostBatch::default()
    });
    scenario
        .resize(640, 480, 1.0)
        .expect("compact while streaming");
    assert!(!scenario.host().application().state.conversation_pinned);
    assert!(
        scenario
            .host()
            .application()
            .state
            .new_content_while_unpinned
    );
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::Button,
            "Jump to latest",
        ))
        .expect("reader can choose to resume following the transcript");
    assert!(scenario.host().application().state.conversation_pinned);
    assert!(
        !scenario
            .host()
            .application()
            .state
            .new_content_while_unpinned
    );
}

#[test]
fn composer_ime_preedit_survives_compact_resize_without_committing_early() {
    let backend = ReplayBackend::from_json(r#"{"name":"ime","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    let mut scenario = Scenario::new(app, 1280, 720);
    let editor = scenario
        .host()
        .query_unique(&twinkle_ui::SemanticSelector::Role(SemanticRole::TextField))
        .expect("composer field");
    let point = twinkle_ui::Point {
        x: editor.bounds.origin.x + editor.bounds.size.width / 2.0,
        y: editor.bounds.origin.y + editor.bounds.size.height / 2.0,
    };
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::PointerPressed(point));
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::PointerReleased(point));
    scenario.ime_preedit("かな").expect("preedit starts");
    assert!(scenario.host().application().state.draft.is_empty());
    scenario.resize(640, 480, 1.0).expect("compact resize");
    assert!(scenario.host().application().state.draft.is_empty());
    scenario.text_input("仮名").expect("commit after resize");
    assert_eq!(scenario.host().application().state.draft, "仮名");
}

#[test]
fn transcript_selection_survives_compact_resize() {
    let backend = ReplayBackend::from_json(r#"{"name":"selection","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::AgentMessageDelta {
                item_id: "selection".into(),
                delta: "A distinct passage for selection across live layout changes.".into(),
            },
        }),
    );
    let mut scenario = Scenario::new(app, 1280, 720);
    let frame = UiFrame::layout(
        configured_chat_view(
            &scenario.host().application().state,
            &DEFAULT_CODEX_SETTINGS,
            ChatHostPanel::default(),
            ChatOverlays::default(),
            semantic_theme(),
            1280.0,
        ),
        Rect::new(0.0, 0.0, 1280.0, 720.0),
    );
    let nodes = frame.resolved_layout().nodes();
    // Markdown body text is paint-only, so select its leaf through the
    // production transcript layout instead of inventing screen coordinates.
    let passage = nodes
        .iter()
        .rfind(|node| {
            node.id.as_str().contains("transcript-selection") && node.allocated.size.height > 0.0
        })
        .unwrap_or_else(|| {
            panic!(
                "rendered transcript passage: {:?}",
                nodes
                    .iter()
                    .map(|node| node.id.as_str())
                    .collect::<Vec<_>>()
            )
        });
    let start = twinkle_ui::Point {
        x: passage.allocated.origin.x + 8.0,
        y: passage.allocated.origin.y + passage.allocated.size.height / 2.0,
    };
    let end = twinkle_ui::Point {
        x: start.x + 100.0,
        y: start.y,
    };
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::PointerPressed(start));
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::PointerMoved(end));
    scenario
        .host_mut()
        .handle_event(twinkle_ui::UiEvent::PointerReleased(end));
    let selected = scenario
        .host()
        .selected_text()
        .expect("selected transcript text");
    assert!(!selected.is_empty());
    scenario.resize(640, 480, 1.0).expect("compact resize");
    assert_eq!(
        scenario.host().selected_text().as_deref(),
        Some(selected.as_str())
    );
}

#[test]
fn open_command_popup_stays_inside_client_during_compact_resize() {
    let backend = ReplayBackend::from_json(r#"{"name":"popup","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.draft = "/".into();
    let mut scenario = Scenario::new(app, 1280, 720);
    for (width, height) in [(1279, 720), (960, 540), (640, 480), (1280, 720)] {
        scenario
            .resize(width, height, 1.0)
            .expect("live popup resize");
        assert_eq!(scenario.host().application().state.draft, "/");
        let popovers = scenario
            .host()
            .query(&twinkle_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Group,
                name: "Slash commands".into(),
            });
        assert!(
            !popovers.is_empty(),
            "command popup missing at {width}×{height}"
        );
        for popover in popovers {
            assert!(popover.bounds.origin.x >= 0.0 && popover.bounds.origin.y >= 0.0);
            assert!(popover.bounds.origin.x + popover.bounds.size.width <= width as f32 + 0.5);
            assert!(popover.bounds.origin.y + popover.bounds.size.height <= height as f32 + 0.5);
        }
    }
}

#[test]
fn slash_search_keyboard_selection_opens_matching_command() {
    let backend = ReplayBackend::from_json(r#"{"name":"slash-keyboard","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.draft = "/sta".into();
    assert!(app.shortcut_outcome(Shortcut::NavigateUp).changed);
    assert!(app.shortcut_outcome(Shortcut::Submit).changed);
    assert!(app.diagnostics_open);
    assert!(app.state.draft.is_empty());
}

#[test]
fn mention_command_searches_and_keyboard_selection_inserts_a_project_path() {
    let backend = ReplayBackend::from_json(r#"{"name":"mention-picker","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;

    app.update(ChatMessage::SelectCommand("/mention".into()));
    assert_eq!(app.state.draft, "@");
    assert_eq!(app.state.file_search_query.as_deref(), Some(""));
    app.state.apply(
        app.state.generation,
        ControllerEvent::FileSearchLoaded {
            query: String::new(),
            matches: vec![
                nickel_codex::FileSearchMatch {
                    root: "/projects/nickel".into(),
                    path: "Cargo.toml".into(),
                    file_name: "Cargo.toml".into(),
                    is_directory: false,
                    score: 100,
                },
                nickel_codex::FileSearchMatch {
                    root: "/projects/nickel".into(),
                    path: "crates/nickel-shell/src/main.rs".into(),
                    file_name: "main.rs".into(),
                    is_directory: false,
                    score: 90,
                },
            ],
        },
    );
    assert!(app.shortcut_outcome(Shortcut::NavigateDown).changed);
    assert!(app.shortcut_outcome(Shortcut::Submit).changed);
    assert_eq!(app.state.draft, "@crates/nickel-shell/src/main.rs ");
    assert!(app.state.file_search_query.is_none());
    assert!(app.state.active_turn.is_none());
}

#[test]
fn mention_replacement_preserves_the_draft_prefix() {
    assert_eq!(mention_fragment("Please inspect @src/ma"), Some("src/ma"));
    assert_eq!(
        replace_mention_fragment("Please inspect @src/ma", "src/main.rs"),
        "Please inspect @src/main.rs "
    );
    assert_eq!(mention_fragment("email@example.test"), None);
}

#[test]
fn feedback_requires_category_and_preserves_explicit_log_consent() {
    let backend = ReplayBackend::from_json(r#"{"name":"feedback","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;

    app.update(ChatMessage::SelectCommand("/feedback".into()));
    assert!(app.feedback_open);
    assert!(app.remote_access_protected());
    app.update(ChatMessage::SubmitFeedback(true));
    assert_eq!(
        app.state.command_feedback.as_deref(),
        Some("Choose a feedback category")
    );
    assert!(commands.try_recv().is_err());

    app.update(ChatMessage::SelectFeedbackCategory("bug".into()));
    app.update(ChatMessage::FeedbackReasonChanged(
        "The picker broke".into(),
    ));
    app.update(ChatMessage::SubmitFeedback(false));
    assert!(app.feedback_pending);
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::UploadFeedback {
            classification,
            reason: Some(reason),
            include_logs: false,
        }) if classification == "bug" && reason == "The picker broke"
    ));
    app.update(ChatMessage::CloseFeedback);
    assert!(app.feedback_open, "an in-flight report cannot be discarded");
}

#[test]
fn feedback_panel_keeps_both_consent_actions_reachable_at_compact_size() {
    let backend = ReplayBackend::from_json(r#"{"name":"feedback-layout","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.update(ChatMessage::SelectCommand("/feedback".into()));
    app.update(ChatMessage::SelectFeedbackCategory("other".into()));

    let mut host = UiHost::new(app, 640, 480);
    host.step(HostBatch::default());
    for name in ["Send without diagnostics", "Send with diagnostics"] {
        let target = host
            .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert!(target.bounds.origin.x >= 0.0 && target.bounds.origin.y >= 0.0);
        assert!(target.bounds.origin.x + target.bounds.size.width <= 640.5);
        assert!(target.bounds.origin.y + target.bounds.size.height <= 480.5);
    }
}

#[test]
fn diff_command_requests_workspace_diff_without_starting_a_turn() {
    let backend = ReplayBackend::from_json(r#"{"name":"diff","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.draft = "/diff".into();

    app.update(ChatMessage::Send);
    assert!(app.state.draft.is_empty());
    assert!(matches!(commands.try_recv(), Ok(ControllerCommand::Diff)));
    assert!(app.state.active_turn.is_none());

    app.state.apply(
        app.state.generation,
        ControllerEvent::DiffLoaded("```diff\n-old\n+new\n```".into()),
    );
    let item = app.state.items.back().expect("local diff transcript item");
    assert_eq!(item.kind, ChatItemKind::SessionNotice);
    assert!(item.text.contains("+new"));
}

#[test]
fn review_command_carries_every_staged_run_setting() {
    let backend = ReplayBackend::from_json(r#"{"name":"review-settings","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.selected_model = Some("gpt-review".into());
    app.state.selected_reasoning_effort = Some("high".into());
    app.state.selected_approval_policy = ApprovalPolicy::Never;
    app.state.selected_sandbox_policy = Some(SandboxPolicy::DangerFullAccess);

    app.update(ChatMessage::SelectCommand("/review".into()));
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::Review {
            model: Some(model),
            reasoning_effort: Some(effort),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
        }) if model == "gpt-review" && effort == "high"
    ));
}

#[test]
fn plan_command_toggles_run_mode_without_sending_a_prompt() {
    let backend = ReplayBackend::from_json(r#"{"name":"plan-toggle","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;

    app.state.draft = "/plan".into();
    app.update(ChatMessage::Send);
    assert!(app.plan_mode);
    assert!(app.state.draft.is_empty());
    assert!(app.state.active_turn.is_none());

    app.state.draft = "/plan".into();
    app.update(ChatMessage::Send);
    assert!(!app.plan_mode);
    assert!(app.state.active_turn.is_none());
}

#[test]
fn compact_user_question_keeps_answer_and_submit_reachable() {
    let mut state = ChatState::default();
    state.account.authenticated = true;
    state.status = ConnectionStatus::Ready;
    let request_id = ServerRequestId("compact-question".into());
    let question = nickel_codex::UserInputQuestion {
        id: "choice".into(),
        header: "Choose an approach".into(),
        question: "Which implementation should Codex use? ".repeat(30),
        options: (0..3)
            .map(|index| nickel_codex::UserInputOption {
                label: format!("Option {index}"),
                description: "Detailed consequences ".repeat(18),
            })
            .collect(),
        is_other: true,
        is_secret: false,
    };
    state.pending.push(PendingInteraction::UserInput {
        request_id,
        question_ids: vec!["choice".into()],
        questions: vec![question],
    });
    let frame = UiFrame::layout(
        configured_chat_view(
            &state,
            &DEFAULT_CODEX_SETTINGS,
            ChatHostPanel::default(),
            ChatOverlays::default(),
            semantic_theme(),
            640.0,
        ),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    let nodes = frame.resolved_layout().nodes();
    let conversation = nodes
        .iter()
        .find(|node| node.id.as_str().ends_with("conversation"))
        .expect("transcript viewport");
    assert!(conversation.allocated.size.height >= 119.5);
    let backend = ReplayBackend::from_json(r#"{"name":"question","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state = state;
    let mut scenario = Scenario::new(app, 640, 480);
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::Button,
            "Submit",
        ))
        .expect("question submit remains clickable at minimum size");
}

#[test]
fn thread_navigation_is_available_before_history_loads() {
    let backend = ReplayBackend::from_json(r#"{"name":"threads","events":[]}"#).unwrap();
    let app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let mut scenario = Scenario::new(app, 640, 480);
    for id in [
        "file-menu",
        "thread-menu",
        "codex-menu",
        "view-menu",
        "connection-menu",
    ] {
        let menu = scenario
            .host()
            .query_unique(&twinkle_ui::SemanticSelector::Id(
                format!("root/menu-bar/{id}").into(),
            ))
            .expect("compact app-chrome menu");
        assert!(menu.bounds.origin.x >= 0.0);
        assert!(menu.bounds.origin.x + menu.bounds.size.width <= 640.0);
    }
    scenario
        .pointer_activate(&twinkle_testkit::Selector::id("root/menu-bar/thread-menu"))
        .expect("thread menu opens before the backend list loads");
    scenario
        .pointer_activate(&twinkle_testkit::Selector::role_name(
            SemanticRole::MenuItem,
            "Browse conversations…",
        ))
        .expect("conversation history remains discoverable");
    assert!(scenario.host_mut().application_mut().resume_picker_open);
}

#[test]
fn source_supplied_thread_title_cannot_expand_menu_without_bound() {
    let title = "A very long project thread title ".repeat(20);
    let compact = bounded_menu_context(&title);
    assert_eq!(compact.chars().count(), 49);
    assert!(compact.ends_with('…'));
    assert_eq!(bounded_menu_context("Current work"), "Current work");
}

#[test]
fn connection_menu_manage_hosts_action_wins_over_loaded_content() {
    let mut state = ChatState::default();
    state.threads.extend((0..40).map(|index| Thread {
        id: ThreadId(format!("thread-{index}")),
        title: Some(format!("Thread {index}")),
        cwd: Some("/projects/nickel".into()),
        last_used_at: Some(index),
        turns: Vec::new(),
        model: None,
        reasoning_effort: None,
    }));
    let backend = ReplayBackend::from_json(r#"{"name":"hosts","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state = state;
    let mut scenario = Scenario::new(app, 900, 640);
    assert!(
        scenario
            .host()
            .query_unique(&twinkle_ui::SemanticSelector::Id(
                "root/menu-bar/connection-menu".into(),
            ))
            .is_ok()
    );

    scenario
        .host_mut()
        .application_mut()
        .update(ChatMessage::ManageRemoteHosts);
    assert!(scenario.host_mut().application_mut().managing_hosts);
}

#[test]
fn chat_uses_shared_markdown_and_keeps_unsupported_html_inert() {
    let item = ChatItem {
        id: "markdown".into(),
        kind: ChatItemKind::Agent,
        text: "# Heading\n- item with `code`\n```rust\nfn main() {}\n```\n<b>plain</b>".into(),
        complete: true,
    };
    let document = item_markdown_document(&item);
    assert_eq!(
        document.logical_text(),
        "Heading\n• item with code\nfn main() {}\n\n<b>plain</b>"
    );
    assert!(!document.diagnostics.is_empty());

    let tree = UiFrame::layout(
        ui! { <ItemCard item={&item} document={std::sync::Arc::new(item_markdown_document(&item))} expanded={true} theme={semantic_theme()} /> },
        Rect::new(0.0, 0.0, 600.0, 400.0),
    );
    assert!(has_accessible_text(&tree, "Heading"));
    assert!(has_accessible_text(&tree, "<b>plain</b>"));
}

#[test]
fn command_activity_is_compact_until_details_are_requested() {
    let item = ChatItem {
        id: "command".into(),
        kind: ChatItemKind::Command,
        text: "$ cargo test\nSENTINEL_OUTPUT".into(),
        complete: true,
    };
    let document = std::sync::Arc::new(item_markdown_document(&item));
    let collapsed = UiFrame::layout(
        ui! { <ItemCard item={&item} document={empty_activity_document()} expanded={false} theme={semantic_theme()} /> },
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&collapsed, "▸"));
    assert!(!has_accessible_text(&collapsed, "SENTINEL_OUTPUT"));
    let expanded = UiFrame::layout(
        ui! { <ItemCard item={&item} document={document} expanded={true} theme={semantic_theme()} /> },
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&expanded, "SENTINEL_OUTPUT"));
}

#[test]
fn completed_command_uses_backend_outcome_not_generic_finished_copy() {
    let item = ChatItem {
        id: "command".into(),
        kind: ChatItemKind::Command,
        text: "$ cargo test\nfailed output".into(),
        complete: true,
    };
    let outcome = ActivityOutcome {
        status: Some("failed".into()),
        exit_code: Some(2),
        duration_ms: Some(17),
    };
    let frame = UiFrame::layout(
        ui! { <ItemCard item={&item} document={empty_activity_document()} expanded={false} outcome={&outcome} theme={semantic_theme()} /> },
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&frame, "failed: $ cargo test"));
    assert!(has_accessible_text(&frame, "● 2"));
    assert!(has_accessible_text(&frame, "Duration: 17 ms"));
    assert!(!has_accessible_text(&frame, "failed output"));
}

#[test]
fn compact_thread_picker_exposes_older_pages_and_partial_window() {
    let mut state = ChatState::default();
    state.thread_snapshot_available = true;
    state.thread_next_cursor = Some("100".into());
    state.thread_windowed = true;
    let frame = UiFrame::layout(
        resume_picker(&state, None, None, false, None, false, semantic_theme()),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&frame, "Older conversations"));
    assert!(has_accessible_text(
        &frame,
        "bounded older-conversation window"
    ));
}

#[test]
fn multiline_code_block_reserves_padding_beyond_both_text_lines() {
    let item = ChatItem {
        id: "code".into(),
        kind: ChatItemKind::Agent,
        text: "```text\nfirst line\nsecond line\n```".into(),
        complete: true,
    };
    let tree = UiFrame::layout(
        ui! { <ItemCard item={&item} document={std::sync::Arc::new(item_markdown_document(&item))} expanded={true} theme={semantic_theme()} /> },
        Rect::new(0.0, 0.0, 600.0, 200.0),
    );
    let nodes = tree.resolved_layout().nodes();
    let (code_index, code) = nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.id.as_str().contains("markdown-code-"))
        .expect("code row");
    let text_bounds = nodes[*code.children.first().expect("code text child")].allocated;
    let code_bounds = nodes
        .iter()
        .find(|node| node.children.contains(&code_index))
        .expect("code container")
        .allocated;
    assert!(text_bounds.size.height >= 31.0);
    assert!(code_bounds.size.height >= text_bounds.size.height + 18.0);
    assert!(text_bounds.origin.y >= code_bounds.origin.y + 9.0);
}

#[test]
fn retry_replaces_a_stopped_controller_generation() {
    let backend = ReplayBackend::from_json(r#"{"name":"retry","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: PathBuf::from("/projects/nickel"),
    });
    app.state.status = ConnectionStatus::Disconnected;
    let generation = app.state.generation;

    app.update(ChatMessage::Refresh);

    assert_eq!(app.state.status, ConnectionStatus::Loading);
    assert_eq!(app.state.generation, generation + 1);
    assert_eq!(app.controller.generation(), generation + 1);
}

#[test]
fn standalone_project_root_remains_fixed_for_new_conversations() {
    let backend = ReplayBackend::from_json(r#"{"name":"fixed-root","events":[]}"#).unwrap();
    let root = PathBuf::from("/projects/nickel");
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: root.clone(),
    });
    app.use_project_root(root.clone());
    app.update(ChatMessage::NewChat);

    assert_eq!(app.shell_project, Some((root, None)));
}

#[test]
fn draft_replacement_uses_host_dialog_and_cancel_preserves_input() {
    let backend = ReplayBackend::from_json(r#"{"name":"discard","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.draft = "Unsent work".into();
    app.update(ChatMessage::NewChat);
    assert!(matches!(
        app.pending_navigation,
        Some(PendingNavigation::NewChat)
    ));
    assert_eq!(app.state.draft, "Unsent work");
    let context = twinkle_ui::ViewContext::new(
        Rect::new(0.0, 0.0, 640.0, 480.0),
        twinkle_ui::InputModality::Keyboard,
    );
    assert_eq!(app.frame_overlays(context).len(), 1);
    let mut host = UiHost::new(app, 640, 480);
    let opened = host.step(HostBatch::default());
    assert_eq!(
        host.inspect().open_overlay,
        Some(OverlayId::new("codex-discard-draft")),
        "{opened:?}"
    );
    let keep = host
        .query_unique(&twinkle_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Keep working".into(),
        })
        .unwrap();
    host.perform_accessibility_action(
        keep.id,
        twinkle_ui::SemanticAction::Invoke(twinkle_ui::ActionKind::Activate),
    );
    assert!(host.application().pending_navigation.is_none());
    assert_eq!(host.application().state.draft, "Unsent work");
    assert_eq!(host.inspect().open_overlay, None);
}

#[test]
fn failed_thread_resume_after_discard_confirmation_keeps_the_draft() {
    let backend = ReplayBackend::from_json(r#"{"name":"resume","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.draft = "Keep on failure".into();
    let destination = ThreadId("other".into());
    app.update(ChatMessage::SelectThread(destination.clone()));
    assert!(matches!(
        app.pending_navigation,
        Some(PendingNavigation::SelectThread(_))
    ));
    app.update(ChatMessage::ConfirmDiscardDraft);
    assert_eq!(app.state.draft, "Keep on failure");
    // A closed controller rejects the command synchronously, so no identity commits.
    assert_eq!(app.resume_picker_pending, None);
    assert_eq!(app.confirmed_thread_discard, None);
    assert_eq!(app.state.draft, "Keep on failure");
}

#[test]
fn shell_resume_requires_discard_confirmation_without_losing_draft() {
    let backend = ReplayBackend::from_json(r#"{"name":"shell-resume","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.shell_host = true;
    app.state.draft = "Unsent work".into();
    let destination = ThreadId("another".into());
    app.update(ChatMessage::SelectThread(destination.clone()));
    assert!(
        matches!(app.take_shell_requests().as_slice(), [ShellRequest::ResumeThread(id)] if *id == destination)
    );
    assert!(!app.prepare_shell_resume(&destination));
    assert!(matches!(
        app.pending_navigation,
        Some(PendingNavigation::SelectThread(_))
    ));
    assert_eq!(app.state.draft, "Unsent work");
    app.update(ChatMessage::CancelDiscardDraft);
    assert_eq!(app.state.draft, "Unsent work");
}

#[test]
fn focusing_existing_writer_clears_picker_without_replacing_draft() {
    let backend = ReplayBackend::from_json(r#"{"name":"owner","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.shell_host = true;
    app.resume_picker_open = true;
    app.resume_picker_pending = Some(ThreadId("open".into()));
    app.state.draft = "Still here".into();
    app.report_resume_owner_activation();
    assert!(!app.resume_picker_open);
    assert!(app.resume_picker_pending.is_none());
    assert_eq!(app.state.draft, "Still here");
}

#[test]
fn shell_resume_commits_only_matching_hydrated_destination() {
    let backend = ReplayBackend::from_json(r#"{"name":"handoff","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, events) = ChatController::fixture_with_events(app.state.generation);
    app.controller = controller;
    app.shell_host = true;
    let old = ThreadId("old".into());
    let destination = ThreadId("destination".into());
    app.state.selected_thread = Some(old.clone());
    app.state.draft = "Discard after success".into();
    app.pending_initial_resume = Some(destination.clone());
    app.shell_writer_thread = Some(destination.clone());
    app.confirmed_thread_discard = Some(destination.clone());
    let thread = |id: ThreadId| nickel_codex::Thread {
        id,
        title: None,
        cwd: None,
        last_used_at: None,
        turns: Vec::new(),
        model: None,
        reasoning_effort: None,
    };
    events
        .send((
            app.state.generation,
            ControllerEvent::ThreadSelected(thread(old.clone())),
        ))
        .unwrap();
    app.poll_controller();
    assert_eq!(app.state.selected_thread, Some(old));
    assert_eq!(app.state.draft, "Discard after success");
    assert!(app.take_shell_requests().is_empty());

    events
        .send((
            app.state.generation,
            ControllerEvent::ThreadSelected(thread(destination.clone())),
        ))
        .unwrap();
    app.poll_controller();
    assert_eq!(app.state.selected_thread, Some(destination.clone()));
    assert!(app.state.draft.is_empty());
    assert_eq!(
        app.take_shell_requests(),
        vec![ShellRequest::ResumeSucceeded(destination)]
    );
}

#[test]
fn failed_shell_resume_restores_previous_writer_tracking() {
    let backend = ReplayBackend::from_json(r#"{"name":"failed-handoff","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, events) = ChatController::fixture_with_events(app.state.generation);
    app.controller = controller;
    app.shell_host = true;
    let old = ThreadId("old".into());
    let destination = ThreadId("destination".into());
    app.shell_writer_thread = Some(destination.clone());
    app.pending_previous_writer = Some(old.clone());
    app.pending_initial_resume = Some(destination.clone());
    app.state.selected_thread = Some(old.clone());
    app.state.draft = "Keep this".into();
    events
        .send((
            app.state.generation,
            ControllerEvent::OperationFailed("no writer".into()),
        ))
        .unwrap();
    app.poll_controller();
    assert_eq!(app.shell_writer_thread, Some(old.clone()));
    assert!(app.pending_previous_writer.is_none());
    assert_eq!(app.state.selected_thread, Some(old));
    assert_eq!(app.state.draft, "Keep this");
    assert_eq!(
        app.take_shell_requests(),
        vec![ShellRequest::ResumeFailed(destination)]
    );
}

#[test]
fn failed_recovery_stays_gated_and_offers_reconnect() {
    let backend = ReplayBackend::from_json(r#"{"name":"recovery","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, events) = ChatController::fixture_with_events(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.draft = "Do not replay".into();
    app.state.recovery_pending = true;
    app.recovery_thread = Some(ThreadId("old".into()));
    events
        .send((
            app.state.generation,
            ControllerEvent::OperationFailed("reload rejected".into()),
        ))
        .unwrap();
    app.poll_controller();
    assert!(app.state.recovery_pending);
    assert!(app.state.recovery_failed);
    assert!(!app.state.can_send());
    assert_eq!(app.state.draft, "Do not replay");
    let frame = UiFrame::layout(
        diagnostics_panel(&app.state, false, None, semantic_theme()),
        Rect::new(0.0, 0.0, 640.0, 480.0),
    );
    assert!(has_accessible_text(&frame, "Conversation reload failed"));
    assert!(has_accessible_text(&frame, "Reconnect"));
}

#[test]
fn reconnect_preserves_draft_and_fences_previous_turn() {
    let backend = ReplayBackend::from_json(r#"{"name":"reconnect","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.state.status = ConnectionStatus::Disconnected;
    app.state.selected_thread = Some(ThreadId("prior".into()));
    app.state.active_turn = Some(nickel_codex::TurnId("unfinished".into()));
    app.state.pending.push(PendingInteraction::Approval {
        request_id: ServerRequestId("old-approval".into()),
        approval_type: "command".into(),
        summary: "Old request".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    app.state.draft = "Unsent prompt".into();
    app.state.attach_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
    let generation = app.state.generation;
    app.reconnect_controller();
    assert_eq!(app.state.generation, generation + 1);
    assert_eq!(app.state.status, ConnectionStatus::Loading);
    assert_eq!(app.recovery_thread, Some(ThreadId("prior".into())));
    assert!(app.state.recovery_pending);
    assert!(!app.state.can_send());
    assert_eq!(app.state.draft, "Unsent prompt");
    assert_eq!(app.state.attachments.len(), 1);
    assert!(app.state.active_turn.is_none());
    assert!(app.state.pending.is_empty());
    assert!(
        app.state
            .diagnostics
            .iter()
            .any(|message| message.contains("unconfirmed"))
    );
}

#[test]
fn matching_recovery_hydration_reopens_send_without_replaying_draft() {
    let backend = ReplayBackend::from_json(r#"{"name":"hydration","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, events) = ChatController::fixture_with_events(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.draft = "Still unsent".into();
    app.state.attach_rgba(1, 1, &[0, 255, 0, 255]).unwrap();
    app.state.recovery_pending = true;
    app.state.unconfirmed_work = true;
    let id = ThreadId("reloaded".into());
    app.recovery_thread = Some(id.clone());
    events
        .send((
            app.state.generation,
            ControllerEvent::ThreadSelected(nickel_codex::Thread {
                id: id.clone(),
                title: None,
                cwd: None,
                last_used_at: None,
                turns: Vec::new(),
                model: None,
                reasoning_effort: None,
            }),
        ))
        .unwrap();
    app.poll_controller();
    assert_eq!(app.state.selected_thread, Some(id));
    assert!(!app.state.recovery_pending);
    assert!(!app.state.recovery_failed);
    assert!(!app.state.unconfirmed_work);
    assert!(app.state.can_send());
    assert_eq!(app.state.draft, "Still unsent");
    assert_eq!(app.state.attachments.len(), 1);
    assert!(app.take_shell_requests().is_empty());
}

#[test]
fn empty_project_recovery_waits_for_configuration_acknowledgment() {
    let backend = ReplayBackend::from_json(r#"{"name":"project-ack","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let (controller, events) = ChatController::fixture_with_events(app.state.generation);
    app.controller = controller;
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.draft = "Unsent".into();
    app.state.recovery_pending = true;
    app.recovery_project_configuration_pending = true;
    assert!(!app.state.can_send());
    events
        .send((app.state.generation, ControllerEvent::ProjectConfigured))
        .unwrap();
    app.poll_controller();
    assert!(!app.state.recovery_pending);
    assert!(app.state.can_send());
    assert_eq!(app.state.draft, "Unsent");
}

#[test]
fn selecting_a_persisted_remote_host_replaces_local_project_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("codex-hosts.toml");
    let host = RemoteHost {
        id: "arm_host".into(),
        name: "ARM host".into(),
        endpoint: "ws://127.0.0.1:9/app-server".into(),
        token_env: None,
        default_cwd: "/srv/nickel".into(),
    };
    let mut settings = CodexSettings::default();
    settings.hosts.push(host.clone());
    settings.save(&path).unwrap();
    let backend = ReplayBackend::from_json(r#"{"name":"switch","events":[]}"#).unwrap();
    let mut app = ChatApplication::with_settings(
        BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        },
        settings,
        Some(path.clone()),
    );
    app.state.threads.push(Thread {
        id: ThreadId("local-thread".into()),
        title: Some("Local thread".into()),
        cwd: Some("/projects/local".into()),
        last_used_at: Some(1),
        turns: Vec::new(),
        model: None,
        reasoning_effort: None,
    });

    app.update(ChatMessage::SelectConnection("arm_host".into()));

    assert!(app.state.threads.is_empty());
    assert!(matches!(
        &app.mode,
        BackendMode::Remote { host } if host.id == "arm_host"
    ));
    assert_eq!(CodexSettings::load(&path).unwrap().selected, "arm_host");
}

#[test]
fn invalid_host_edits_do_not_replace_last_known_good_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("codex-hosts.toml");
    let settings = CodexSettings::default();
    settings.save(&path).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();
    let backend = ReplayBackend::from_json(r#"{"name":"invalid","events":[]}"#).unwrap();
    let mut app = ChatApplication::with_settings(
        BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        },
        settings,
        Some(path.clone()),
    );

    app.update(ChatMessage::ManageRemoteHosts);
    app.update(ChatMessage::AddRemoteHost);
    app.update(ChatMessage::RemoteHostIdChanged("bad host".into()));
    app.update(ChatMessage::SaveRemoteHost);

    assert!(app.settings_error.is_some());
    assert!(app.settings.hosts.is_empty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

fn active_turn_app() -> ChatApplication {
    let backend = ReplayBackend::from_json(r#"{"name":"queue","events":[]}"#).unwrap();
    let mut app = ChatApplication::new(BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    app.controller = ChatController::fixture_idle(app.state.generation);
    app.state.status = ConnectionStatus::Ready;
    app.state.account.authenticated = true;
    app.state.selected_thread = Some(ThreadId("thread".into()));
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 1,
            kind: nickel_codex::EventKind::TurnStarted {
                thread_id: ThreadId("thread".into()),
                turn_id: nickel_codex::TurnId("turn".into()),
            },
        }),
    );
    app
}

#[test]
fn queued_message_is_bounded_and_rejection_preserves_the_draft() {
    let mut app = active_turn_app();
    for index in 0..QUEUED_MESSAGE_CAPACITY {
        app.state.draft = format!("queued {index}");
        app.update(ChatMessage::QueueMessage);
    }
    assert_eq!(app.queued_messages.len(), QUEUED_MESSAGE_CAPACITY);
    app.state.draft = "keep this draft".into();
    app.update(ChatMessage::QueueMessage);
    assert_eq!(app.state.draft, "keep this draft");
    assert_eq!(app.queued_messages.len(), QUEUED_MESSAGE_CAPACITY);
}

#[test]
fn queued_message_dispatch_uses_captured_image_and_run_settings() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        1,
        1,
        image::Rgba([20, 40, 60, 255]),
    ))
    .write_to(&mut encoded, image::ImageFormat::Png)
    .unwrap();
    app.state.attach_image(encoded.get_ref()).unwrap();
    app.state.draft = "captured payload".into();
    app.state.selected_model = Some("original-model".into());
    app.state.selected_reasoning_effort = Some("high".into());
    app.state.selected_approval_policy = ApprovalPolicy::Never;
    app.state.selected_sandbox_policy = Some(SandboxPolicy::DangerFullAccess);
    app.plan_mode = true;

    app.update(ChatMessage::QueueMessage);
    assert!(commands.try_recv().is_err(), "queueing is not sending");
    assert!(app.state.attachments.is_empty());
    assert!(app.state.draft.is_empty());
    app.state.draft = "next draft".into();
    app.state.selected_model = Some("later-model".into());
    app.state.selected_reasoning_effort = Some("low".into());
    app.state.selected_approval_policy = ApprovalPolicy::OnRequest;
    app.state.selected_sandbox_policy = Some(SandboxPolicy::ReadOnly);
    app.plan_mode = false;
    app.state.active_turn = None;

    app.dispatch_queued_after_boundary();
    let Ok(ControllerCommand::Send {
        text,
        images,
        model,
        reasoning_effort,
        approval_policy,
        sandbox_policy,
        plan_mode,
    }) = commands.try_recv()
    else {
        panic!("expected one queued send");
    };
    assert_eq!(text, "captured payload");
    assert_eq!(images.len(), 1);
    assert!(images[0].data_url.starts_with("data:image/png;base64,"));
    assert_eq!(model.as_deref(), Some("original-model"));
    assert_eq!(reasoning_effort.as_deref(), Some("high"));
    assert_eq!(approval_policy, ApprovalPolicy::Never);
    assert_eq!(sandbox_policy, Some(SandboxPolicy::DangerFullAccess));
    assert!(plan_mode);
    assert_eq!(app.state.draft, "next draft");
}

#[test]
fn queued_message_dispatches_once_only_after_matching_boundary() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "follow up".into();
    app.update(ChatMessage::QueueMessage);
    assert!(
        commands.try_recv().is_err(),
        "queueing must not start a turn"
    );

    app.update(ChatMessage::InterruptAndSend);
    assert!(app.controller.fixture_interrupt_requested());
    app.state.apply(
        app.state.generation,
        ControllerEvent::Protocol(nickel_codex::CodexEvent {
            sequence: 2,
            kind: nickel_codex::EventKind::TurnCompleted {
                thread_id: ThreadId("thread".into()),
                turn_id: nickel_codex::TurnId("turn".into()),
                status: "interrupted".into(),
            },
        }),
    );
    app.dispatch_queued_after_boundary();
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::Send { ref text, .. }) if text == "follow up"
    ));
    app.dispatch_queued_after_boundary();
    assert!(
        commands.try_recv().is_err(),
        "a boundary cannot double-send"
    );
}

#[test]
fn timed_out_interrupt_never_sends_the_queued_message() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "do not send without a boundary".into();
    app.update(ChatMessage::QueueMessage);
    app.update(ChatMessage::InterruptAndSend);
    app.queued_interrupt_deadline = Some(std::time::Instant::now());

    assert!(app.poll_controller());
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::InterruptTimedOut)
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn conversation_change_preserves_queue_for_explicit_review() {
    let mut app = active_turn_app();
    app.state.draft = "keep this for the old conversation".into();
    app.update(ChatMessage::QueueMessage);

    app.mark_queued_messages_for_review("changing conversation");

    let queued = app.queued_messages.front().unwrap();
    assert_eq!(queued.phase, QueuedMessagePhase::Unconfirmed);
    assert_eq!(queued.thread_id, ThreadId("thread".into()));
    assert_eq!(queued.text, "keep this for the old conversation");
}

#[test]
fn conversation_change_keeps_uncertain_dispatched_message_for_review() {
    let mut app = active_turn_app();
    app.state.draft = "delivery may already be in flight".into();
    app.update(ChatMessage::QueueMessage);
    app.queued_messages.front_mut().unwrap().phase = QueuedMessagePhase::Dispatching;

    app.mark_queued_messages_for_review("changing conversation");

    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Unconfirmed)
    );
    assert_eq!(
        app.queued_messages
            .front()
            .map(|queued| queued.text.as_str()),
        Some("delivery may already be in flight")
    );
}

#[test]
fn saving_an_unconfirmed_message_does_not_retry_until_explicit_action() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "possibly delivered".into();
    app.update(ChatMessage::QueueMessage);
    let id = app.queued_messages.front().unwrap().id;
    app.mark_queued_messages_for_review("connection loss");
    app.state.active_turn = None;

    app.update(ChatMessage::EditQueuedMessage(id));
    app.state.draft = "reviewed text".into();
    app.update(ChatMessage::QueueMessage);
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Unconfirmed)
    );
    assert!(commands.try_recv().is_err(), "saving is not a retry");

    app.update(ChatMessage::RetryQueuedMessage(id));
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::Send { ref text, .. }) if text == "reviewed text"
    ));
    app.update(ChatMessage::RetryQueuedMessage(id));
    assert!(commands.try_recv().is_err(), "retry cannot double-send");
}

#[test]
fn retry_rejects_a_different_conversation_without_sending() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "old conversation".into();
    app.update(ChatMessage::QueueMessage);
    let id = app.queued_messages.front().unwrap().id;
    app.mark_queued_messages_for_review("changing conversation");
    app.state.active_turn = None;
    app.state.selected_thread = Some(ThreadId("other".into()));

    app.update(ChatMessage::RetryQueuedMessage(id));
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Unconfirmed)
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn stale_generation_never_dispatches_a_queued_message() {
    let mut app = active_turn_app();
    let (controller, commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "generation-bound".into();
    app.update(ChatMessage::QueueMessage);
    app.state.active_turn = None;
    app.state.generation = app.state.generation.saturating_add(1);

    app.dispatch_queued_after_boundary();

    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Unconfirmed)
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn repeated_interrupt_and_send_emits_only_one_interrupt() {
    let mut app = active_turn_app();
    let (controller, _commands) = ChatController::fixture_with_commands(app.state.generation);
    app.controller = controller;
    app.state.draft = "one interrupt".into();
    app.update(ChatMessage::QueueMessage);

    app.update(ChatMessage::InterruptAndSend);
    let first_deadline = app.queued_interrupt_deadline;
    app.update(ChatMessage::InterruptAndSend);

    assert!(app.controller.fixture_interrupt_requested());
    assert_eq!(app.queued_interrupt_deadline, first_deadline);
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::InterruptRequested)
    );
}

#[test]
fn natural_completion_racing_interrupt_acknowledgement_sends_once() {
    let mut app = active_turn_app();
    let (controller, commands, events) =
        ChatController::fixture_with_commands_and_events(app.state.generation);
    app.controller = controller;
    app.state.draft = "send after the natural boundary".into();
    app.update(ChatMessage::QueueMessage);
    app.update(ChatMessage::InterruptAndSend);

    events
        .send((
            app.state.generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 2,
                kind: nickel_codex::EventKind::TurnCompleted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: nickel_codex::TurnId("turn".into()),
                    status: "completed".into(),
                },
            }),
        ))
        .unwrap();
    assert!(app.poll_controller());
    assert!(matches!(
        commands.try_recv(),
        Ok(ControllerCommand::Send { ref text, .. })
            if text == "send after the natural boundary"
    ));

    events
        .send((app.state.generation, ControllerEvent::InterruptAccepted))
        .unwrap();
    app.poll_controller();
    assert!(commands.try_recv().is_err());
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Dispatching)
    );
}

#[test]
fn rejected_interrupt_keeps_the_queued_message_without_sending() {
    let mut app = active_turn_app();
    let (controller, commands, events) =
        ChatController::fixture_with_commands_and_events(app.state.generation);
    app.controller = controller;
    app.state.draft = "wait for review".into();
    app.update(ChatMessage::QueueMessage);
    app.update(ChatMessage::InterruptAndSend);

    events
        .send((
            app.state.generation,
            ControllerEvent::InterruptFailed("backend rejected interrupt".into()),
        ))
        .unwrap();
    assert!(app.poll_controller());
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Waiting)
    );
    assert_eq!(app.queued_interrupt_deadline, None);
    assert!(!app.state.interrupt_requested);
    assert!(commands.try_recv().is_err());
}

#[test]
fn disconnect_during_interrupt_requires_review_and_ignores_late_boundary() {
    let mut app = active_turn_app();
    let (controller, commands, events) =
        ChatController::fixture_with_commands_and_events(app.state.generation);
    app.controller = controller;
    app.state.draft = "preserve on disconnect".into();
    app.update(ChatMessage::QueueMessage);
    app.update(ChatMessage::InterruptAndSend);
    let generation = app.state.generation;

    events
        .send((
            generation,
            ControllerEvent::Failure("connection closed".into()),
        ))
        .unwrap();
    assert!(app.poll_controller());
    assert_eq!(app.state.status, ConnectionStatus::Disconnected);
    assert_eq!(app.queued_interrupt_deadline, None);
    assert_eq!(
        app.queued_messages.front().map(|queued| queued.phase),
        Some(QueuedMessagePhase::Unconfirmed)
    );
    assert_eq!(
        app.queued_messages
            .front()
            .map(|queued| queued.text.as_str()),
        Some("preserve on disconnect")
    );

    events
        .send((
            generation,
            ControllerEvent::Protocol(nickel_codex::CodexEvent {
                sequence: 2,
                kind: nickel_codex::EventKind::TurnCompleted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: nickel_codex::TurnId("turn".into()),
                    status: "interrupted".into(),
                },
            }),
        ))
        .unwrap();
    app.poll_controller();
    assert!(commands.try_recv().is_err());
}
