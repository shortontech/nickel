use std::{hint::black_box, mem::size_of, time::Instant};

use super::*;

#[test]
fn new_chat_commits_only_after_controller_acknowledgement() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("current".into())),
        draft: "unsent work".into(),
        ..Default::default()
    };
    state.items.push_back(ChatItem {
        id: "current-answer".into(),
        kind: ChatItemKind::Agent,
        text: "Existing transcript".into(),
        complete: true,
    });

    state.apply(
        1,
        ControllerEvent::NewChatFailed("Workspace unavailable".into()),
    );
    assert_eq!(state.selected_thread, Some(ThreadId("current".into())));
    assert_eq!(state.draft, "unsent work");
    assert_eq!(state.items[0].text, "Existing transcript");

    state.apply(1, ControllerEvent::NewChatPrepared);
    assert_eq!(state.selected_thread, None);
    assert!(state.draft.is_empty());
    assert!(state.items.is_empty());
}

#[test]
fn presentation_status_keeps_transport_failure_ahead_of_pending_work() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Ready
    );
    state.send_pending = true;
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Starting
    );
    state.send_pending = false;
    state.active_turn = Some(TurnId("turn".into()));
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Working
    );
    state.pending.push(PendingInteraction::Approval {
        request_id: ServerRequestId("approval".into()),
        approval_type: "command".into(),
        summary: "Run a command".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::AwaitingApproval
    );
    state.apply(1, ControllerEvent::Failure("transport closed".into()));
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Disconnected
    );
    assert_eq!(
        state.pending.len(),
        1,
        "transport loss must not erase authority facts"
    );
    assert!(state.run_secondary_status().is_some());
    state.status = ConnectionStatus::Ready;
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Unconfirmed
    );
    assert!(!state.can_send());
    state.new_chat();
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Ready
    );
}

#[test]
fn healthy_unauthenticated_transport_requires_sign_in_before_actions() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        draft: "not yet sent".into(),
        ..Default::default()
    };
    let request_id = ServerRequestId("approval".into());
    state.pending.push(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: "command".into(),
        summary: "Do something".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::AuthenticationRequired
    );
    assert!(!state.can_send());
    assert!(!state.interaction_is_actionable(&request_id));
    state.account.authenticated = true;
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::AwaitingApproval
    );
    assert!(state.interaction_is_actionable(&request_id));
}

#[test]
fn pending_response_is_fenced_until_matching_resolution_or_reconnect() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    state.selected_thread = Some(ThreadId("thread-a".into()));
    let request_id = ServerRequestId("request-1".into());
    let kind = "item/commandExecution/requestApproval";
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: kind.into(),
        summary: "Run tests".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(!state.begin_approval_response(&request_id, "wrong-kind"));
    assert!(state.begin_approval_response(&request_id, kind));
    assert!(!state.begin_approval_response(&request_id, kind));
    assert_eq!(state.pending.len(), 1, "dispatch is not resolution");
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("other-thread".into()),
            request_id: request_id.clone(),
        },
    });
    assert_eq!(
        state.pending.len(),
        1,
        "another thread cannot retire the request"
    );
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("thread-a".into()),
            request_id: request_id.clone(),
        },
    });
    assert!(state.pending.is_empty());
    assert!(!state.interaction_submission_pending(&request_id));
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: kind.into(),
        summary: "Different request after resolution".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(state.interaction_is_actionable(&request_id));
    assert_eq!(state.invalidate_pending_for_reconnect(), 1);
    assert!(!state.interaction_is_actionable(&request_id));
    assert!(
        state.items.is_empty(),
        "approvals have no transcript duplicate"
    );
}

#[test]
fn interaction_references_preserve_chronology_without_owning_decisions() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread-a".into())),
        ..Default::default()
    };
    let approval_id = ServerRequestId("approval-1".into());
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::ApprovalRequested {
            request_id: approval_id.clone(),
            thread_id: Some(ThreadId("thread-a".into())),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: Some("Run cargo test".into()),
            context: nickel_codex::ApprovalContext::default(),
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: EventKind::ItemStarted {
            thread_id: Some(ThreadId("thread-a".into())),
            turn_id: Some(TurnId("turn-a".into())),
            item_id: "agent-1".into(),
            item_type: "agentMessage".into(),
            command_actions: Vec::new(),
            initial_text: "Continuing".into(),
        },
    });
    let question_id = ServerRequestId("question-1".into());
    state.apply_protocol(CodexEvent {
        sequence: 3,
        kind: EventKind::UserInputRequested {
            request_id: question_id.clone(),
            question_ids: vec!["choice".into()],
            questions: vec![nickel_codex::UserInputQuestion {
                id: "choice".into(),
                header: "Choice".into(),
                question: "Which option?".into(),
                options: Vec::new(),
                is_other: false,
                is_secret: false,
            }],
        },
    });
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[0].id, "agent-1");
    assert_eq!(state.items[1].kind, ChatItemKind::QuestionReference);
    assert!(state.items[1].text.contains("Which option?"));
    assert!(!state.items[1].text.contains("choice"));

    state.apply_protocol(CodexEvent {
        sequence: 4,
        kind: EventKind::ApprovalRequested {
            request_id: approval_id.clone(),
            thread_id: Some(ThreadId("thread-a".into())),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: Some("Run cargo test with network".into()),
            context: nickel_codex::ApprovalContext::default(),
        },
    });
    assert_eq!(state.items.len(), 2);
    assert!(
        matches!(&state.pending[0], PendingInteraction::Approval { summary, .. } if summary.contains("with network"))
    );
    state.apply_protocol(CodexEvent {
        sequence: 5,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("thread-a".into()),
            request_id: approval_id,
        },
    });
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.pending.len(), 1);
    assert!(matches!(
        &state.pending[0],
        PendingInteraction::UserInput { request_id, .. } if request_id == &question_id
    ));
}

#[test]
fn scoped_warnings_are_distinct_cards_and_global_warnings_stay_diagnostic() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread-a".into())),
        ..Default::default()
    };
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::Warning {
            thread_id: Some(ThreadId("thread-a".into())),
            message: "Approval policy changed".into(),
            guardian: true,
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: EventKind::Warning {
            thread_id: None,
            message: "Deprecated setting".into(),
            guardian: false,
        },
    });
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].kind, ChatItemKind::Warning);
    assert!(state.items[0].text.starts_with("Security warning"));
    assert!(
        state
            .diagnostics
            .back()
            .unwrap()
            .contains("Deprecated setting")
    );
}

#[test]
fn resolution_uses_request_thread_not_current_selection() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    let request_id = ServerRequestId("request-1".into());
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::ApprovalRequested {
            request_id: request_id.clone(),
            thread_id: Some(ThreadId("owning-thread".into())),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: Some("Run tests".into()),
            context: nickel_codex::ApprovalContext::default(),
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("different-thread".into()),
            request_id: request_id.clone(),
        },
    });
    assert_eq!(state.pending.len(), 1);
    state.apply_protocol(CodexEvent {
        sequence: 3,
        kind: EventKind::ApprovalRequested {
            request_id: request_id.clone(),
            thread_id: Some(ThreadId("different-thread".into())),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: Some("Run tests".into()),
            context: nickel_codex::ApprovalContext::default(),
        },
    });
    assert_eq!(state.status, ConnectionStatus::Disconnected);
    assert_eq!(state.pending.len(), 1);
    state.apply_protocol(CodexEvent {
        sequence: 4,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("owning-thread".into()),
            request_id,
        },
    });
    assert!(state.pending.is_empty());
}

#[test]
fn changed_scope_during_submission_requires_recovery() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    let request_id = ServerRequestId("request-1".into());
    let kind = "item/fileChange/requestApproval";
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: kind.into(),
        summary: "Write A".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(state.begin_approval_response(&request_id, kind));
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: kind.into(),
        summary: "Write B".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert_eq!(state.status, ConnectionStatus::Disconnected);
    assert!(!state.interaction_is_actionable(&request_id));
}

#[test]
fn failed_response_stays_visible_and_cannot_be_replayed() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    let request_id = ServerRequestId("request-1".into());
    let kind = "item/commandExecution/requestApproval";
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: kind.into(),
        summary: "Run command".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(state.begin_approval_response(&request_id, kind));
    state.apply(
        1,
        ControllerEvent::InteractionResponseFailed {
            request_id: request_id.clone(),
            message: "socket closed".into(),
        },
    );
    assert_eq!(state.pending.len(), 1);
    assert!(state.interaction_response_unconfirmed(&request_id));
    assert!(!state.begin_approval_response(&request_id, kind));
    assert_eq!(state.invalidate_pending_for_reconnect(), 1);
    assert!(!state.interaction_response_unconfirmed(&request_id));
}

#[test]
fn oversized_interaction_metadata_cannot_enter_pending_authority() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::ApprovalRequested {
                request_id: ServerRequestId("request".into()),
                thread_id: Some(ThreadId("thread".into())),
                approval_type: "item/commandExecution/requestApproval".into(),
                summary: Some("x".repeat(4097)),
                context: nickel_codex::ApprovalContext::default(),
            },
        }),
    );
    assert!(state.pending.is_empty());
    assert_eq!(state.status, ConnectionStatus::Disconnected);
}

#[test]
fn login_completion_only_consumes_the_matching_challenge() {
    let mut state = ChatState::default();
    state.apply(
        1,
        ControllerEvent::LoginStarted(LoginChallenge::DeviceCode {
            login_id: "current".into(),
            user_code: "ABCD-EFGH".into(),
            verification_url: "https://example.test/device".into(),
        }),
    );
    assert!(state.login_qr.is_some());
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::AccountLoginCompleted {
                completion: nickel_codex::LoginCompletion {
                    login_id: Some("stale".into()),
                    success: true,
                    error: None,
                },
            },
        }),
    );
    assert!(!state.account.authenticated);
    assert!(state.login_challenge.is_some());
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::AccountLoginCompleted {
                completion: nickel_codex::LoginCompletion {
                    login_id: Some("current".into()),
                    success: true,
                    error: None,
                },
            },
        }),
    );
    assert!(!state.account.authenticated);
    assert!(state.login_challenge.is_none());
    assert!(state.login_qr.is_none());
}

#[test]
fn codex_phone_pairing_secret_is_cleared_on_cancel_failure_and_disconnect() {
    let challenge = || nickel_codex::RemotePairingChallenge {
        environment_id: "environment-1".into(),
        expires_at: 42,
        pairing_code: "opaque-secret-payload".into(),
        manual_pairing_code: Some("1234".into()),
    };
    let mut state = ChatState::default();
    state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
    assert!(state.remote_pairing.is_some());
    assert!(state.remote_pairing_qr.is_some());

    state.apply(1, ControllerEvent::RemotePairingCancelled);
    assert!(state.remote_pairing.is_none());
    assert!(state.remote_pairing_qr.is_none());

    state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
    state.apply(1, ControllerEvent::RemotePairingFailed("expired".into()));
    assert!(state.remote_pairing.is_none());
    assert!(state.remote_pairing_qr.is_none());

    state.apply(1, ControllerEvent::RemotePairingStarted(challenge()));
    state.apply(
        1,
        ControllerEvent::RemoteControlStatus(nickel_codex::RemoteControlStatus {
            status: nickel_codex::RemoteControlConnectionStatus::Disabled,
            server_name: "workstation".into(),
            installation_id: "installation-1".into(),
            environment_id: None,
        }),
    );
    assert!(state.remote_pairing.is_none());
    assert!(state.remote_pairing_qr.is_none());
}

#[test]
fn phone_pairing_qr_uses_url_with_opaque_code() {
    assert_eq!(
        phone_pairing_url("abc123").as_deref(),
        Some("https://chatgpt.com/codex/pair?pairing_code=abc123")
    );
    assert_eq!(
        phone_pairing_url("a+b/c").as_deref(),
        Some("https://chatgpt.com/codex/pair?pairing_code=a%2Bb%2Fc")
    );
    assert!(phone_pairing_url("").is_none());
}

#[test]
fn paired_client_list_replaces_transient_pairing_success_message() {
    let mut state = ChatState {
        remote_control_message: Some("Phone paired".into()),
        ..ChatState::default()
    };
    state.apply(
        1,
        ControllerEvent::RemoteClients(nickel_codex::RemoteControlClientPage {
            data: vec![nickel_codex::RemoteControlClient {
                client_id: "phone-1".into(),
                display_name: Some("iPhone".into()),
                device_model: None,
                device_type: None,
                platform: None,
                os_version: None,
                app_version: None,
                last_seen_at: None,
            }],
            next_cursor: None,
        }),
    );
    assert!(state.remote_control_message.is_none());
    assert_eq!(state.remote_clients.len(), 1);
}

#[test]
fn confirmed_account_snapshot_not_completion_establishes_authentication() {
    let mut state = ChatState::default();
    state.apply(
        1,
        ControllerEvent::LoginStarted(LoginChallenge::Browser {
            login_id: "current".into(),
            auth_url: "https://example.test/login".into(),
        }),
    );
    state.apply(
        1,
        ControllerEvent::Ready {
            provenance: "test".into(),
            account: AccountState {
                authenticated: true,
                ..Default::default()
            },
            models: Vec::new(),
            projects: Vec::new(),
            threads: Vec::new(),
            runtime: HashMap::new(),
            thread_error: None,
            thread_next_cursor: None,
        },
    );
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::AccountLoginCompleted {
                completion: nickel_codex::LoginCompletion {
                    login_id: Some("current".into()),
                    success: true,
                    error: None,
                },
            },
        }),
    );
    assert!(state.account.authenticated);
    assert!(state.login_challenge.is_none());
}

fn delta(state: &mut ChatState, text: &str) {
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::AgentMessageDelta {
                item_id: "stream".into(),
                delta: text.into(),
            },
        }),
    );
}

#[test]
fn unicode_stream_truncation_is_visible_bounded_and_terminal() {
    let mut state = ChatState::default();
    delta(&mut state, &"界".repeat(MAX_ITEM_TEXT_BYTES));
    assert!(state.items[0].text.ends_with(OMISSION_MARKER));
    assert!(state.items[0].text.capacity() <= MAX_ITEM_TEXT_BYTES);
    let retained = state.items[0].text.clone();
    delta(&mut state, "late text");
    assert_eq!(state.items[0].text, retained);
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 2,
            kind: EventKind::ItemCompleted {
                item_id: "stream".into(),
                completion: None,
            },
        }),
    );
    assert!(state.items[0].complete);
    assert!(
        state
            .transcript_selection_document()
            .runs()
            .iter()
            .any(|run| run.text.contains("Further output omitted"))
    );
}

#[test]
fn ordinary_output_can_quote_the_local_omission_marker() {
    let mut text = OMISSION_MARKER.to_owned();
    append_bounded(&mut text, " followed by more output");
    assert!(text.ends_with(" followed by more output"));
}

#[test]
fn streaming_rebuilds_only_consumed_items_once_per_batch() {
    let text = "## Heading\n\n**Unicode 世界** and `incomplete";
    let mut small = ChatState::default();
    small.push_item(ChatItem {
        id: "offscreen".into(),
        kind: ChatItemKind::Agent,
        text: "untouched".into(),
        complete: true,
    });
    for character in text.chars() {
        delta(&mut small, &character.to_string());
    }
    assert!(
        small
            .item_selection_runs
            .iter()
            .all(|projection| projection.builds.get() == 0)
    );
    let rendered = small.markdown_document(1);
    assert!(Arc::ptr_eq(&rendered, &small.markdown_document(1)));
    assert_eq!(small.item_selection_runs[1].builds.get(), 1);
    assert_eq!(small.item_selection_runs[0].builds.get(), 0);
    // The full selection document is an explicit consumer of offscreen logical text.
    small.transcript_selection_document().runs();
    assert_eq!(small.item_selection_runs[1].builds.get(), 1);
    assert_eq!(small.item_selection_runs[0].builds.get(), 1);
    let mut big = ChatState::default();
    delta(&mut big, text);
    assert_eq!(*rendered, *big.markdown_document(0));
    assert_eq!(
        small.item_selection_runs[1]
            .cached
            .borrow()
            .as_ref()
            .unwrap()
            .runs,
        big.item_selection_runs[0]
            .cached
            .borrow()
            .as_ref()
            .unwrap()
            .runs
    );
    delta(&mut small, "` end");
    assert_eq!(small.item_selection_runs[1].builds.get(), 1);
    assert!(!Arc::ptr_eq(&rendered, &small.markdown_document(1)));
    assert_eq!(small.item_selection_runs[1].builds.get(), 2);
}

#[test]
fn adjacent_completed_agent_items_preserve_started_text_and_aliases() {
    let mut state = ChatState::default();
    let turn_id = TurnId("turn".into());
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnStarted {
                thread_id: ThreadId("thread".into()),
                turn_id: turn_id.clone(),
            },
        }),
    );
    for (item_id, initial_text) in [("first", "first text"), ("second", "世界 second")] {
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 2,
                kind: EventKind::ItemStarted {
                    thread_id: None,
                    turn_id: Some(turn_id.clone()),
                    item_id: item_id.into(),
                    item_type: "agentMessage".into(),
                    command_actions: Vec::new(),
                    initial_text: initial_text.into(),
                },
            }),
        );
        state.apply(
            1,
            ControllerEvent::Protocol(CodexEvent {
                sequence: 3,
                kind: EventKind::ItemCompleted {
                    item_id: item_id.into(),
                    completion: None,
                },
            }),
        );
    }
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "first text\n\n世界 second");
    assert!(state.items[0].complete);
    assert_eq!(state.resolve_item_index("second"), Some(0));
}

#[test]
fn merged_agent_final_snapshot_replaces_only_its_own_segment() {
    let mut state = ChatState::default();
    let turn_id = TurnId("turn".into());
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::TurnStarted {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
        },
    });
    let complete = |id: &str, text: &str| EventKind::ItemCompleted {
        item_id: id.into(),
        completion: Some(nickel_codex::CompletedItem {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
            completed_at_ms: Some(42),
            item_type: "agentMessage".into(),
            text: text.into(),
            status: None,
            exit_code: None,
            duration_ms: None,
            changes: Vec::new(),
            summary_parts: Vec::new(),
            command_actions: Vec::new(),
        }),
    };
    for (index, id) in ["first", "second"].into_iter().enumerate() {
        state.apply_protocol(CodexEvent {
            sequence: (index * 3 + 2) as u64,
            kind: EventKind::ItemStarted {
                thread_id: Some(ThreadId("thread".into())),
                turn_id: Some(turn_id.clone()),
                item_id: id.into(),
                item_type: "agentMessage".into(),
                command_actions: Vec::new(),
                initial_text: String::new(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: (index * 3 + 3) as u64,
            kind: EventKind::AgentMessageDelta {
                item_id: id.into(),
                delta: "partial".into(),
            },
        });
        state.apply_protocol(CodexEvent {
            sequence: (index * 3 + 4) as u64,
            kind: complete(id, &format!("{id} final")),
        });
    }
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "first final\n\nsecond final");
    state.apply_protocol(CodexEvent {
        sequence: 9,
        kind: complete("first", "stale overwrite"),
    });
    state.apply_protocol(CodexEvent {
        sequence: 10,
        kind: EventKind::AgentMessageDelta {
            item_id: "first".into(),
            delta: "stale delta".into(),
        },
    });
    assert_eq!(state.items[0].text, "first final\n\nsecond final");
}

#[test]
fn assistant_items_do_not_merge_across_an_operation_live_or_resumed() {
    let mut live = ChatState::default();
    let turn_id = TurnId("turn".into());
    live.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::TurnStarted {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
        },
    });
    for (sequence, item_id, item_type, text) in [
        (2, "before", "agentMessage", "Before"),
        (4, "command", "commandExecution", "cargo test"),
        (6, "after", "agentMessage", "After"),
    ] {
        live.apply_protocol(CodexEvent {
            sequence,
            kind: EventKind::ItemStarted {
                thread_id: Some(ThreadId("thread".into())),
                turn_id: Some(turn_id.clone()),
                item_id: item_id.into(),
                item_type: item_type.into(),
                command_actions: Vec::new(),
                initial_text: text.into(),
            },
        });
        live.apply_protocol(CodexEvent {
            sequence: sequence + 1,
            kind: EventKind::ItemCompleted {
                item_id: item_id.into(),
                completion: None,
            },
        });
    }
    let expected = ["Before", "cargo test", "After"];
    assert_eq!(
        live.items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        expected
    );

    let mut resumed = ChatState::default();
    resumed.hydrate_thread(&Thread {
        id: ThreadId("thread".into()),
        title: None,
        cwd: None,
        last_used_at: None,
        turns: vec![nickel_codex::ThreadHistoryTurn {
            id: turn_id,
            status: "completed".into(),
            items: [
                ("before", "agentMessage", "Before"),
                ("command", "commandExecution", "cargo test"),
                ("after", "agentMessage", "After"),
            ]
            .into_iter()
            .map(|(id, item_type, text)| nickel_codex::ThreadHistoryItem {
                id: id.into(),
                item_type: item_type.into(),
                text: text.into(),
                command_actions: Vec::new(),
                status: (id == "command").then(|| "failed".into()),
                exit_code: (id == "command").then_some(2),
                duration_ms: (id == "command").then_some(17),
            })
            .collect(),
        }],
        model: None,
        reasoning_effort: None,
    });
    assert_eq!(
        resumed
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        resumed.activity_outcomes.get("command"),
        Some(&ActivityOutcome {
            status: Some("failed".into()),
            exit_code: Some(2),
            duration_ms: Some(17),
        })
    );
}

#[test]
fn structured_patch_and_turn_plan_snapshots_replace_in_place() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread".into())),
        ..Default::default()
    };
    let turn_id = TurnId("turn".into());
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::TurnStarted {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
        },
    });
    let patch = |diff: &str| EventKind::FilePatchUpdated {
        thread_id: ThreadId("thread".into()),
        turn_id: turn_id.clone(),
        item_id: "file".into(),
        changes: vec![nickel_codex::FilePatchChange {
            path: "/project/file".into(),
            kind: "update".into(),
            move_path: None,
            diff: diff.into(),
        }],
    };
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: patch("+first"),
    });
    state.apply_protocol(CodexEvent {
        sequence: 3,
        kind: EventKind::ItemStarted {
            thread_id: Some(ThreadId("thread".into())),
            turn_id: Some(turn_id.clone()),
            item_id: "file".into(),
            item_type: "fileChange".into(),
            command_actions: Vec::new(),
            initial_text: String::new(),
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 4,
        kind: patch("+second"),
    });
    assert_eq!(state.items.len(), 1);
    assert!(state.items[0].text.contains("+second"));
    assert!(!state.items[0].text.contains("+first"));
    assert_eq!(
        state.file_change_summaries.get("file").map(String::as_str),
        Some("update: /project/file")
    );
    state.apply_protocol(CodexEvent {
        sequence: 5,
        kind: EventKind::FilePatchUpdated {
            thread_id: ThreadId("thread".into()),
            turn_id: turn_id.clone(),
            item_id: "file".into(),
            changes: Vec::new(),
        },
    });
    assert!(!state.file_change_summaries.contains_key("file"));
    let plan = |status: &str| EventKind::TurnPlanUpdated {
        thread_id: ThreadId("thread".into()),
        turn_id: turn_id.clone(),
        explanation: None,
        steps: vec![nickel_codex::TurnPlanStep {
            step: "Test".into(),
            status: status.into(),
        }],
    };
    state.apply_protocol(CodexEvent {
        sequence: 6,
        kind: plan("pending"),
    });
    state.apply_protocol(CodexEvent {
        sequence: 7,
        kind: plan("completed"),
    });
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[1].kind, ChatItemKind::Plan);
    assert_eq!(state.items[1].text, "[completed] Test");
    state.remove_item(0);
    assert!(!state.file_change_summaries.contains_key("file"));
}

#[test]
fn completed_snapshot_replaces_deltas_and_restores_final_only_items() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread".into())),
        ..Default::default()
    };
    let complete = |id: &str, text: &str| EventKind::ItemCompleted {
        item_id: id.into(),
        completion: Some(nickel_codex::CompletedItem {
            thread_id: ThreadId("thread".into()),
            turn_id: TurnId("turn".into()),
            completed_at_ms: Some(42),
            item_type: "commandExecution".into(),
            text: text.into(),
            status: Some("completed".into()),
            exit_code: Some(0),
            duration_ms: Some(12),
            changes: Vec::new(),
            summary_parts: Vec::new(),
            command_actions: Vec::new(),
        }),
    };
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::CommandOutputDelta {
            item_id: "streamed".into(),
            delta: "partial".into(),
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: complete("streamed", "$ pwd\n/project"),
    });
    state.apply_protocol(CodexEvent {
        sequence: 3,
        kind: complete("final-only", "$ date\nToday"),
    });
    assert_eq!(state.items.len(), 2);
    assert_eq!(state.items[0].text, "$ pwd\n/project");
    assert_eq!(state.items[1].text, "$ date\nToday");
    assert!(state.items.iter().all(|item| item.complete));
    assert_eq!(state.activity_outcomes["streamed"].exit_code, Some(0));
    assert_eq!(state.activity_outcomes["final-only"].duration_ms, Some(12));
    state.remove_item(0);
    assert!(!state.activity_outcomes.contains_key("streamed"));
    assert!(state.activity_outcomes.contains_key("final-only"));
    state.clear_conversation();
    assert!(state.activity_outcomes.is_empty());
}

#[test]
fn final_only_file_change_uses_typed_targets_not_diff_lines() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread".into())),
        ..Default::default()
    };
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::ItemCompleted {
            item_id: "final-file".into(),
            completion: Some(nickel_codex::CompletedItem {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                completed_at_ms: None,
                item_type: "fileChange".into(),
                text: "+not-a-path\n-previous line".into(),
                status: Some("completed".into()),
                exit_code: None,
                duration_ms: None,
                changes: vec![FilePatchChange {
                    kind: "update".into(),
                    path: "/project/real.rs".into(),
                    move_path: None,
                    diff: "+not-a-path".into(),
                }],
                summary_parts: Vec::new(),
                command_actions: Vec::new(),
            }),
        },
    });
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].text, "+not-a-path\n-previous line");
    assert_eq!(
        state
            .file_change_summaries
            .get("final-file")
            .map(String::as_str),
        Some("update: /project/real.rs")
    );
    state.clear_conversation();
    assert!(state.file_change_summaries.is_empty());
}

#[test]
fn retrying_turn_error_keeps_connection_healthy_and_final_failure_is_distinct() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    state.selected_thread = Some(ThreadId("thread".into()));
    state.active_turn = Some(TurnId("turn".into()));
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::TurnError {
            thread_id: ThreadId("thread".into()),
            turn_id: TurnId("turn".into()),
            message: "Temporary outage".into(),
            will_retry: true,
        },
    });
    assert_eq!(state.status, ConnectionStatus::Ready);
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::Retrying
    );
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].kind, ChatItemKind::Error);
    assert!(state.items[0].text.starts_with("Codex is retrying"));
    let error_item_id = state.items[0].id.clone();
    state.apply_protocol(CodexEvent {
        sequence: 2,
        kind: EventKind::TurnError {
            thread_id: ThreadId("thread".into()),
            turn_id: TurnId("turn".into()),
            message: "Final failure".into(),
            will_retry: false,
        },
    });
    state.apply_protocol(CodexEvent {
        sequence: 3,
        kind: EventKind::TurnCompleted {
            thread_id: ThreadId("thread".into()),
            turn_id: TurnId("turn".into()),
            status: "failed".into(),
        },
    });
    assert_eq!(state.status, ConnectionStatus::Ready);
    assert_eq!(
        state.run_presentation_status(),
        RunPresentationStatus::TurnFailed
    );
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].id, error_item_id);
    assert_eq!(state.items[0].text, "Turn failed: Final failure");
    assert!(state.items[0].complete);
}

#[test]
fn successful_retry_closes_error_card_once() {
    let mut state = ChatState {
        selected_thread: Some(ThreadId("thread".into())),
        active_turn: Some(TurnId("turn".into())),
        ..Default::default()
    };
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::TurnError {
            thread_id: ThreadId("thread".into()),
            turn_id: TurnId("turn".into()),
            message: "Temporary outage".into(),
            will_retry: true,
        },
    });
    for sequence in [2, 3] {
        state.apply_protocol(CodexEvent {
            sequence,
            kind: EventKind::TurnCompleted {
                thread_id: ThreadId("thread".into()),
                turn_id: TurnId("turn".into()),
                status: "completed".into(),
            },
        });
    }
    assert_eq!(state.items.len(), 1);
    assert!(state.items[0].complete);
    assert_eq!(state.items[0].text.matches("Retry succeeded").count(), 1);
}

#[test]
fn replacement_is_blocked_by_starting_active_and_pending_interactions() {
    let mut state = ChatState::default();
    assert_eq!(state.replacement_block_reason(), None);
    state.send_pending = true;
    assert!(state.replacement_block_reason().unwrap().contains("start"));
    state.send_pending = false;
    state.active_turn = Some(TurnId("turn".into()));
    assert!(
        state
            .replacement_block_reason()
            .unwrap()
            .contains("Interrupt")
    );
    state.active_turn = None;
    state.pending.push(PendingInteraction::Approval {
        request_id: ServerRequestId("request".into()),
        approval_type: "item/fileChange/requestApproval".into(),
        summary: "Change files".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(
        state
            .replacement_block_reason()
            .unwrap()
            .contains("Resolve")
    );
}

#[test]
fn collapsed_command_uses_bounded_virtual_height_until_expanded() {
    let mut state = ChatState::default();
    state.push_item(ChatItem {
        id: "command".into(),
        kind: ChatItemKind::Command,
        text: format!("$ cargo test\n{}", "output\n".repeat(10_000)),
        complete: true,
    });
    assert_eq!(state.estimated_item_heights(), vec![104.0]);
    state.expanded_items.insert("command".into());
    assert!(state.estimated_item_heights()[0] > 104.0);
}

#[test]
fn unknown_methods_coalesce_in_diagnostics_without_flooding_transcript() {
    let mut state = ChatState::default();
    for sequence in 1..=1000 {
        state.apply_protocol(CodexEvent {
            sequence,
            kind: EventKind::UnsupportedEvent {
                method: "future/status".into(),
            },
        });
    }
    assert!(state.items.is_empty());
    assert_eq!(state.unsupported_method_counts["future/status"], 1000);
    assert!(state.diagnostics.len() <= 10);
    assert!(state.diagnostics.back().unwrap().contains("512"));
    for index in 0..40 {
        state.apply_protocol(CodexEvent {
            sequence: 1001 + index,
            kind: EventKind::UnsupportedEvent {
                method: format!("future/{index}"),
            },
        });
    }
    state.apply_protocol(CodexEvent {
        sequence: 1041,
        kind: EventKind::UnsupportedEvent {
            method: "future/status".into(),
        },
    });
    assert_eq!(state.unsupported_method_counts["future/status"], 1001);
    assert!(state.unsupported_method_counts.len() <= 33);
}

#[test]
fn app_server_activity_families_are_not_mislabeled_as_unknown() {
    for (wire, kind) in [
        ("mcpToolCall", ChatItemKind::Tool),
        ("dynamicToolCall", ChatItemKind::Tool),
        ("webSearch", ChatItemKind::Search),
        ("imageGeneration", ChatItemKind::Image),
        ("collabAgentToolCall", ChatItemKind::Delegation),
        ("subAgentActivity", ChatItemKind::Delegation),
        ("contextCompaction", ChatItemKind::SessionNotice),
    ] {
        assert_eq!(chat_item_kind(wire), kind);
    }
}

#[test]
fn thread_pages_advance_without_silently_discarding_unreachable_history() {
    let thread = |index: usize| Thread {
        id: ThreadId(format!("thread-{index}")),
        title: Some(format!("Conversation {index}")),
        cwd: None,
        last_used_at: Some(1000 - index as i64),
        turns: Vec::new(),
        model: None,
        reasoning_effort: None,
    };
    let mut state = ChatState::default();
    state.apply(
        1,
        ControllerEvent::Ready {
            provenance: "fixture".into(),
            account: AccountState {
                authenticated: true,
                ..Default::default()
            },
            models: Vec::new(),
            projects: Vec::new(),
            threads: (0..100).map(thread).collect(),
            runtime: HashMap::new(),
            thread_error: None,
            thread_next_cursor: Some("100".into()),
        },
    );
    assert_eq!(state.begin_thread_page(), Some(("100".into(), 1)));
    state.apply(
        1,
        ControllerEvent::ThreadPageLoaded {
            request: 1,
            cursor: "stale".into(),
            threads: vec![thread(900)],
            runtime: HashMap::new(),
            next_cursor: None,
        },
    );
    assert_eq!(state.threads.len(), 100);
    state.apply(
        1,
        ControllerEvent::ThreadPageLoaded {
            request: 1,
            cursor: "100".into(),
            threads: (100..200).map(thread).collect(),
            runtime: HashMap::new(),
            next_cursor: Some("200".into()),
        },
    );
    assert_eq!(state.threads.len(), 200);
    assert_eq!(state.begin_thread_page(), Some(("200".into(), 2)));
    state.apply(
        1,
        ControllerEvent::ThreadPageLoaded {
            request: 2,
            cursor: "200".into(),
            threads: (200..250).map(thread).collect(),
            runtime: HashMap::new(),
            next_cursor: None,
        },
    );
    assert_eq!(state.threads.len(), 200);
    assert!(state.thread_windowed);
    assert_eq!(state.threads.first().unwrap().id.0, "thread-50");
    assert_eq!(state.threads.last().unwrap().id.0, "thread-249");
    assert!(state.thread_next_cursor.is_none());
}

#[test]
fn refreshed_thread_page_rejects_old_response_even_when_cursor_matches() {
    let ready = || ControllerEvent::Ready {
        provenance: "fixture".into(),
        account: AccountState {
            authenticated: true,
            ..Default::default()
        },
        models: Vec::new(),
        projects: Vec::new(),
        threads: Vec::new(),
        runtime: HashMap::new(),
        thread_error: None,
        thread_next_cursor: Some("100".into()),
    };
    let mut state = ChatState::default();
    state.apply(1, ready());
    assert_eq!(state.begin_thread_page(), Some(("100".into(), 1)));
    state.apply(1, ready());
    assert_eq!(state.begin_thread_page(), Some(("100".into(), 2)));
    state.apply(
        1,
        ControllerEvent::ThreadPageFailed {
            request: 1,
            cursor: "100".into(),
            message: "stale failure".into(),
        },
    );
    assert!(state.thread_paging);
    assert!(state.thread_page_error.is_none());
    state.apply(
        1,
        ControllerEvent::ThreadPageFailed {
            request: 2,
            cursor: "100".into(),
            message: "current failure".into(),
        },
    );
    assert!(!state.thread_paging);
    assert_eq!(state.thread_page_error.as_deref(), Some("current failure"));
}

#[test]
fn transcript_capacity_plateaus_and_evicted_late_deltas_stay_retired() {
    let mut state = ChatState::default();
    for index in 0..100 {
        state.push_item(ChatItem {
            id: format!("item-{index}"),
            kind: ChatItemKind::Agent,
            text: "x".repeat(MAX_ITEM_TEXT_BYTES),
            complete: true,
        });
        assert!(
            state
                .items
                .iter()
                .map(|item| item.text.capacity())
                .sum::<usize>()
                <= MAX_TRANSCRIPT_TEXT_BYTES
        );
    }
    let retained = state.items.len();
    state.append_delta("item-0".into(), "stale".into(), ChatItemKind::Agent);
    assert_eq!(state.items.len(), retained);
    let cached = state.transcript_selection_document();
    let weak = Arc::downgrade(&cached);
    drop(cached);
    state.new_chat();
    assert!(weak.upgrade().is_none());
    assert!(state.retired_item_ids.is_empty());
}

#[test]
fn pending_approval_outlives_evicted_transcript_reference() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        ..Default::default()
    };
    state.account.authenticated = true;
    state.selected_thread = Some(ThreadId("thread".into()));
    let request_id = ServerRequestId("pending-after-eviction".into());
    state.push_pending(PendingInteraction::Approval {
        request_id: request_id.clone(),
        approval_type: "item/commandExecution/requestApproval".into(),
        summary: "Run a command".into(),
        context: nickel_codex::ApprovalContext::default(),
    });
    assert!(!state.interaction_item_ids.contains_key(&request_id));
    // A pending approval does not need a transcript reference. It remains
    // authoritative even when the transcript fills and evicts older items.
    for index in 0..MAX_ITEMS {
        state.push_item(ChatItem {
            id: format!("running-{index}"),
            kind: ChatItemKind::Agent,
            text: "streaming".into(),
            complete: false,
        });
    }
    assert!(state.interaction_is_actionable(&request_id));
    assert_eq!(state.pending.len(), 1);
    state.apply_protocol(CodexEvent {
        sequence: 1,
        kind: EventKind::ServerRequestResolved {
            thread_id: ThreadId("thread".into()),
            request_id: request_id.clone(),
        },
    });
    assert!(state.pending.is_empty());
    assert!(!state.interaction_is_actionable(&request_id));
}

#[test]
fn pathological_markdown_expansion_uses_visible_bounded_plain_text() {
    let mut state = ChatState::default();
    state.push_item(ChatItem {
        id: "i".repeat(4096),
        kind: ChatItemKind::Agent,
        text: "**x** ".repeat(1000),
        complete: true,
    });
    let document = state.markdown_document(0);
    assert!(document.source.contains("Formatting omitted"));
    let cache = state.item_selection_runs[0].cached.borrow();
    let cache = cache.as_ref().unwrap();
    assert!(
        crate::projection_memory::derived_capacity(&document, &cache.runs)
            <= 2048 + 16 * state.items[0].text.capacity() + 4 * state.items[0].id.capacity()
    );
}

#[test]
#[ignore = "release streaming comparison; run with --release --ignored --nocapture"]
fn streaming_projection_measurement() {
    for bytes in [4096, 32768, 131072] {
        let chunk = "Some **text** and Unicode 世界. ".repeat(2);
        let count = bytes / chunk.len();
        let mut legacy = ChatItem {
            id: "stream".into(),
            kind: ChatItemKind::Agent,
            text: String::new(),
            complete: false,
        };
        let started = Instant::now();
        for _ in 0..count {
            legacy.text.push_str(&chunk);
            black_box(selection_runs_for_item(&legacy));
        }
        let legacy_elapsed = started.elapsed();
        let mut state = ChatState::default();
        let started = Instant::now();
        for index in 0..count {
            delta(&mut state, &chunk);
            if (index + 1) % 128 == 0 {
                black_box(state.transcript_selection_document().runs());
            }
        }
        let selected = state.transcript_selection_document();
        black_box(selected.runs());
        let elapsed = started.elapsed();
        let builds = state.item_selection_runs[0].builds.get();
        assert_eq!(
            selected.runs(),
            SelectionDocument::new(selection_runs_for_item(&legacy)).runs()
        );
        assert_eq!(builds as usize, count.div_ceil(128));
        let cache = state.item_selection_runs[0].cached.borrow();
        let cache = cache.as_ref().unwrap();
        let retained = state.items[0].text.capacity()
            + crate::projection_memory::derived_capacity(&cache.document, &cache.runs);
        eprintln!(
            "streaming bytes={} deltas={count} legacy_rebuilds={count} batched_rebuilds={builds} legacy_us={} batched_us={} retained_capacity={retained}",
            legacy.text.len(),
            legacy_elapsed.as_micros(),
            elapsed.as_micros()
        );
        assert!(
            elapsed < legacy_elapsed / 4,
            "batching should reduce this repeated-parse workload by at least 75%"
        );
    }
}

const TINY_DERIVED_OPERATION_P95_ADDITION: std::time::Duration =
    std::time::Duration::from_micros(100);

#[test]
fn failed_send_retains_unicode_draft_and_images_until_turn_is_accepted() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        draft: "hello 世界".into(),
        ..ChatState::default()
    };
    state.account.authenticated = true;
    state.attach_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
    let (text, images) = state.begin_send().unwrap();
    assert_eq!(text, "hello 世界");
    assert_eq!(images.len(), 1);
    assert!(!state.can_send());
    state.apply(1, ControllerEvent::OperationFailed("offline".into()));
    assert_eq!(state.draft, "hello 世界");
    assert_eq!(state.attachments.len(), 1);
    assert!(state.can_send());

    state.begin_send().unwrap();
    state.apply(
        1,
        ControllerEvent::Protocol(CodexEvent {
            sequence: 1,
            kind: EventKind::TurnStarted {
                thread_id: ThreadId("t".into()),
                turn_id: TurnId("turn".into()),
            },
        }),
    );
    assert!(state.draft.is_empty());
    assert!(state.attachments.is_empty());
}

#[test]
fn attachment_admission_is_count_and_total_resident_memory_bounded() {
    let mut state = ChatState::default();
    let tiny_count = AttachmentLimits {
        count: 1,
        ..AttachmentLimits::default()
    };
    state
        .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_count)
        .unwrap();
    assert_eq!(
        state
            .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_count)
            .unwrap_err(),
        AttachmentError::TooMany
    );

    let first = state.attachments[0].id;
    assert!(state.remove_attachment(first));
    assert!(state.attachments.is_empty());
    let tiny_memory = AttachmentLimits {
        aggregate_decoded_bytes: 4,
        ..AttachmentLimits::default()
    };
    assert_eq!(
        state
            .attach_rgba_with_limits(1, 1, &[1, 2, 3, 255], tiny_memory)
            .unwrap_err(),
        AttachmentError::AggregateLimit
    );
    assert!(state.attachments.is_empty());
}

fn representative_long_transcript() -> ChatState {
    let mut state = ChatState::default();
    for index in 0..MAX_ITEMS {
        state.push_item(ChatItem {
            id: format!("item-{index}"),
            kind: if index % 4 == 0 {
                ChatItemKind::User
            } else {
                ChatItemKind::Agent
            },
            text: format!(
                "## Transcript item {index}\n\nThis is representative prose with **formatting**, \
                 a [link](https://example.invalid/{index}), and enough content to exercise \
                 Markdown selection projection.\n\n- first result\n- second result\n\n```text\n\
                 deterministic output {index}\n```"
            ),
            complete: true,
        });
    }
    state
}

fn p95(samples: &mut [std::time::Duration]) -> std::time::Duration {
    samples.sort_unstable();
    samples[samples.len() * 95 / 100]
}

fn selection_run_retained_bytes(state: &ChatState) -> usize {
    state
        .item_selection_runs
        .iter()
        .flat_map(|projection| {
            projection
                .cached
                .borrow()
                .as_ref()
                .map(|cached| cached.runs.clone())
                .unwrap_or_default()
        })
        .map(|run| size_of::<SelectionRun>() + run.id.capacity() + run.text.len())
        .sum()
}

fn item_index_retained_bytes(index: &HashMap<String, usize>) -> usize {
    index.capacity() * (size_of::<String>() + size_of::<usize>() + size_of::<usize>())
        + index.keys().map(|key| key.capacity()).sum::<usize>()
}

#[test]
#[ignore = "release-profile cache admission measurement; run explicitly"]
fn recomputing_2k_item_heights_is_within_tiny_operation_budget() {
    let items = (0..MAX_ITEMS)
        .map(|index| ChatItem {
            id: format!("item-{index}"),
            kind: if index % 3 == 0 {
                ChatItemKind::User
            } else {
                ChatItemKind::Agent
            },
            text: format!(
                "Measured transcript item {index}: deterministic text spanning a representative chat line."
            ),
            complete: true,
        })
        .collect::<Vec<_>>();
    let cached = items.iter().map(estimate_item_height).collect::<Vec<_>>();
    let recomputed = items.iter().map(estimate_item_height).collect::<Vec<_>>();
    assert_eq!(cached, recomputed, "cached and recomputed heights differ");

    let mut cached_samples = Vec::with_capacity(200);
    let mut recomputed_samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let start = Instant::now();
        let cached_result = black_box(&cached).to_vec();
        black_box(cached_result);
        cached_samples.push(start.elapsed());

        let start = Instant::now();
        let recomputed_result = black_box(&items)
            .iter()
            .map(estimate_item_height)
            .collect::<Vec<_>>();
        black_box(recomputed_result);
        recomputed_samples.push(start.elapsed());
    }
    cached_samples.sort_unstable();
    recomputed_samples.sort_unstable();
    let p95_index = cached_samples.len() * 95 / 100;
    let cached_p95 = cached_samples[p95_index];
    let recomputed_p95 = recomputed_samples[p95_index];
    let addition = recomputed_p95.saturating_sub(cached_p95);
    eprintln!(
        "2k item heights: cached_p95={cached_p95:?} recomputed_p95={recomputed_p95:?} addition={addition:?}"
    );
    assert!(
        addition <= TINY_DERIVED_OPERATION_P95_ADDITION,
        "recomputation added {addition:?}, exceeding the predeclared {:?} p95 budget",
        TINY_DERIVED_OPERATION_P95_ADDITION
    );
}

#[test]
fn selection_projections_are_bounded_released_and_equivalent() {
    let mut state = representative_long_transcript();
    for index in MAX_ITEMS..MAX_ITEMS + 20 {
        state.push_item(ChatItem {
            id: format!("item-{index}"),
            kind: ChatItemKind::Agent,
            text: format!("replacement {index}"),
            complete: true,
        });
    }

    assert_eq!(state.items.len(), MAX_ITEMS);
    assert!(state.item_aliases.is_empty());
    assert_eq!(state.item_selection_runs.len(), MAX_ITEMS);
    assert_eq!(state.resolve_item_index("item-0"), None);
    assert_eq!(state.resolve_item_index("item-2019"), Some(MAX_ITEMS - 1));

    let cached = state.transcript_selection_document();
    let recomputed = SelectionDocument::new(state.items.iter().flat_map(selection_runs_for_item));
    assert_eq!(cached.runs(), recomputed.runs());

    state.clear_conversation();
    assert!(state.items.is_empty());
    assert!(state.item_aliases.is_empty());
    assert!(state.item_selection_runs.is_empty());
    assert!(state.selection_document_cache.borrow().2.runs().is_empty());
}

#[test]
fn transcript_mutation_drops_stale_cached_document() {
    let mut state = representative_long_transcript();
    let stale = state.transcript_selection_document();
    assert!(!stale.runs().is_empty());
    assert_eq!(Arc::strong_count(&stale), 2);

    state.append_delta("item-1999".into(), " tail".into(), ChatItemKind::Agent);

    assert!(state.selection_document_cache.borrow().2.runs().is_empty());
    assert_eq!(Arc::strong_count(&stale), 1);
    let current = state.transcript_selection_document();
    assert_ne!(&*current, &*stale);
}

#[test]
fn rejected_selected_model_falls_back_visibly_without_losing_the_draft() {
    let mut state = ChatState {
        status: ConnectionStatus::Ready,
        models: vec![
            Model {
                id: "rejected".into(),
                display_name: "Rejected".into(),
                default_reasoning_effort: Some("high".into()),
                supported_reasoning_efforts: Vec::new(),
            },
            Model {
                id: "fallback".into(),
                display_name: "Fallback".into(),
                default_reasoning_effort: Some("medium".into()),
                supported_reasoning_efforts: Vec::new(),
            },
        ],
        selected_model: Some("rejected".into()),
        selected_reasoning_effort: Some("high".into()),
        draft: "keep this retry".into(),
        ..Default::default()
    };
    state.account.authenticated = true;
    assert!(state.begin_send().is_some());

    state.apply(
        state.generation,
        ControllerEvent::ModelRejected {
            model: "rejected".into(),
            message: "unknown model rejected".into(),
        },
    );

    assert_eq!(state.selected_model.as_deref(), Some("fallback"));
    assert_eq!(state.selected_reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(state.draft, "keep this retry");
    assert!(state.can_send());
    assert!(state.diagnostics.back().is_some_and(|message| {
        message.contains("selected model was rejected")
            && message.contains("unknown model rejected")
    }));
}

#[test]
fn late_rejection_cannot_replace_a_newer_explicit_model_choice() {
    let mut state = ChatState {
        models: vec![Model {
            id: "new-choice".into(),
            display_name: "New choice".into(),
            default_reasoning_effort: Some("medium".into()),
            supported_reasoning_efforts: Vec::new(),
        }],
        selected_model: Some("new-choice".into()),
        selected_reasoning_effort: Some("medium".into()),
        ..Default::default()
    };

    state.apply(
        state.generation,
        ControllerEvent::ModelRejected {
            model: "old-choice".into(),
            message: "unknown model old-choice".into(),
        },
    );

    assert_eq!(state.selected_model.as_deref(), Some("new-choice"));
    assert_eq!(state.selected_reasoning_effort.as_deref(), Some("medium"));
}

#[test]
fn merged_item_alias_routing_is_bounded_and_reconciled() {
    let mut state = ChatState::default();
    state.push_item(ChatItem {
        id: "canonical".into(),
        kind: ChatItemKind::Agent,
        text: "first".into(),
        complete: true,
    });
    for index in 0..MAX_ITEM_ALIASES + 20 {
        state.register_item_alias(format!("alias-{index}"), 0);
    }
    assert_eq!(state.item_aliases.len(), MAX_ITEM_ALIASES);
    assert_eq!(state.resolve_item_index("alias-0"), None);
    assert_eq!(
        state.resolve_item_index(&format!("alias-{}", MAX_ITEM_ALIASES + 19)),
        Some(0)
    );

    state.clear_conversation();
    assert!(state.item_aliases.is_empty());
}

#[test]
#[ignore = "release-profile cache admission measurement; run explicitly"]
fn long_transcript_cache_admission_measurement() {
    let state = representative_long_transcript();
    let cached_document = state.transcript_selection_document();
    let expected_runs = cached_document.runs().len();
    let cached_index = state
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let index_bytes = item_index_retained_bytes(&cached_index);
    let selection_run_bytes = selection_run_retained_bytes(&state);
    let document_bytes = expected_runs
        * (size_of::<SelectionRun>() + size_of::<String>() + size_of::<usize>() * 2)
        + cached_document
            .runs()
            .iter()
            .map(|run| run.id.capacity() * 2)
            .sum::<usize>();

    let mut indexed = Vec::with_capacity(400);
    let mut linear = Vec::with_capacity(400);
    for sample in 0..400 {
        let id = format!("item-{}", (sample * 1543) % MAX_ITEMS);
        let start = Instant::now();
        black_box(cached_index.get(black_box(&id)).copied());
        indexed.push(start.elapsed());
        let start = Instant::now();
        black_box(state.items.iter().position(|item| item.id == id));
        linear.push(start.elapsed());
    }

    let mut cached_runs = Vec::with_capacity(40);
    let mut rebuilt_runs = Vec::with_capacity(40);
    for _ in 0..40 {
        let start = Instant::now();
        let runs = state
            .item_selection_runs
            .iter()
            .flat_map(|projection| projection.cached.borrow().as_ref().unwrap().runs.clone())
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), expected_runs);
        black_box(runs);
        cached_runs.push(start.elapsed());

        let start = Instant::now();
        let runs = state
            .items
            .iter()
            .flat_map(selection_runs_for_item)
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), expected_runs);
        black_box(runs);
        rebuilt_runs.push(start.elapsed());
    }

    let mut cached_documents = Vec::with_capacity(100);
    let mut rebuilt_documents = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        black_box(state.transcript_selection_document());
        cached_documents.push(start.elapsed());

        let start = Instant::now();
        let document = SelectionDocument::new(
            state
                .item_selection_runs
                .iter()
                .flat_map(|projection| projection.cached.borrow().as_ref().unwrap().runs.clone()),
        );
        assert_eq!(document.runs().len(), expected_runs);
        black_box(document);
        rebuilt_documents.push(start.elapsed());
    }

    let indexed_p95 = p95(&mut indexed);
    let linear_p95 = p95(&mut linear);
    let cached_runs_p95 = p95(&mut cached_runs);
    let rebuilt_runs_p95 = p95(&mut rebuilt_runs);
    let cached_document_p95 = p95(&mut cached_documents);
    let rebuilt_document_p95 = p95(&mut rebuilt_documents);
    eprintln!(
        "long transcript ({MAX_ITEMS} items, {expected_runs} runs): \
         item_index cached_p95={indexed_p95:?} linear_p95={linear_p95:?} retained={index_bytes}B; \
         selection_runs cached_p95={cached_runs_p95:?} rebuilt_p95={rebuilt_runs_p95:?} retained={selection_run_bytes}B; \
         selection_document cached_p95={cached_document_p95:?} rebuilt_p95={rebuilt_document_p95:?} retained={document_bytes}B"
    );

    assert!(linear_p95.saturating_sub(indexed_p95) <= TINY_DERIVED_OPERATION_P95_ADDITION);
    assert!(rebuilt_runs_p95.saturating_sub(cached_runs_p95) > TINY_DERIVED_OPERATION_P95_ADDITION);
    assert!(
        rebuilt_document_p95.saturating_sub(cached_document_p95)
            > TINY_DERIVED_OPERATION_P95_ADDITION
    );
}
