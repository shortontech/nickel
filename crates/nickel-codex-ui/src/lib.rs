mod controller;
mod model;
mod view;

pub use controller::{
    BackendMode, ChatController, ControllerCommand, ControllerEvent, create_managed_workspace,
};
pub use model::{ChatItem, ChatItemKind, ChatState, ConnectionStatus, PendingInteraction};
pub use view::{ChatApplication, ChatMessage};

#[cfg(test)]
mod tests {
    use nickel_codex::{
        BackendChoice, CodexEvent, CodexSettings, EventKind, ReplayBackend, ServerRequestId,
        Thread, ThreadId, TurnId,
    };
    use nickel_ui::{
        Application, DocumentSelection, PaintCommand, Point, Rect, SdlComponentRenderer,
        SelectionEndpoint, Shortcut, UiEvent, UiStateStore, UiTree,
    };

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
    fn new_thread_and_server_user_item_preserve_the_optimistic_message() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.draft = "hello".into();
        assert_eq!(state.begin_send().as_deref(), Some("hello"));

        state.apply(
            1,
            ControllerEvent::ThreadCreated(nickel_codex::Thread {
                id: ThreadId("new-thread".into()),
                title: Some("Untitled conversation".into()),
                cwd: None,
                last_used_at: None,
                turns: Vec::new(),
            }),
        );
        state.apply(
            1,
            event(
                1,
                EventKind::ItemStarted {
                    thread_id: Some(ThreadId("new-thread".into())),
                    turn_id: Some(TurnId("turn".into())),
                    item_id: "server-user".into(),
                    item_type: "userMessage".into(),
                },
            ),
        );
        state.apply(
            1,
            event(
                2,
                EventKind::ItemCompleted {
                    item_id: "server-user".into(),
                },
            ),
        );

        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].id, "server-user");
        assert_eq!(state.items[0].text, "hello");
        assert!(state.items[0].complete);
    }

    #[test]
    fn completed_protocol_items_without_content_are_not_rendered() {
        let mut state = ChatState::default();
        state.apply(
            1,
            event(
                1,
                EventKind::ItemStarted {
                    thread_id: None,
                    turn_id: None,
                    item_id: "empty-reasoning".into(),
                    item_type: "reasoning".into(),
                },
            ),
        );
        state.apply(
            1,
            event(
                2,
                EventKind::ItemCompleted {
                    item_id: "empty-reasoning".into(),
                },
            ),
        );
        assert!(state.items.is_empty());
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
    fn selecting_another_thread_clears_the_previous_thread_error() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.interaction_answer = "stale answer".into();
        state.apply(
            1,
            ControllerEvent::OperationFailed("thread already has an active writer".into()),
        );
        assert_eq!(
            state.diagnostics.back().map(String::as_str),
            Some("thread already has an active writer")
        );

        let selected = ThreadId("different-thread".into());
        state.begin_thread_selection(selected.clone());

        assert_eq!(state.selected_thread, Some(selected));
        assert!(state.items.is_empty());
        assert!(state.pending.is_empty());
        assert!(state.diagnostics.is_empty());
        assert!(state.interaction_answer.is_empty());
        assert_eq!(state.conversation_scroll, 0.0);
        assert!(state.conversation_pinned);
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
                last_used_at: None,
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
        state.provenance = "powered by OpenAI Codex CLI vfixture.".into();
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
    fn sidebar_attributes_the_codex_cli_without_branding_nickel_as_codex() {
        assert_eq!(
            controller::codex_attribution("codex-cli 0.149.0"),
            "powered by OpenAI Codex CLI v0.149.0."
        );
        let mut state = ChatState::default();
        state.provenance = "powered by OpenAI Codex CLI v0.149.0.".into();
        let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(
            tree.commands().iter().any(
                |command| matches!(command, PaintCommand::Text { text, .. } if text == "Nickel")
            )
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "powered by OpenAI Codex CLI v0.149.0.")
        ));
        assert!(!tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Nickel Codex")
        ));
    }

    #[test]
    fn file_menu_exposes_existing_new_and_refresh_actions() {
        let state = ChatState::default();
        let mut ui_state = UiStateStore::default();
        let closed = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 900.0, 640.0),
            &mut ui_state,
        );
        let toggle = closed
            .message_rect(&ChatMessage::ToggleFileMenu)
            .expect("File menu");
        let point = Point {
            x: toggle.origin.x + 4.0,
            y: toggle.origin.y + 4.0,
        };
        closed.handle_event(&mut ui_state, UiEvent::PointerPressed(point));
        closed.handle_event(&mut ui_state, UiEvent::PointerReleased(point));
        let open = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 900.0, 640.0),
            &mut ui_state,
        );
        assert!(open.message_rect(&ChatMessage::NewChat).is_some());
        assert!(open.message_rect(&ChatMessage::Refresh).is_some());
    }

    #[test]
    fn remote_host_editor_validates_and_persists_nickel_owned_settings() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("nickel").join("codex-hosts.toml");
        let backend = ReplayBackend::from_json(r#"{"name":"hosts","events":[]}"#).unwrap();
        let mut app = ChatApplication::with_settings(
            BackendMode::Replay {
                backend,
                cwd: directory.path().into(),
            },
            CodexSettings::default(),
            Some(settings_path.clone()),
        );

        app.update(ChatMessage::ManageRemoteHosts);
        app.update(ChatMessage::AddRemoteHost);
        app.update(ChatMessage::RemoteHostIdChanged("workstation".into()));
        app.update(ChatMessage::RemoteHostNameChanged("Workstation".into()));
        app.update(ChatMessage::RemoteHostEndpointChanged(
            "wss://codex.example.test/app-server".into(),
        ));
        app.update(ChatMessage::RemoteHostTokenEnvChanged(
            "NICKEL_CODEX_TOKEN".into(),
        ));
        app.update(ChatMessage::RemoteHostCwdChanged("/projects/nickel".into()));
        app.update(ChatMessage::SaveRemoteHost);

        let persisted = CodexSettings::load(&settings_path).unwrap();
        assert_eq!(persisted.hosts.len(), 1);
        assert_eq!(persisted.hosts[0].name, "Workstation");
        let stored = std::fs::read_to_string(settings_path).unwrap();
        assert!(stored.contains("NICKEL_CODEX_TOKEN"));
        assert!(!stored.contains("fixture-secret"));

        let tree = UiTree::layout(app.view(), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Workstation")
        ));
        assert!(
            tree.message_rect(&ChatMessage::EditRemoteHost("workstation".into()))
                .is_some()
        );
        assert!(
            tree.message_rect(&ChatMessage::RemoveRemoteHost("workstation".into()))
                .is_some()
        );
    }

    #[test]
    fn grouped_sidebar_scrolls_without_crushing_header_or_actions() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.threads = (0..200)
            .map(|index| Thread {
                id: ThreadId(format!("thread-{index}")),
                title: Some(format!("Task {index}")),
                cwd: Some(
                    if index % 2 == 0 {
                        "/projects/nickel"
                    } else {
                        "/projects/galen"
                    }
                    .into(),
                ),
                last_used_at: Some(index as i64),
                turns: Vec::new(),
            })
            .collect();
        let mut ui_state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 600.0),
            &mut ui_state,
        );
        let menu_bar = tree
            .resolved_layout()
            .find(&nickel_ui::UiId::from("root/menu-bar"))
            .expect("menu bar");
        assert_eq!(menu_bar.allocated.size.height, 30.0);
        let title = tree
            .resolved_layout()
            .find(&nickel_ui::UiId::from(
                "root/#1/thread-sidebar/sidebar-title",
            ))
            .expect("sidebar title");
        let actions = tree
            .resolved_layout()
            .find(&nickel_ui::UiId::from(
                "root/#1/thread-sidebar/sidebar-actions",
            ))
            .expect("sidebar actions");
        let projects = tree
            .resolved_layout()
            .find(&nickel_ui::UiId::from(
                "root/#1/thread-sidebar/project-list",
            ))
            .expect("project list");
        assert!(title.allocated.size.height > 10.0);
        assert!(actions.allocated.size.height > 10.0);
        assert!(projects.scroll.is_some_and(|extent| extent.can_scroll()));
        let visible_tasks = tree
            .commands()
            .iter()
            .filter(|command| {
                matches!(command, PaintCommand::Text { text, .. } if text.starts_with("Task "))
            })
            .count();
        assert!(visible_tasks < 20, "{visible_tasks} task titles emitted");

        let message = ChatMessage::SelectThread(ThreadId("thread-199".into()));
        let button = tree.message_rect(&message).expect("first task button");
        let point = Point {
            x: button.origin.x + 2.0,
            y: button.origin.y + 2.0,
        };
        tree.handle_event(&mut ui_state, UiEvent::PointerPressed(point));
        assert_eq!(
            tree.handle_event(&mut ui_state, UiEvent::PointerReleased(point))
                .messages,
            vec![message]
        );

        state.expanded_projects.insert("/projects/nickel".into());
        let expanded = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 600.0),
            &mut ui_state,
        );
        expanded.handle_event(
            &mut ui_state,
            UiEvent::Scroll {
                point: Point {
                    x: projects.allocated.origin.x + 10.0,
                    y: projects.allocated.origin.y + 10.0,
                },
                delta_y: 100_000.0,
            },
        );
        let scrolled = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 600.0),
            &mut ui_state,
        );
        assert!(
            scrolled.commands().iter().any(
                |command| matches!(command, PaintCommand::Text { text, .. } if text == "Task 0")
            )
        );
    }

    #[test]
    fn project_disclosure_emits_toggle_and_changes_its_label() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.threads = (0..11)
            .map(|index| Thread {
                id: ThreadId(format!("thread-{index}")),
                title: Some(format!("Task {index}")),
                cwd: Some("/projects/nickel".into()),
                last_used_at: Some(index as i64),
                turns: Vec::new(),
            })
            .collect();
        let mut ui_state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 900.0),
            &mut ui_state,
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Show 1 more")
        ));
        let toggle = ChatMessage::ToggleProject("/projects/nickel".into());
        let rect = tree.message_rect(&toggle).expect("visible disclosure");
        let point = Point {
            x: rect.origin.x + 2.0,
            y: rect.origin.y + 2.0,
        };
        tree.handle_event(&mut ui_state, UiEvent::PointerPressed(point));
        assert_eq!(
            tree.handle_event(&mut ui_state, UiEvent::PointerReleased(point))
                .messages,
            vec![toggle]
        );

        state.expanded_projects.insert("/projects/nickel".into());
        let expanded = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 900.0),
            &mut ui_state,
        );
        assert!(expanded.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Show less")
        ));
    }

    #[test]
    fn collapsed_project_keeps_its_header_and_hides_its_tasks() {
        let mut state = ChatState::default();
        state.threads = vec![Thread {
            id: ThreadId("thread".into()),
            title: Some("Visible task".into()),
            cwd: Some("/projects/nickel-ui".into()),
            last_used_at: Some(1),
            turns: Vec::new(),
        }];
        let expanded = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(expanded.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "▾  📁  Nickel UI")
        ));
        assert!(expanded.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Visible task")
        ));
        let header_x = expanded
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text == "▾  📁  Nickel UI" => {
                    Some(bounds.origin.x)
                }
                _ => None,
            })
            .expect("project header text");
        let task_x = expanded
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text == "Visible task" => {
                    Some(bounds.origin.x)
                }
                _ => None,
            })
            .expect("task text");
        assert!(task_x >= header_x + 20.0);

        state
            .collapsed_projects
            .insert("/projects/nickel-ui".into());
        let collapsed = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(collapsed.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "▸  📁  Nickel UI")
        ));
        assert!(!collapsed.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Visible task")
        ));
    }

    #[test]
    fn long_project_names_stay_inside_a_single_header_line() {
        let mut state = ChatState::default();
        state.threads = vec![Thread {
            id: ThreadId("thread".into()),
            title: Some("Hidden task".into()),
            cwd: Some("/projects/llama.cpp-turboquant-post20260629".into()),
            last_used_at: Some(1),
            turns: Vec::new(),
        }];
        state
            .collapsed_projects
            .insert("/projects/llama.cpp-turboquant-post20260629".into());

        let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        let header = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text.starts_with("▸  📁  Llama") => {
                    Some((bounds, text))
                }
                _ => None,
            })
            .expect("long project header");
        assert!(header.1.ends_with('…'));
        assert!(header.0.size.height < 24.0);
        assert!(!tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Hidden task")
        ));
    }

    #[test]
    fn long_transcript_cannot_crush_sidebar_or_composer() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.threads = (0..12)
            .map(|index| nickel_codex::Thread {
                id: ThreadId(format!("thread-{index}")),
                title: Some(format!("Conversation number {index}")),
                cwd: None,
                last_used_at: Some(index as i64),
                turns: Vec::new(),
            })
            .collect();
        for index in 0..10 {
            state.items.push_back(ChatItem {
                id: format!("message-{index}"),
                kind: if index % 2 == 0 {
                    ChatItemKind::User
                } else {
                    ChatItemKind::Agent
                },
                text: "A deliberately long paragraph that must wrap within the readable conversation column instead of widening its parent or shrinking fixed application controls. ".repeat(5),
                complete: true,
            });
        }
        for (width, height) in [(640.0, 480.0), (1120.0, 760.0), (2240.0, 1520.0)] {
            let tree = UiTree::layout(view::chat_view(&state), Rect::new(0.0, 0.0, width, height));
            let find = |suffix: &str| {
                tree.resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .expect("named chat layout node")
            };
            let sidebar = find("thread-sidebar");
            let conversation = find("conversation");
            let composer = find("composer");
            let draft = find("chat-draft");
            assert!(sidebar.allocated.size.width >= 259.0);
            assert!(composer.allocated.size.height >= 70.0);
            assert!(draft.allocated.size.height > 0.0);
            assert!(
                conversation.allocated.origin.y + conversation.allocated.size.height
                    <= composer.allocated.origin.y + 0.01
            );
            assert!(composer.allocated.origin.y + composer.allocated.size.height <= height + 0.01);
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
    fn canonical_replay_fixture_projects_its_agent_bubble() {
        let backend = ReplayBackend::from_json(include_str!(
            "../../nickel-codex-fixture/fixtures/basic.json"
        ))
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        wait_until(&mut app, |state| {
            state
                .items
                .iter()
                .any(|item| item.kind == ChatItemKind::Agent && item.text == "fixture response")
        });
        let tree = UiTree::layout(
            view::chat_view(&app.state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "fixture response")
        ));
    }

    #[test]
    fn composer_submit_shortcut_sends_nonblank_drafts() {
        let backend = ReplayBackend::from_json(r#"{"name":"shortcuts","events":[]}"#).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.draft = "first\nsecond".into();
        assert!(app.shortcut(Shortcut::Submit));
        assert!(app.state.draft.is_empty());
        assert_eq!(app.state.items.back().unwrap().kind, ChatItemKind::User);
    }

    #[test]
    fn multiline_paste_normalizes_newlines_without_submitting() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        let mut ui_state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let draft = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/chat-draft"))
            .expect("draft field");
        tree.handle_event(
            &mut ui_state,
            UiEvent::PointerPressed(Point {
                x: draft.allocated.origin.x + 2.0,
                y: draft.allocated.origin.y + 2.0,
            }),
        );
        let pasted = tree.handle_event(
            &mut ui_state,
            UiEvent::TextPaste("one\r\ntwo\rthree\nfour".into()),
        );
        let expected = "one\ntwo\nthree\nfour";
        assert_eq!(
            pasted.messages,
            vec![ChatMessage::DraftChanged(expected.into())]
        );
        assert!(state.items.is_empty());

        state.draft = (0..30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rebuilt = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let composer_viewport = rebuilt
            .resolved_layout()
            .find(&nickel_ui::UiId::from("root/#1/#1/composer/#0"))
            .expect("composer viewport");
        assert!(composer_viewport.allocated.size.height <= 140.0);
        let extent = composer_viewport.scroll.expect("multiline scroll extent");
        assert!(extent.content.height > extent.viewport.height);
        assert!(extent.offset > 0.0);
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
    fn long_transcript_builds_only_the_pinned_virtual_window() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        for index in 0..2_000 {
            state.items.push_back(ChatItem {
                id: format!("message-{index}"),
                kind: ChatItemKind::Agent,
                text: format!("history message {index}"),
                complete: true,
            });
        }

        let mut ui_state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        assert!(tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text.contains("history message 1999"))
        ));
        assert!(!tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text.contains("history message 0"))
        ));
        assert!(
            tree.commands().len() < 500,
            "{} commands",
            tree.commands().len()
        );
        let conversation = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/conversation"))
            .and_then(|node| node.scroll)
            .expect("virtual transcript scroll extent");
        assert!(conversation.content.height > 100_000.0);
    }

    #[test]
    fn virtual_transcript_copies_logical_text_across_offscreen_messages() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        for index in 0..2_000 {
            state.items.push_back(ChatItem {
                id: format!("message-{index}"),
                kind: ChatItemKind::Agent,
                text: format!("history message {index}"),
                complete: true,
            });
        }
        let mut ui_state = UiStateStore::default();
        let tree = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let region_id = tree
            .selection_region_ids()
            .next()
            .expect("transcript selection region")
            .clone();
        ui_state.set_selection_owner(Some(region_id.clone()));
        *ui_state.document_selection_mut(region_id) = DocumentSelection {
            anchor: Some(SelectionEndpoint::new("message-0/label", 0)),
            focus: Some(SelectionEndpoint::new(
                "message-1999/body/0",
                "history message 1999".len(),
            )),
        };

        let copied = tree
            .selected_text(&ui_state)
            .expect("logical transcript selection");
        assert!(copied.starts_with("Codex\nhistory message 0\nCodex"));
        assert!(copied.contains("history message 1000"));
        assert!(copied.ends_with("Codex\nhistory message 1999"));
        assert!(!tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text.contains("history message 0"))
        ));

        let selected = UiTree::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        assert!(selected.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Fill {
                color: 0x315a8f,
                ..
            }
        )));
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
