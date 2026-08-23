mod controller;
mod model;
mod view;

pub use controller::{BackendMode, ChatController, ControllerCommand, ControllerEvent};
pub use model::{ChatItem, ChatItemKind, ChatState, ConnectionStatus, PendingInteraction};
pub use view::{ChatApplication, ChatMessage};

#[cfg(test)]
mod tests {
    use nickel_codex::{
        BackendChoice, CodexEvent, EventKind, ReplayBackend, ServerRequestId, ThreadId, TurnId,
    };
    use nickel_ui::{Application, PaintCommand, Rect, SdlComponentRenderer, Shortcut, UiTree};

    use super::*;

    fn event(sequence: u64, kind: EventKind) -> ControllerEvent {
        ControllerEvent::Protocol(CodexEvent { sequence, kind })
    }

    #[test]
    fn streamed_items_keep_identity_and_terminal_state_is_idempotent() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.apply(
            1,
            event(
                1,
                EventKind::TurnStarted {
                    thread_id: ThreadId("t".into()),
                    turn_id: TurnId("turn".into()),
                },
            ),
        );
        state.apply(
            1,
            event(
                2,
                EventKind::ItemStarted {
                    thread_id: Some(ThreadId("t".into())),
                    turn_id: Some(TurnId("turn".into())),
                    item_id: "agent".into(),
                    item_type: "agentMessage".into(),
                },
            ),
        );
        state.apply(
            1,
            event(
                3,
                EventKind::AgentMessageDelta {
                    item_id: "agent".into(),
                    delta: "hello".into(),
                },
            ),
        );
        let terminal = EventKind::TurnCompleted {
            thread_id: ThreadId("t".into()),
            turn_id: TurnId("turn".into()),
            status: "completed".into(),
        };
        state.apply(1, event(4, terminal.clone()));
        state.apply(1, event(5, terminal));
        assert_eq!(state.items[0].id, "agent");
        assert_eq!(state.items[0].text, "hello");
        assert!(state.active_turn.is_none());
    }

    #[test]
    fn stale_generations_and_blank_sends_are_ignored() {
        let mut state = ChatState::default();
        state.apply(2, ControllerEvent::Failure("stale".into()));
        assert_eq!(state.status, ConnectionStatus::Loading);
        state.status = ConnectionStatus::Ready;
        state.draft = "   ".into();
        assert!(state.begin_send().is_none());
        state.draft = "hello".into();
        assert_eq!(state.begin_send().as_deref(), Some("hello"));
        assert_eq!(state.items[0].kind, ChatItemKind::User);
    }

    #[test]
    fn terminal_and_recoverable_failures_have_distinct_connection_states() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.apply(1, ControllerEvent::OperationFailed("turn failed".into()));
        assert_eq!(state.status, ConnectionStatus::Ready);
        state.apply(1, ControllerEvent::Incompatible("schema mismatch".into()));
        assert_eq!(state.status, ConnectionStatus::Incompatible);
        state.apply(1, ControllerEvent::Failure("process stopped".into()));
        assert_eq!(state.status, ConnectionStatus::Disconnected);
    }

    #[test]
    fn resumed_delta_materializes_an_item_without_a_visible_inconsistency() {
        let mut state = ChatState::default();
        state.apply(
            1,
            event(
                1,
                EventKind::AgentMessageDelta {
                    item_id: "resumed-message".into(),
                    delta: "continued".into(),
                },
            ),
        );
        state.apply(
            1,
            event(
                2,
                EventKind::Inconsistency {
                    message: "delta for unknown item resumed-message".into(),
                },
            ),
        );
        assert_eq!(state.items[0].kind, ChatItemKind::Agent);
        assert_eq!(state.items[0].text, "continued");
        assert!(state.diagnostics.is_empty());
    }

    #[test]
    fn selected_resumed_thread_hydrates_persisted_history_in_order() {
        let mut state = ChatState::default();
        let thread_id = ThreadId("persisted".into());
        state.apply(
            1,
            ControllerEvent::ThreadSelected(nickel_codex::Thread {
                id: thread_id.clone(),
                title: Some("Persisted".into()),
                cwd: None,
                turns: vec![nickel_codex::ThreadHistoryTurn {
                    id: TurnId("turn".into()),
                    status: "completed".into(),
                    items: vec![
                        nickel_codex::ThreadHistoryItem {
                            id: "user".into(),
                            item_type: "userMessage".into(),
                            text: "previous question".into(),
                        },
                        nickel_codex::ThreadHistoryItem {
                            id: "agent".into(),
                            item_type: "agentMessage".into(),
                            text: "previous answer".into(),
                        },
                    ],
                }],
            }),
        );
        assert_eq!(state.selected_thread, Some(thread_id));
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].kind, ChatItemKind::User);
        assert_eq!(state.items[1].text, "previous answer");
    }

    #[test]
    fn approvals_are_visible_and_never_implicit() {
        let mut state = ChatState::default();
        state.apply(
            1,
            event(
                1,
                EventKind::ApprovalRequested {
                    request_id: ServerRequestId("approval".into()),
                    approval_type: "commandExecution".into(),
                    summary: Some("cargo test".into()),
                },
            ),
        );
        assert_eq!(state.pending.len(), 1);
        let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Decline")
        ));
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Approve")
        ));
    }

    #[test]
    fn canonical_states_layout_at_small_normal_and_large_sizes() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.provenance = "Installed · codex-cli fixture".into();
        state.items.push_back(ChatItem {
            id: "unicode".into(),
            kind: ChatItemKind::Agent,
            text: "Hello 👋🏽\n```rust\nfn main() {}\n```".into(),
            complete: true,
        });
        for (width, height) in [(640.0, 480.0), (1120.0, 760.0), (2240.0, 1520.0)] {
            let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, width, height));
            assert!(tree.resolved_layout().nodes().iter().all(|node| {
                node.allocated.origin.x.is_finite()
                    && node.allocated.origin.y.is_finite()
                    && node.allocated.size.width.is_finite()
                    && node.allocated.size.height.is_finite()
                    && node.allocated.size.width >= 0.0
                    && node.allocated.size.height >= 0.0
            }));
            assert!(tree.commands().iter().any(
                |command| matches!(command, PaintCommand::Text { text, .. } if text.contains("Hello"))
            ));
        }
    }

    #[test]
    fn canonical_conversation_rasterizes_at_low_and_high_dpi() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.items.push_back(ChatItem {
            id: "agent".into(),
            kind: ChatItemKind::Agent,
            text: "Visible response".into(),
            complete: true,
        });
        for scale in [1.0, 2.0] {
            let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 800.0, 600.0));
            let mut renderer =
                SdlComponentRenderer::new((800.0 * scale) as u32, (600.0 * scale) as u32, scale);
            assert!(!renderer.render(tree.commands()).is_empty());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
        }
    }

    #[test]
    fn replay_application_reaches_ready_without_a_process() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"ui-ready",
                "account":{"authenticated":false,"account_type":null,"email":null},
                "models":[{"id":"fixture","display_name":"Fixture"}],
                "threads":[],
                "events":[]
            }"#,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        for _ in 0..100 {
            app.poll_controller();
            if app.state.status == ConnectionStatus::Ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(app.state.status, ConnectionStatus::Ready);
        assert_eq!(app.state.provenance, "Replay fixture");
    }

    #[test]
    fn composer_shortcuts_insert_newlines_and_submit_nonblank_drafts() {
        let backend = ReplayBackend::from_json(r#"{"name":"shortcuts","events":[]}"#).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "first".into();
        assert!(app.shortcut(Shortcut::Newline));
        assert_eq!(app.state.draft, "first\n");
        assert!(app.shortcut(Shortcut::Submit));
        assert!(app.state.draft.is_empty());
        assert_eq!(app.state.items.back().unwrap().kind, ChatItemKind::User);
    }

    #[test]
    fn interrupt_request_is_visible_and_clears_only_at_terminal_turn_state() {
        let backend = ReplayBackend::from_json(r#"{"name":"interrupt","events":[]}"#).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        app.state.active_turn = Some(TurnId("turn".into()));
        app.update(ChatMessage::Interrupt);
        assert!(app.state.interrupt_requested);
        app.update(ChatMessage::Interrupt);
        assert!(app.state.interrupt_requested);
        app.state.apply(
            1,
            event(
                1,
                EventKind::TurnCompleted {
                    thread_id: ThreadId("thread".into()),
                    turn_id: TurnId("turn".into()),
                    status: "interrupted".into(),
                },
            ),
        );
        assert!(!app.state.interrupt_requested);
        assert!(app.state.active_turn.is_none());
    }

    #[test]
    fn independent_applications_do_not_share_conversation_state() {
        let directory = tempfile::tempdir().unwrap();
        let mode = || BackendMode::Replay {
            backend: ReplayBackend::from_json(r#"{"name":"independent","events":[]}"#).unwrap(),
            cwd: directory.path().into(),
        };
        let mut first = ChatApplication::new(mode());
        let second = ChatApplication::new(mode());
        first.state.status = ConnectionStatus::Ready;
        first.state.draft = "only first".into();
        first.state.begin_send();
        assert_eq!(first.state.items.len(), 1);
        assert!(second.state.items.is_empty());
        assert!(second.state.draft.is_empty());
    }

    #[test]
    fn state_is_bounded_and_sensitive_diagnostics_are_redacted() {
        let mut state = ChatState::default();
        for sequence in 0..2_100 {
            state.apply(
                1,
                event(
                    sequence,
                    EventKind::ItemStarted {
                        thread_id: None,
                        turn_id: None,
                        item_id: format!("item-{sequence}"),
                        item_type: "agentMessage".into(),
                    },
                ),
            );
        }
        assert_eq!(state.items.len(), 2_000);
        state.apply(
            1,
            event(
                2_101,
                EventKind::Error {
                    message: "Authorization: Bearer private".into(),
                },
            ),
        );
        assert_eq!(
            state.diagnostics.back().map(String::as_str),
            Some("Sensitive backend diagnostic redacted")
        );
    }

    #[test]
    fn crate_manifest_has_no_shell_or_platform_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("nickel-shell"));
        assert!(!manifest.contains("nickel-session"));
        assert!(!manifest.contains("nickel-platform"));
    }

    #[test]
    #[ignore = "requires explicit authenticated Codex subscription access"]
    fn authenticated_live_first_turn_and_fresh_connection_resume() {
        assert_eq!(
            std::env::var("NICKEL_CODEX_LIVE").as_deref(),
            Ok("1"),
            "set NICKEL_CODEX_LIVE=1 explicitly"
        );
        let directory = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(directory.path())
            .status()
            .unwrap();
        assert!(status.success());

        let mut first = ChatApplication::new(BackendMode::Live {
            choice: BackendChoice::Installed,
            cwd: directory.path().into(),
        });
        wait_until(&mut first, |state| state.status == ConnectionStatus::Ready);
        first.update(ChatMessage::DraftChanged(
            "Do not use tools or modify files. Reply with exactly: NICKEL_UI_LIVE_OK".into(),
        ));
        first.update(ChatMessage::Send);
        wait_for_message(&mut first, "NICKEL_UI_LIVE_OK");
        assert!(
            first.state.diagnostics.is_empty(),
            "{:?}",
            first.state.diagnostics
        );
        let thread_id = first.state.selected_thread.clone().expect("live thread id");
        drop(first);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut resumed = ChatApplication::new(BackendMode::Live {
            choice: BackendChoice::Installed,
            cwd: directory.path().into(),
        });
        wait_until(&mut resumed, |state| {
            state.status == ConnectionStatus::Ready
        });
        resumed.update(ChatMessage::SelectThread(thread_id.clone()));
        wait_until(&mut resumed, |state| {
            state.selected_thread.as_ref() == Some(&thread_id)
                && state.items.iter().any(|item| {
                    item.kind == ChatItemKind::Agent && item.text.trim() == "NICKEL_UI_LIVE_OK"
                })
        });
        resumed.update(ChatMessage::DraftChanged(
            "Do not use tools or modify files. Reply with exactly: NICKEL_UI_RESUME_OK".into(),
        ));
        resumed.update(ChatMessage::Send);
        wait_for_message(&mut resumed, "NICKEL_UI_RESUME_OK");
        assert!(resumed.state.pending.is_empty());
        assert!(
            resumed.state.diagnostics.is_empty(),
            "{:?}",
            resumed.state.diagnostics
        );
        drop(resumed);

        let non_git_entries = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name() != ".git")
            .count();
        assert_eq!(non_git_entries, 0);
    }

    fn wait_until(app: &mut ChatApplication, mut predicate: impl FnMut(&ChatState) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            app.poll_controller();
            if predicate(&app.state) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("application state did not reach the expected condition");
    }

    fn wait_for_message(app: &mut ChatApplication, expected: &str) {
        let mut saw_active_turn = false;
        wait_until(app, |state| {
            saw_active_turn |= state.active_turn.is_some();
            saw_active_turn
                && state.active_turn.is_none()
                && state
                    .items
                    .iter()
                    .any(|item| item.kind == ChatItemKind::Agent && item.text.trim() == expected)
        });
    }
}
