mod controller;
mod model;
mod view;

pub use controller::{
    BackendMode, ChatController, ControllerCommand, ControllerEvent, create_managed_workspace,
};
pub use model::{ChatItem, ChatItemKind, ChatState, ConnectionStatus, PendingInteraction};
pub use view::{ChatApplication, ChatMessage, ShellRequest};

pub fn shell_application(
    cwd: std::path::PathBuf,
    project_menu: bool,
    thread: Option<nickel_codex::ThreadId>,
    project_id: Option<String>,
) -> Result<ChatApplication, String> {
    let settings_path =
        nickel_codex::CodexSettings::default_path().map_err(|error| error.to_string())?;
    let settings =
        nickel_codex::CodexSettings::load(&settings_path).map_err(|error| error.to_string())?;
    let mode = settings.selected_host().map_or_else(
        || BackendMode::Live {
            choice: nickel_codex::BackendChoice::Automatic,
            cwd: cwd.clone(),
        },
        |host| BackendMode::Remote { host: host.clone() },
    );
    let mut application = ChatApplication::with_settings(mode, settings, Some(settings_path));
    application = if project_menu {
        application.as_shell_project_menu()
    } else {
        application.as_shell_chat(&cwd)
    };
    if let Some(project_id) = project_id {
        application.use_project(cwd, project_id);
    }
    if let Some(thread) = thread {
        application.resume_thread(thread)?;
    }
    Ok(application)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "authenticated-live-tests")]
    use nickel_codex::BackendChoice;
    use nickel_codex::{
        CodexBackend, CodexEvent, CodexSettings, EventKind, ReplayBackend, ServerRequestId, Thread,
        ThreadId, TurnId,
    };
    use nickel_ui::{
        Application, DocumentSelection, Rect, SdlComponentRenderer, SelectionEndpoint,
        SemanticRole, Shortcut, UiEvent, UiFrame, UiStateStore,
    };
    use nickel_ui_testkit::{ActivationVia, Scenario, Selector};

    use super::*;

    fn event(sequence: u64, kind: EventKind) -> ControllerEvent {
        ControllerEvent::Protocol(CodexEvent { sequence, kind })
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
                    command_actions: Vec::new(),
                    initial_text: String::new(),
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
                model: None,
                reasoning_effort: None,
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
                    command_actions: Vec::new(),
                    initial_text: String::new(),
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
                    command_actions: Vec::new(),
                    initial_text: String::new(),
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
    fn repeated_reads_coalesce_into_one_per_turn_activity() {
        let mut state = ChatState::default();
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
        for (sequence, item_id, path) in [
            (2, "read-1", "/project/README.md"),
            (3, "read-2", "/project/README.md"),
            (4, "read-3", "/project/AGENTS.md"),
        ] {
            state.apply(
                1,
                event(
                    sequence,
                    EventKind::ItemStarted {
                        thread_id: Some(ThreadId("t".into())),
                        turn_id: Some(TurnId("turn".into())),
                        item_id: item_id.into(),
                        item_type: "commandExecution".into(),
                        command_actions: vec![nickel_codex::CommandAction::Read {
                            name: path.rsplit('/').next().unwrap().into(),
                            path: path.into(),
                        }],
                        initial_text: String::new(),
                    },
                ),
            );
        }
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].kind, ChatItemKind::Activity);
        assert_eq!(state.items[0].text, "Exploring\nRead 2 files");

        state.apply(
            1,
            event(
                5,
                EventKind::TurnCompleted {
                    thread_id: ThreadId("t".into()),
                    turn_id: TurnId("turn".into()),
                    status: "completed".into(),
                },
            ),
        );
        assert_eq!(state.items[0].text, "Explored\nRead 2 files");
    }

    #[test]
    fn adjacent_agent_items_in_one_turn_share_a_response_card() {
        let mut state = ChatState::default();
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
        for (sequence, item_id, text) in [
            (2, "progress", "I’ll read the README."),
            (6, "final", "Read it completely."),
        ] {
            state.apply(
                1,
                event(
                    sequence,
                    EventKind::ItemStarted {
                        thread_id: Some(ThreadId("t".into())),
                        turn_id: Some(TurnId("turn".into())),
                        item_id: item_id.into(),
                        item_type: "agentMessage".into(),
                        command_actions: Vec::new(),
                        initial_text: String::new(),
                    },
                ),
            );
            state.apply(
                1,
                event(
                    sequence + 1,
                    EventKind::AgentMessageDelta {
                        item_id: item_id.into(),
                        delta: text.into(),
                    },
                ),
            );
            state.apply(
                1,
                event(
                    sequence + 2,
                    EventKind::ItemCompleted {
                        item_id: item_id.into(),
                    },
                ),
            );
            if item_id == "progress" {
                state.apply(
                    1,
                    event(
                        5,
                        EventKind::ItemStarted {
                            thread_id: Some(ThreadId("t".into())),
                            turn_id: Some(TurnId("turn".into())),
                            item_id: "read".into(),
                            item_type: "commandExecution".into(),
                            command_actions: vec![nickel_codex::CommandAction::Read {
                                name: "README.md".into(),
                                path: "/project/README.md".into(),
                            }],
                            initial_text: String::new(),
                        },
                    ),
                );
            }
        }
        assert_eq!(state.items.len(), 2);
        assert_eq!(
            state.items[0].text,
            "I’ll read the README.\n\nRead it completely."
        );
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
                            command_actions: Vec::new(),
                        },
                        nickel_codex::ThreadHistoryItem {
                            id: "agent".into(),
                            item_type: "agentMessage".into(),
                            text: "previous answer".into(),
                            command_actions: Vec::new(),
                        },
                    ],
                }],
                model: Some("fixture-model".into()),
                reasoning_effort: Some("high".into()),
            }),
        );
        assert_eq!(state.selected_thread, Some(thread_id));
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].kind, ChatItemKind::User);
        assert_eq!(state.items[1].text, "previous answer");
        assert_eq!(state.selected_model.as_deref(), Some("fixture-model"));
        assert_eq!(state.selected_reasoning_effort.as_deref(), Some("high"));
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
        for action in ["Approve", "Decline"] {
            let backend = ReplayBackend::from_json(r#"{"name":"approval","events":[]}"#).unwrap();
            let mut app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            app.state = state.clone();
            let mut scenario = Scenario::new(app, 900, 640);
            scenario
                .pointer_activate(&Selector::role_name(SemanticRole::Button, action))
                .unwrap();
            assert!(
                scenario
                    .host_mut()
                    .application_mut()
                    .state
                    .pending
                    .is_empty()
            );
        }
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
            let tree = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, width, height));
            assert!(tree.resolved_layout().nodes().iter().all(|node| {
                node.allocated.origin.x.is_finite()
                    && node.allocated.origin.y.is_finite()
                    && node.allocated.size.width.is_finite()
                    && node.allocated.size.height.is_finite()
                    && node.allocated.size.width >= 0.0
                    && node.allocated.size.height >= 0.0
            }));
            assert!(has_accessible_text(&tree, "Hello"));
        }
    }

    #[test]
    fn shell_project_menu_raster_shows_projects_without_conversations() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.projects = vec![nickel_codex::Project {
            id: "nickel".into(),
            name: "Nickel".into(),
            roots: vec!["/projects/nickel".into()],
        }];
        state.threads = vec![Thread {
            id: ThreadId("available".into()),
            title: Some("Integrate Codex with the shell".into()),
            cwd: Some("/projects/nickel".into()),
            last_used_at: Some(1),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        }];
        state.thread_runtime.insert(
            ThreadId("available".into()),
            nickel_codex::ThreadRuntime {
                project_id: Some("nickel".into()),
                status: nickel_codex::ThreadRuntimeStatus::Idle,
                ..nickel_codex::ThreadRuntime::default()
            },
        );
        state.thread_error = Some("thread/list rejected".into());
        for (width, height) in [(360.0, 420.0), (280.0, 320.0)] {
            let mut ui_state = UiStateStore::default();
            let tree = UiFrame::layout_with_state(
                view::shell_project_menu_view(&state),
                Rect::new(0.0, 0.0, width, height),
                &mut ui_state,
            );
            assert!(tree.resolved_layout().nodes().iter().all(|node| {
                node.allocated.origin.x.is_finite()
                    && node.allocated.origin.y.is_finite()
                    && node.allocated.size.width.is_finite()
                    && node.allocated.size.height.is_finite()
                    && node.allocated.size.width >= 0.0
                    && node.allocated.size.height >= 0.0
            }));
            assert!(
                tree.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Nickel".into(),
                })
                .is_ok()
            );
        }

        let make_app = || {
            let backend =
                ReplayBackend::from_json(r#"{"name":"project-menu","events":[]}"#).unwrap();
            let mut app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            })
            .as_shell_project_menu();
            app.state = state.clone();
            app
        };
        let project = Selector::role_name(SemanticRole::Button, "Nickel");
        for via in [ActivationVia::Keyboard, ActivationVia::Controller] {
            let mut scenario = Scenario::new(make_app(), 360, 420);
            assert!(
                scenario
                    .host()
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Integrate Codex with the shell".into(),
                    })
                    .is_err()
            );
            scenario.activate_via(via, &project).unwrap();
            assert_eq!(
                scenario.host_mut().application_mut().take_shell_requests(),
                vec![ShellRequest::OpenProject {
                    cwd: "/projects/nickel".into(),
                    project_id: "nickel".into(),
                    name: "Nickel".into(),
                    initial_thread: None,
                }]
            );
        }

        let tree = UiFrame::layout(
            view::shell_project_menu_view(&state),
            Rect::new(0.0, 0.0, 360.0, 420.0),
        );
        let mut renderer = SdlComponentRenderer::new(360, 420, 1.0);
        renderer.render(tree.commands());
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-codex-snapshots/project-menu.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(360, 420, |x, y| {
            let pixel = renderer.pixels()[(y * 360 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        })
        .save(output)
        .unwrap();
    }

    #[test]
    fn sidebarless_chat_attributes_the_codex_cli_without_branding_nickel_as_codex() {
        assert_eq!(
            controller::codex_attribution("codex-cli 0.149.0"),
            "powered by OpenAI Codex CLI v0.149.0."
        );
        let mut state = ChatState::default();
        state.provenance = "powered by OpenAI Codex CLI v0.149.0.".into();
        let tree = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(
            !tree
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("thread-sidebar"))
        );
        assert!(has_accessible_text(
            &tree,
            "powered by OpenAI Codex CLI v0.149.0."
        ));
        assert!(!has_accessible_text(&tree, "Nickel Codex"));
    }

    #[test]
    fn file_menu_exposes_existing_new_and_refresh_actions() {
        let backend = ReplayBackend::from_json(r#"{"name":"file-menu","events":[]}"#).unwrap();
        let app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let mut scenario = Scenario::new(app, 900, 640);
        scenario
            .pointer_activate(&Selector::id("root/menu-bar/file-menu"))
            .expect("production semantic menu expansion");
        for name in ["New conversation", "Refresh"] {
            scenario
                .assert_action_available(
                    &Selector::role_name(SemanticRole::MenuItem, name),
                    nickel_ui::ActionKind::Activate,
                )
                .expect("expanded menu item is semantic and actionable");
        }
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

        let mut scenario = Scenario::new(app, 900, 640);
        scenario
            .activate(&Selector::role_name(SemanticRole::Button, "Edit"))
            .unwrap();
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Save host".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn long_transcript_cannot_crush_sidebarless_composer() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.threads = (0..12)
            .map(|index| nickel_codex::Thread {
                id: ThreadId(format!("thread-{index}")),
                title: Some(format!("Conversation number {index}")),
                cwd: None,
                last_used_at: Some(index as i64),
                turns: Vec::new(),
                model: None,
                reasoning_effort: None,
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
            let tree = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, width, height));
            let find = |suffix: &str| {
                tree.resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .expect("named chat layout node")
            };
            let conversation = find("conversation");
            let composer = find("composer");
            let draft = find("chat-draft");
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
            text: "# Shared Markdown\n\nThis is **bold**, *emphasized*, and `inline code`.\n\n- Lists use the shared renderer\n- [Links stay typed](https://example.com)\n\n| Feature | State |\n| --- | --- |\n| Tables | Working |\n\n```rust\nfn integrated() -> bool { true }\n```"
                .into(),
            complete: true,
        });
        for scale in [1.0, 2.0] {
            let tree = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 800.0, 600.0));
            let mut renderer =
                SdlComponentRenderer::new((800.0 * scale) as u32, (600.0 * scale) as u32, scale);
            assert!(!renderer.render(tree.commands()).is_empty());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
            if scale == 1.0 {
                let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/nickel-codex-snapshots/shared-markdown.png");
                std::fs::create_dir_all(output.parent().unwrap()).unwrap();
                let image =
                    image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(800, 600, |x, y| {
                        let pixel = renderer.pixels()[(y * 800 + x) as usize];
                        image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                    });
                image.save(output).unwrap();
            }
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
        let tree = UiFrame::layout(
            view::chat_view(&app.state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
        );
        assert!(has_accessible_text(&tree, "fixture response"));
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
    fn composer_commands_open_pickers_and_confirm_first_shell_execution() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"commands","models":[{"id":"fixture","display_name":"Fixture","default_reasoning_effort":"medium","supported_reasoning_efforts":[{"reasoning_effort":"low","description":"Fast"},{"reasoning_effort":"high","description":"Deep"}]}],"events":[]}"#,
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.poll_controller();

        app.update(ChatMessage::ToggleCommandPicker);
        assert!(app.command_picker_open);
        app.update(ChatMessage::SelectCommand("/model".into()));
        assert!(app.model_picker_open);
        assert!(!app.command_picker_open);
        app.update(ChatMessage::SelectReasoningEffort("high".into()));
        assert_eq!(app.state.selected_reasoning_effort.as_deref(), Some("high"));

        app.state.draft = "/model".into();
        app.update(ChatMessage::Send);
        assert!(app.model_picker_open);
        assert!(app.state.draft.is_empty());

        app.state.draft = "/resume".into();
        app.update(ChatMessage::Send);
        assert!(app.resume_picker_open);
        assert!(!app.model_picker_open);

        app.state.draft = "!printf hello".into();
        app.update(ChatMessage::Send);
        assert_eq!(app.pending_shell_command.as_deref(), Some("printf hello"));
        assert!(app.state.items.is_empty());
        app.update(ChatMessage::ConfirmShell);
        assert!(app.pending_shell_command.is_none());
        assert!(app.shell_warning_acknowledged);
    }

    #[test]
    fn shell_hosted_resume_requests_the_shell_lease_before_controller_resume() {
        let backend = ReplayBackend::from_json(r#"{"name":"resume","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_chat(std::path::Path::new("/projects/nickel"));
        let thread = ThreadId("idle-thread".into());

        app.update(ChatMessage::SelectThread(thread.clone()));

        assert_eq!(
            app.take_shell_requests(),
            vec![ShellRequest::ResumeThread(thread)]
        );
        assert!(app.state.selected_thread.is_none());
    }

    #[test]
    fn chat_overlays_expose_commands_reasoning_and_only_resumable_project_threads() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"overlays","models":[{"id":"fixture","display_name":"Fixture","default_reasoning_effort":"medium","supported_reasoning_efforts":[{"reasoning_effort":"high","description":"Deep reasoning"}]}],"events":[]}"#,
        )
        .unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_chat(std::path::Path::new("/projects/nickel"));
        app.use_project("/projects/nickel".into(), "nickel".into());
        app.state.status = ConnectionStatus::Ready;
        app.state.models = ReplayBackend::from_json(
            r#"{"name":"model","models":[{"id":"fixture","display_name":"Fixture","default_reasoning_effort":"medium","supported_reasoning_efforts":[{"reasoning_effort":"high","description":"Deep reasoning"}]}]}"#,
        )
        .unwrap()
        .models()
        .unwrap();
        app.state.selected_model = Some("fixture".into());
        app.state.selected_reasoning_effort = Some("medium".into());
        for (id, cwd, project_id, status) in [
            (
                "eligible",
                "/projects/nickel",
                "nickel",
                nickel_codex::ThreadRuntimeStatus::Idle,
            ),
            (
                "active",
                "/projects/nickel",
                "nickel",
                nickel_codex::ThreadRuntimeStatus::Active,
            ),
            (
                "other",
                "/projects/nickel",
                "other",
                nickel_codex::ThreadRuntimeStatus::Idle,
            ),
        ] {
            let id = ThreadId(id.into());
            app.state.threads.push(Thread {
                id: id.clone(),
                title: Some(id.0.clone()),
                cwd: Some(cwd.into()),
                last_used_at: Some(1),
                turns: Vec::new(),
                model: None,
                reasoning_effort: None,
            });
            app.state.thread_runtime.insert(
                id,
                nickel_codex::ThreadRuntime {
                    project_id: Some(project_id.into()),
                    status,
                    ..nickel_codex::ThreadRuntime::default()
                },
            );
        }

        app.update(ChatMessage::ToggleCommandPicker);
        let commands = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&commands, "/review — unavailable"));

        app.update(ChatMessage::ToggleCommandPicker);
        app.update(ChatMessage::ToggleModelPicker);
        let models = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&models, "high — Deep reasoning"));

        app.update(ChatMessage::ToggleModelPicker);
        app.update(ChatMessage::ToggleResumePicker);
        let mut scenario = Scenario::new(app, 900, 640);
        let button = |name: &str| nickel_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: name.into(),
        };
        assert!(scenario.host().query_unique(&button("eligible")).is_ok());
        assert!(scenario.host().query_unique(&button("active")).is_err());
        assert!(scenario.host().query_unique(&button("other")).is_err());
        scenario
            .activate(&Selector::role_name(SemanticRole::Button, "eligible"))
            .unwrap();
        assert_eq!(
            scenario.host_mut().application_mut().take_shell_requests(),
            vec![ShellRequest::ResumeThread(ThreadId("eligible".into()))]
        );
    }

    #[test]
    fn multiline_paste_normalizes_newlines_without_submitting() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        let backend = ReplayBackend::from_json(r#"{"name":"paste","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state = state;
        let mut scenario = Scenario::new(app, 1120, 760);
        let draft = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .unwrap();
        scenario
            .host_mut()
            .handle_event(UiEvent::AccessibilityFocus(draft.id));
        scenario
            .host_mut()
            .handle_event(UiEvent::TextPaste("one\r\ntwo\rthree\nfour".into()));
        let expected = "one\ntwo\nthree\nfour";
        let state = &mut scenario.host_mut().application_mut().state;
        assert_eq!(state.draft, expected);
        assert!(state.items.is_empty());

        state.draft = (0..30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut ui_state = UiStateStore::default();
        let rebuilt = UiFrame::layout_with_state(
            view::chat_view(state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let composer_viewport = rebuilt
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("composer/#0"))
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
    fn embedded_project_menu_emits_shell_requests_without_starting_a_process() {
        let backend = ReplayBackend::from_json(r#"{"name":"embedded","events":[]}"#).unwrap();
        let cwd = std::path::PathBuf::from("/projects/nickel");
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: cwd.clone(),
        })
        .as_shell_project_menu();
        app.update(ChatMessage::NewChatIn(cwd.clone(), "project-1".into()));
        assert_eq!(
            app.take_shell_requests(),
            vec![ShellRequest::OpenProject {
                cwd,
                project_id: "project-1".into(),
                name: "nickel".into(),
                initial_thread: None,
            }]
        );
    }

    #[test]
    fn project_menu_never_enters_the_thread_failure_domain() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"projects-only",
                "projects":[{"id":"nickel","name":"Nickel","roots":["/projects/nickel"]}],
                "thread_error":"duplicate thread id"
            }"#,
        )
        .unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_project_menu();
        wait_until(&mut app, |state| state.status == ConnectionStatus::Ready);

        assert_eq!(app.state.projects[0].id, "nickel");
        assert!(app.state.thread_error.is_none());
    }

    #[test]
    fn new_project_chat_never_enters_the_thread_failure_domain() {
        let backend = ReplayBackend::from_json(
            r#"{
                "name":"new-project-chat",
                "projects":[{"id":"sentrygist","name":"sentrygist","roots":["/work/sentrygist"]}],
                "thread_error":"duplicate thread id"
            }"#,
        )
        .unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/work/sentrygist".into(),
        })
        .as_shell_chat(std::path::Path::new("/work/sentrygist"));
        wait_until(&mut app, |state| state.status == ConnectionStatus::Ready);

        assert!(app.state.thread_error.is_none());
        assert!(app.state.diagnostics.is_empty());
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
        let tree = UiFrame::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        assert!(has_accessible_text(&tree, "history message 1999"));
        assert!(!has_accessible_text(&tree, "history message 0"));
        assert!(
            tree.resource_diagnostics().paint_primitive_count < 500,
            "{} paint primitives",
            tree.resource_diagnostics().paint_primitive_count
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
        let tree = UiFrame::layout_with_state(
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
                "markdown-message-1999/body/0",
                "history message 1999".len(),
            )),
        };

        let copied = tree
            .selected_text(&ui_state)
            .expect("logical transcript selection");
        assert!(copied.starts_with("Codex\nhistory message 0\nCodex"));
        assert!(copied.contains("history message 1000"));
        assert!(copied.ends_with("Codex\nhistory message 1999"));
        assert!(!has_accessible_text(&tree, "history message 0"));

        let mut unselected_renderer = SdlComponentRenderer::new(1120, 760, 1.0);
        unselected_renderer.render(tree.commands());
        let unselected_pixels = unselected_renderer.pixels().to_vec();

        let selected = UiFrame::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let mut selected_renderer = SdlComponentRenderer::new(1120, 760, 1.0);
        selected_renderer.render(selected.commands());
        assert_ne!(selected_renderer.pixels(), unselected_pixels.as_slice());
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
                        command_actions: Vec::new(),
                        initial_text: String::new(),
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
    fn crate_manifest_has_no_shell_or_session_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("nickel-shell"));
        assert!(!manifest.contains("nickel-session"));
    }

    #[cfg(feature = "authenticated-live-tests")]
    #[test]
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

    #[cfg(feature = "authenticated-live-tests")]
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
