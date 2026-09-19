mod attachments;
mod controller;
mod model;
mod projection_memory;
mod view;

pub use attachments::{
    AttachmentError, AttachmentId, AttachmentLimits, ClipboardOffer, ClipboardPaste,
    PendingAttachment,
};
pub use controller::{
    BackendMode, ChatController, ControllerCommand, ControllerEvent, create_managed_workspace,
};
pub use model::{ChatItem, ChatItemKind, ChatState, ConnectionStatus, PendingInteraction};
pub use view::{
    ChatApplication, ChatMessage, CodexApprovalChoice, CodexApprovalNotification, ShellRequest,
};

/// Supported logical client minimum shared by the Winit and internal chat hosts.
pub const CHAT_MINIMUM_LOGICAL_SIZE: (u32, u32) = (640, 480);

pub fn shell_application(
    cwd: std::path::PathBuf,
    project_menu: bool,
    thread: Option<nickel_codex::ThreadId>,
    project_id: Option<String>,
) -> Result<ChatApplication, String> {
    shell_application_with_backend(cwd, project_menu, thread, project_id, None)
}

pub fn shell_application_with_backend(
    cwd: std::path::PathBuf,
    project_menu: bool,
    thread: Option<nickel_codex::ThreadId>,
    project_id: Option<String>,
    backend: Option<nickel_codex::BackendChoice>,
) -> Result<ChatApplication, String> {
    let settings_path =
        nickel_codex::CodexSettings::default_path().map_err(|error| error.to_string())?;
    let settings =
        nickel_codex::CodexSettings::load(&settings_path).map_err(|error| error.to_string())?;
    let mode = backend.map_or_else(
        || {
            settings.selected_host().map_or_else(
                || BackendMode::Live {
                    choice: nickel_codex::BackendChoice::Automatic,
                    cwd: cwd.clone(),
                },
                |host| BackendMode::Remote { host: host.clone() },
            )
        },
        |choice| BackendMode::Live {
            choice,
            cwd: cwd.clone(),
        },
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
        CodexBackend, CodexEvent, CodexSettings, EventKind, Model, ReplayBackend, ServerRequestId,
        Thread, ThreadId, TurnId,
    };
    use nickel_ui::{
        Application, DocumentSelection, HostBatch, HostEvent, Rect, SelectionEndpoint,
        SemanticRole, Shortcut, SoftwareRenderer, UiEvent, UiFrame, UiHost, UiId, UiStateStore,
    };
    use nickel_ui_testkit::{ActivationVia, Scenario, Selector};

    fn open_run_settings(scenario: &mut Scenario<ChatApplication>) {
        scenario
            .pointer_activate(&Selector::id("root/menu-bar/codex-menu"))
            .expect("Codex menu opens");
        scenario
            .pointer_activate(&Selector::role_name(SemanticRole::MenuItem, "Run settings"))
            .expect("Run settings opens from Codex menu");
    }

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
        state.account.authenticated = true;
        state.draft = "   ".into();
        assert!(state.begin_send().is_none());
        state.draft = "hello".into();
        assert_eq!(state.begin_send().map(|sent| sent.0), Some("hello".into()));
        assert_eq!(state.items[0].kind, ChatItemKind::User);
    }

    #[test]
    fn new_thread_and_server_user_item_preserve_the_optimistic_message() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        state.account.authenticated = true;
        state.draft = "hello".into();
        assert_eq!(state.begin_send().map(|sent| sent.0), Some("hello".into()));

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
                    completion: None,
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
                    completion: None,
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
    fn activity_between_agent_items_preserves_chronology() {
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
                        completion: None,
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
        assert_eq!(state.items.len(), 3);
        assert_eq!(state.items[0].text, "I’ll read the README.");
        assert_eq!(state.items[1].kind, ChatItemKind::Activity);
        assert_eq!(state.items[2].text, "Read it completely.");
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
                            ..Default::default()
                        },
                        nickel_codex::ThreadHistoryItem {
                            id: "agent".into(),
                            item_type: "agentMessage".into(),
                            text: "previous answer".into(),
                            command_actions: Vec::new(),
                            ..Default::default()
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
        state.status = ConnectionStatus::Ready;
        state.account.authenticated = true;
        state.apply(
            1,
            event(
                1,
                EventKind::ApprovalRequested {
                    request_id: ServerRequestId("approval".into()),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("cargo test".into()),
                    context: nickel_codex::ApprovalContext::default(),
                },
            ),
        );
        assert_eq!(state.pending.len(), 1);
        for action in ["Approve", "Decline", "Cancel"] {
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
            let application = scenario.host_mut().application_mut();
            assert_eq!(application.state.pending.len(), 1);
            assert!(
                application
                    .state
                    .interaction_submission_pending(&ServerRequestId("approval".into()))
            );
            application.state.apply(
                1,
                event(
                    2,
                    EventKind::ServerRequestResolved {
                        thread_id: ThreadId("thread".into()),
                        request_id: ServerRequestId("approval".into()),
                    },
                ),
            );
            assert!(application.state.pending.is_empty());
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
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
        ] {
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
        let mut renderer = SoftwareRenderer::new(360, 420, 1.0);
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
        assert!(!has_accessible_text(
            &tree,
            "powered by OpenAI Codex CLI v0.149.0."
        ));
        assert!(!has_accessible_text(&tree, "Nickel Codex"));
    }

    #[test]
    fn app_chrome_exposes_file_and_codex_actions() {
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
        scenario
            .pointer_activate(&Selector::id("root/menu-bar/codex-menu"))
            .expect("Codex actions open from app chrome");
        for name in ["Run settings", "Phone access…", "Diagnostics…"] {
            scenario
                .assert_action_available(
                    &Selector::role_name(SemanticRole::MenuItem, name),
                    nickel_ui::ActionKind::Activate,
                )
                .expect("expanded Codex action is semantic and actionable");
        }
    }

    #[test]
    fn phone_access_panel_separates_codex_pairing_from_nickel_desktop_authority() {
        let backend = ReplayBackend::from_json(r#"{"name":"phone-access","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.account.authenticated = true;
        app.state.remote_control_status = Some(nickel_codex::RemoteControlStatus {
            status: nickel_codex::RemoteControlConnectionStatus::Connected,
            server_name: "workstation".into(),
            installation_id: "installation-1".into(),
            environment_id: Some("environment-1".into()),
        });
        app.update(ChatMessage::OpenRemoteControl);
        app.state.remote_control_pending = false;
        let scenario = Scenario::new(app, 900, 700);
        scenario
            .assert_action_available(
                &Selector::role_name(SemanticRole::Button, "Pair a phone"),
                nickel_ui::ActionKind::Activate,
            )
            .unwrap();
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
        state.account.authenticated = true;
        state.draft = "How are you doing?".into();
        state.models = vec![Model {
            id: "gpt-5.6-sol".into(),
            display_name: "GPT-5.6-Sol".into(),
            default_reasoning_effort: Some("low".into()),
            supported_reasoning_efforts: vec![nickel_codex::ReasoningEffortOption {
                reasoning_effort: "low".into(),
                description: "Fast".into(),
            }],
        }];
        state.selected_model = Some("gpt-5.6-sol".into());
        state.selected_reasoning_effort = Some("low".into());
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
        state.thread_runtime.insert(
            ThreadId("thread-0".into()),
            nickel_codex::ThreadRuntime {
                status: nickel_codex::ThreadRuntimeStatus::Idle,
                ..Default::default()
            },
        );
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
        for (width, height) in [
            (640.0, 480.0),
            (1120.0, 760.0),
            (1318.0, 889.0),
            (2240.0, 1520.0),
        ] {
            let tree = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, width, height));
            let find = |suffix: &str| {
                tree.resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .expect("named chat layout node")
            };
            let conversation = find("conversation");
            let transcript = find("transcript-column");
            let composer = find("composer");
            let draft = find("chat-draft");
            let status = find("composer-status");
            let send = find("send-button");
            assert!(composer.allocated.size.height >= 70.0);
            assert!(transcript.allocated.size.width <= conversation.allocated.size.width + 0.01);
            assert!(
                transcript.allocated.size.width >= conversation.allocated.size.width - 36.01,
                "transcript must use the conversation width at {width}×{height}: {:?}",
                transcript.allocated
            );
            let conversation_center =
                conversation.allocated.origin.x + conversation.allocated.size.width / 2.0;
            let transcript_center =
                transcript.allocated.origin.x + transcript.allocated.size.width / 2.0;
            assert!((transcript_center - conversation_center).abs() <= 1.0);
            assert!(draft.allocated.size.height > 0.0);
            assert!(
                conversation.allocated.origin.y + conversation.allocated.size.height
                    <= composer.allocated.origin.y + 0.01
            );
            assert!(composer.allocated.origin.y + composer.allocated.size.height <= height + 0.01);
            assert!(status.allocated.size.height > 0.0);
            assert!(
                send.allocated.size.width >= 48.0,
                "send: {:?}",
                send.allocated
            );
            assert!(
                send.allocated.origin.x + send.allocated.size.width <= width + 0.01,
                "send escapes the edge-to-edge composer: {:?}",
                send.allocated
            );
        }
    }

    #[test]
    fn footer_resume_is_contextual() {
        let mut state = ChatState::default();
        state.status = ConnectionStatus::Ready;
        let empty = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(
            !empty
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("resume-button"))
        );
        state.threads.push(Thread {
            id: ThreadId("saved".into()),
            title: Some("Saved work".into()),
            cwd: None,
            last_used_at: None,
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        });
        state.thread_runtime.insert(
            ThreadId("saved".into()),
            nickel_codex::ThreadRuntime {
                status: nickel_codex::ThreadRuntimeStatus::Idle,
                ..Default::default()
            },
        );
        let resumable = UiFrame::layout(view::chat_view(&state), Rect::new(0.0, 0.0, 900.0, 640.0));
        assert!(
            resumable
                .resolved_layout()
                .nodes()
                .iter()
                .all(|node| !node.id.as_str().ends_with("resume-button"))
        );
    }

    #[test]
    fn streamed_content_offscreen_offers_explicit_jump_without_auto_following() {
        let backend = ReplayBackend::from_json(r#"{"name":"scroll","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        for index in 0..24 {
            app.state.items.push_back(ChatItem {
                id: format!("item-{index}"),
                kind: ChatItemKind::Agent,
                text: "A paragraph of work in progress. ".repeat(20),
                complete: index != 23,
            });
        }
        app.update(ChatMessage::ConversationScrolled(nickel_ui::ScrollExtent {
            viewport: nickel_ui::Size::new(900.0, 400.0),
            content: nickel_ui::Size::new(900.0, 3000.0),
            offset_x: 0.0,
            offset: 0.0,
        }));
        assert!(!app.state.conversation_pinned);
        assert!(!app.state.new_content_while_unpinned);
        app.state.apply(
            app.state.generation,
            event(
                1,
                EventKind::AgentMessageDelta {
                    item_id: "item-23".into(),
                    delta: " More streamed output.".into(),
                },
            ),
        );
        assert!(!app.state.conversation_pinned);
        assert!(app.state.new_content_while_unpinned);
        let mut scenario = Scenario::new(app, 900, 640);
        scenario
            .pointer_activate(&Selector::role_name(SemanticRole::Button, "Jump to latest"))
            .unwrap();
        assert!(scenario.host().application().state.conversation_pinned);
        assert!(
            !scenario
                .host()
                .application()
                .state
                .new_content_while_unpinned
        );
        let measured = nickel_ui::ScrollExtent {
            viewport: nickel_ui::Size::new(900.0, 400.0),
            content: nickel_ui::Size::new(900.0, 3000.0),
            offset_x: 0.0,
            offset: 2579.0,
        };
        scenario
            .host_mut()
            .application_mut()
            .update(ChatMessage::ConversationScrolled(measured));
        assert!(!scenario.host().application().state.conversation_pinned);
        scenario
            .host_mut()
            .application_mut()
            .update(ChatMessage::ConversationScrolled(nickel_ui::ScrollExtent {
                offset: 2580.0,
                ..measured
            }));
        assert!(scenario.host().application().state.conversation_pinned);
    }

    #[test]
    fn compact_composer_retains_primary_action_and_reveals_all_run_settings() {
        let backend = ReplayBackend::from_json(r#"{"name":"compact","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "Run the tests".into();
        app.state.models.push(Model {
            id: "model".into(),
            display_name: "A very long model name".into(),
            default_reasoning_effort: Some("high".into()),
            supported_reasoning_efforts: vec![nickel_codex::ReasoningEffortOption {
                reasoning_effort: "high".into(),
                description: "High effort".into(),
            }],
        });
        app.state.selected_model = Some("model".into());
        let rect = Rect::new(0.0, 0.0, 640.0, 480.0);
        let layout = |app: &ChatApplication| {
            UiFrame::layout(
                app.view(nickel_ui::ViewContext::new(
                    rect,
                    nickel_ui::InputModality::Keyboard,
                )),
                rect,
            )
        };
        let closed = layout(&app);
        let has = |tree: &UiFrame<ChatMessage>, suffix: &str| {
            tree.resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with(suffix))
        };
        assert!(!has(&closed, "run-settings-button"));
        assert!(has(&closed, "send-button"));
        assert!(!has(&closed, "model-selector"));
        let mut scenario = Scenario::new(app, 640, 480);
        open_run_settings(&mut scenario);
        let app = scenario.host().application();
        let open = layout(app);
        for id in [
            "model-selector",
            "reasoning-effort-selector",
            "approval-policy-selector",
            "close-run-settings",
        ] {
            assert!(has(&open, id), "missing {id}");
        }
        assert!(has(&open, "send-button"));
        assert_eq!(app.state.draft, "Run the tests");
        let find = |suffix: &str| {
            open.resolved_layout()
                .nodes()
                .iter()
                .find(|node| node.id.as_str().ends_with(suffix))
                .unwrap()
                .allocated
        };
        let conversation = find("conversation");
        let composer = find("composer");
        let settings = find("compact-run-settings");
        let action = find("send-button");
        assert!(
            conversation.size.height >= 120.0,
            "conversation: {conversation:?}, settings: {settings:?}, composer: {composer:?}"
        );
        assert!(composer.origin.y + composer.size.height <= rect.size.height + 0.01);
        assert!(
            action.origin.x + action.size.width <= composer.origin.x + composer.size.width + 0.01
        );
        for (width, height) in [
            (960.0, 540.0),
            (1120.0, 760.0),
            (1279.0, 720.0),
            (1280.0, 720.0),
            (1366.0, 768.0),
            (1920.0, 1080.0),
            (640.0, 960.0),
        ] {
            let area = Rect::new(0.0, 0.0, width, height);
            let frame = UiFrame::layout(
                app.view(nickel_ui::ViewContext::new(
                    area,
                    nickel_ui::InputModality::Keyboard,
                )),
                area,
            );
            assert!(!has(&frame, "run-settings-button"), "width {width}");
            assert!(has(&frame, "send-button"), "width {width}");
            assert!(has(&frame, "model-selector"), "width {width}");
            let find = |suffix: &str| {
                frame
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .unwrap()
                    .allocated
            };
            let transcript = find("conversation");
            let composer = find("composer");
            let status = find("composer-status");
            let action = find("send-button");
            let model = find("model-selector");
            let approval = find("approval-policy-selector");
            assert!(
                transcript.size.height >= 120.0,
                "{width}×{height}: {transcript:?}"
            );
            assert!(composer.origin.y + composer.size.height <= height + 0.01);
            assert!(
                status.origin.x + status.size.width
                    <= composer.origin.x + composer.size.width + 0.01
            );
            assert!(
                action.origin.x + action.size.width
                    <= composer.origin.x + composer.size.width + 0.01
            );
            assert!(
                model.origin.x + model.size.width <= composer.origin.x + composer.size.width + 0.01
            );
            if width >= 900.0 {
                assert_eq!(model.origin.y, approval.origin.y, "width {width}");
            } else {
                assert!(approval.origin.y > model.origin.y, "width {width}");
            }
        }
        scenario
            .pointer_activate(&Selector::role_name(
                SemanticRole::Button,
                "Close run settings",
            ))
            .expect("graphical close control dismisses run settings");
        assert!(!has(
            &layout(scenario.host().application()),
            "model-selector"
        ));
    }

    #[test]
    fn resizing_across_settings_collapse_retains_primary_action_and_rehomes_popup() {
        let backend = ReplayBackend::from_json(r#"{"name":"resize","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "Keep this prompt".into();
        app.state.models.push(Model {
            id: "long-model".into(),
            display_name: "Long model name requiring compact layout".into(),
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
        });
        app.state.models.push(Model {
            id: "alternative".into(),
            display_name: "Alternative model".into(),
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
        });
        app.state.selected_model = Some("long-model".into());
        let mut scenario = Scenario::new(app, 1280, 720);
        open_run_settings(&mut scenario);
        scenario
            .pointer_activate(&Selector::role_name(SemanticRole::Button, "Model selector"))
            .unwrap();
        assert!(scenario.host().application().model_picker_generation > 0);
        let wide_option = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::MenuItem,
                name: "Alternative model".into(),
            })
            .expect("open model option");
        assert!(wide_option.bounds.origin.x >= 0.0);
        assert!(wide_option.bounds.origin.y >= 0.0);
        assert!(wide_option.bounds.origin.x + wide_option.bounds.size.width <= 1280.0);
        assert!(wide_option.bounds.origin.y + wide_option.bounds.size.height <= 720.0);
        scenario.host_mut().resize(1279, 540);
        // Menu-owned settings persist across the width threshold; the open
        // choice must be rehomed within the smaller client area.
        let resized_option = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::MenuItem,
                name: "Alternative model".into(),
            })
            .expect("model option remains reachable after resize");
        assert!(resized_option.bounds.origin.x >= 0.0);
        assert!(resized_option.bounds.origin.y >= 0.0);
        assert!(resized_option.bounds.origin.x + resized_option.bounds.size.width <= 1279.0);
        assert!(resized_option.bounds.origin.y + resized_option.bounds.size.height <= 540.0);
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Send".into()
                })
                .is_ok()
        );
        assert_eq!(
            scenario.host().application().state.draft,
            "Keep this prompt"
        );
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Model selector".into()
                })
                .is_ok()
        );
        let compact_option = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::MenuItem,
                name: "Alternative model".into(),
            })
            .expect("compact model option");
        assert!(compact_option.bounds.origin.x >= 0.0);
        assert!(compact_option.bounds.origin.y >= 0.0);
        assert!(compact_option.bounds.origin.x + compact_option.bounds.size.width <= 1279.0);
        assert!(compact_option.bounds.origin.y + compact_option.bounds.size.height <= 540.0);
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
                SoftwareRenderer::new((800.0 * scale) as u32, (600.0 * scale) as u32, scale);
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
    fn two_hundred_percent_output_uses_960_by_540_logical_client_geometry() {
        let backend = ReplayBackend::from_json(r#"{"name":"high-scale","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "A prompt at high scale".into();
        let mut host = UiHost::new(app, 960, 540);
        host.set_scale_factor(2.0);
        assert_eq!(host.inspect().scale_factor, 2.0);
        assert_eq!(host.render_frame().logical_size, (960, 540));
        let send = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Send".into(),
            })
            .expect("send at high scale");
        assert!(send.bounds.origin.x >= 0.0);
        assert!(send.bounds.origin.y >= 0.0);
        assert!(send.bounds.origin.x + send.bounds.size.width <= 960.0);
        assert!(send.bounds.origin.y + send.bounds.size.height <= 540.0);
        let mut renderer = SoftwareRenderer::new(1920, 1080, 2.0);
        assert!(!host.render_software(&mut renderer).is_empty());
    }

    #[test]
    fn content_stress_preserves_transcript_and_composer_at_required_viewports() {
        let backend = ReplayBackend::from_json(r#"{"name":"stress","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.draft = "A long unsent prompt. ".repeat(280);
        app.state.models.push(Model {
            id: "long-label".into(),
            display_name: "A particularly long model label that must not widen the composer".into(),
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
        });
        app.state.selected_model = Some("long-label".into());
        let thread_id = ThreadId("stress-thread".into());
        app.state.selected_thread = Some(thread_id.clone());
        app.state.threads.push(Thread {
            id: thread_id,
            title: Some("Very long thread title ".repeat(60)),
            cwd: Some("/projects/nickel".into()),
            last_used_at: None,
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        });
        for (kind, text) in [
            (ChatItemKind::User, "Long user prose. ".repeat(180)),
            (ChatItemKind::Agent, "Long assistant prose. ".repeat(240)),
            (
                ChatItemKind::Command,
                format!(
                    "$ {}\n{}",
                    "very-long-command ".repeat(50),
                    "output ".repeat(300)
                ),
            ),
            (
                ChatItemKind::FileChange,
                format!(
                    "/very/long/path/{}\n{}",
                    "nested/".repeat(80),
                    "+ changed line\n".repeat(160)
                ),
            ),
            (ChatItemKind::Error, "Backend error detail. ".repeat(160)),
            (
                ChatItemKind::Unknown("future-event".into()),
                "Unknown event summary. ".repeat(120),
            ),
        ] {
            app.state.items.push_back(ChatItem {
                id: format!("stress-{}", app.state.items.len()),
                kind,
                text,
                complete: true,
            });
        }
        for (width, height) in [
            (1920.0, 1080.0),
            (1366.0, 768.0),
            (1280.0, 720.0),
            (1120.0, 760.0),
            (960.0, 540.0),
            (640.0, 480.0),
            (640.0, 960.0),
        ] {
            let area = Rect::new(0.0, 0.0, width, height);
            let frame = UiFrame::layout(
                app.view(nickel_ui::ViewContext::new(
                    area,
                    nickel_ui::InputModality::Keyboard,
                )),
                area,
            );
            let bounds = |suffix: &str| {
                frame
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.id.as_str().ends_with(suffix))
                    .unwrap_or_else(|| panic!("missing {suffix} at {width}×{height}"))
                    .allocated
            };
            let transcript = bounds("conversation");
            let composer = bounds("composer");
            let status = bounds("composer-status");
            let send = bounds("send-button");
            assert!(
                transcript.size.height >= 120.0,
                "{width}×{height}: {transcript:?}"
            );
            assert!(
                composer.origin.y >= 0.0
                    && composer.origin.y + composer.size.height <= height + 0.01
            );
            assert!(send.origin.x >= 0.0 && send.origin.x + send.size.width <= width + 0.01);
            assert!(send.origin.y >= 0.0 && send.origin.y + send.size.height <= height + 0.01);
            assert!(
                status.origin.x >= composer.origin.x
                    && status.origin.x + status.size.width
                        <= composer.origin.x + composer.size.width + 0.01,
                "status escapes composer at {width}×{height}: {status:?} within {composer:?}"
            );
            assert!(
                frame
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .any(|node| node.id.as_str().ends_with("codex-menu")),
                "Codex settings menu missing at {width}×{height}"
            );
        }
    }

    #[test]
    fn pending_approval_remains_actionable_through_live_collapse_and_resize() {
        let backend =
            ReplayBackend::from_json(r#"{"name":"approval-resize","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.apply(
            app.state.generation,
            event(
                1,
                EventKind::ApprovalRequested {
                    request_id: ServerRequestId("approval-resize".into()),
                    thread_id: Some(ThreadId("thread".into())),
                    approval_type: "item/commandExecution/requestApproval".into(),
                    summary: Some("Review this command".into()),
                    context: nickel_codex::ApprovalContext {
                        command: Some("cargo test ".repeat(80)),
                        cwd: Some("/projects/nickel".into()),
                        ..Default::default()
                    },
                },
            ),
        );
        let mut scenario = Scenario::new(app, 1280, 720);
        for (width, height) in [(1280, 720), (1279, 540), (640, 480), (960, 540)] {
            scenario.host_mut().resize(width, height);
            assert!(
                scenario
                    .host()
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Approve".into(),
                    })
                    .is_ok(),
                "approval lost at {width}×{height}"
            );
            assert!(
                scenario
                    .host()
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Decline".into(),
                    })
                    .is_ok(),
                "decline lost at {width}×{height}"
            );
            assert_eq!(scenario.host().application().state.pending.len(), 1);
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
        app.state.account.authenticated = true;
        app.state.draft = "first\nsecond".into();
        assert!(app.shortcut_outcome(Shortcut::Submit).changed);
        // Submission is staged until the backend acknowledges TurnStarted, so a failed
        // transport cannot destroy the user's draft or attachments.
        assert_eq!(app.state.draft, "first\nsecond");
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
        app.state.account.authenticated = true;

        app.state.draft = "/".into();
        assert_eq!(app.state.draft, "/");
        let generation = app.model_picker_generation;
        app.update(ChatMessage::SelectCommand("/model".into()));
        assert!(app.model_picker_generation > generation);
        assert!(app.state.draft.is_empty());
        let model_frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(
            model_frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| { node.id.as_str().ends_with("model-selector") })
        );
        app.update(ChatMessage::SelectReasoningEffort("high".into()));
        assert_eq!(app.state.selected_reasoning_effort.as_deref(), Some("high"));

        app.state.draft = "/model".into();
        let generation = app.model_picker_generation;
        app.update(ChatMessage::Send);
        assert!(app.model_picker_generation > generation);
        assert!(app.state.draft.is_empty());

        app.state.draft = "/resume".into();
        app.update(ChatMessage::Send);
        assert!(app.resume_picker_open);

        app.update(ChatMessage::SelectCommand("/permissions".into()));
        assert!(app.state.draft.is_empty());
        let permissions_frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(
            permissions_frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| { node.id.as_str().ends_with("approval-policy-selector") })
        );

        app.update(ChatMessage::SelectCommand("/status".into()));
        assert!(app.state.draft.is_empty());
        let status_frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(
            status_frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| { node.id.as_str().ends_with("codex-diagnostics") })
        );

        app.state.draft = "!printf hello".into();
        app.update(ChatMessage::Send);
        assert_eq!(app.pending_shell_command.as_deref(), Some("printf hello"));
        assert!(app.state.items.is_empty());
        app.update(ChatMessage::ConfirmShell);
        assert!(app.pending_shell_command.is_none());
        assert!(app.shell_warning_acknowledged);
    }

    #[test]
    fn compact_command_dispatches_to_the_selected_thread_without_sending_a_prompt() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"compact","threads":[{"id":"recent","title":"Chosen","cwd":"/projects/nickel"}],"thread_runtime":{"recent":{"project_id":"nickel","status":"Idle","active_flags":[],"can_accept_direct_input":true}},"events":[]}"#,
        )
        .unwrap();
        let backend_probe = backend.clone();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        for _ in 0..100 {
            app.poll_controller();
            if app.state.status == ConnectionStatus::Ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.update(ChatMessage::SelectThread(ThreadId("recent".into())));
        for _ in 0..100 {
            app.poll_controller();
            if app.state.selected_thread.as_ref() == Some(&ThreadId("recent".into())) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(app.state.selected_thread, Some(ThreadId("recent".into())));
        app.update(ChatMessage::SelectCommand("/compact".into()));
        for _ in 0..100 {
            app.poll_controller();
            if !backend_probe.compacted_threads().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            backend_probe.compacted_threads(),
            vec![ThreadId("recent".into())]
        );
        assert!(backend_probe.started_turns().is_empty());
        assert!(app.state.draft.is_empty());

        app.update(ChatMessage::SelectCommand("/review".into()));
        for _ in 0..100 {
            app.poll_controller();
            if !backend_probe.reviewed_threads().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(
            backend_probe.reviewed_threads(),
            vec![ThreadId("recent".into())]
        );
        assert!(backend_probe.started_turns().is_empty());
    }

    #[test]
    fn logout_command_uses_account_api_and_refreshes_authentication() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"logout","account":{"authenticated":true,"account_type":"chatgpt","email":null},"events":[]}"#,
        )
        .unwrap();
        let backend_probe = backend.clone();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        for _ in 0..100 {
            app.poll_controller();
            if app.state.account.authenticated {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.state.account.authenticated);
        app.update(ChatMessage::SelectCommand("/logout".into()));
        for _ in 0..100 {
            app.poll_controller();
            if backend_probe.was_logged_out() && !app.state.account.authenticated {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(backend_probe.was_logged_out());
        assert!(!app.state.account.authenticated);
        assert!(app.state.draft.is_empty());
    }

    #[test]
    fn status_command_reads_and_displays_backend_rate_limits() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"status","rate_limits":{"ordinary_usage_allowed":true,"buckets":[{"name":"Codex","primary_used_percent":25,"secondary_used_percent":40}]},"events":[]}"#,
        )
        .unwrap();
        let backend_probe = backend.clone();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.update(ChatMessage::SelectCommand("/status".into()));
        for _ in 0..100 {
            app.poll_controller();
            if app.state.rate_limits.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(backend_probe.rate_limit_reads(), 1);
        assert_eq!(
            app.state.rate_limits.as_ref().unwrap().buckets[0].primary_used_percent,
            Some(25)
        );
        let frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&frame, "Ordinary usage allowed"));
        assert!(has_accessible_text(&frame, "Codex: primary 25% used"));
    }

    #[test]
    fn model_dropdown_commits_before_blur_and_keeps_the_committed_presentation() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"models","models":[{"id":"first","display_name":"First","supported_reasoning_efforts":[]},{"id":"second","display_name":"Second","supported_reasoning_efforts":[]}],"events":[]}"#,
        )
        .unwrap();
        let backend_probe = backend.clone();
        let directory = tempfile::tempdir().unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: directory.path().into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.poll_controller();
        app.state.account.authenticated = true;
        app.state.models = vec![
            Model {
                id: "first".into(),
                display_name: "First".into(),
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
            },
            Model {
                id: "second".into(),
                display_name: "Second".into(),
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
            },
        ];
        app.state.selected_model = Some("first".into());
        let mut scenario = Scenario::new(app, 900, 640);

        open_run_settings(&mut scenario);
        scenario
            .pointer_activate(&Selector::role_name(SemanticRole::Button, "Model selector"))
            .unwrap();
        scenario
            .pointer_activate(&Selector::role_name(SemanticRole::MenuItem, "Second"))
            .unwrap();
        assert_eq!(
            scenario
                .host()
                .application()
                .state
                .selected_model
                .as_deref(),
            Some("second")
        );

        scenario.host_mut().handle_event(UiEvent::FocusLost);
        assert_eq!(
            scenario
                .host()
                .application()
                .state
                .selected_model
                .as_deref(),
            Some("second")
        );
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::Id(UiId::from(
                    "model-selector/option-1"
                )))
                .is_err()
        );
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Model selector".into(),
                })
                .is_ok()
        );

        {
            let application = scenario.host_mut().application_mut();
            application.state.draft = "use the committed model".into();
            application.update(ChatMessage::Send);
        }
        for _ in 0..100 {
            scenario.host_mut().application_mut().poll_controller();
            if !backend_probe.started_turns().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let turns = backend_probe.started_turns();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].model.as_deref(), Some("second"));
    }

    #[test]
    fn approval_policy_requires_selection_and_only_persists_after_acceptance() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("nickel").join("codex-hosts.toml");
        let backend = ReplayBackend::from_json(r#"{"name":"policy","events":[]}"#).unwrap();
        let mut settings = CodexSettings::default();
        settings.approval_policy = nickel_codex::ApprovalPolicy::Untrusted;
        let mut app = ChatApplication::with_settings(
            BackendMode::Replay {
                backend,
                cwd: directory.path().into(),
            },
            settings,
            Some(settings_path.clone()),
        );

        assert_eq!(
            app.state.effective_approval_policy,
            nickel_codex::ApprovalPolicy::Untrusted
        );
        app.update(ChatMessage::SelectApprovalPolicy(
            nickel_codex::ApprovalPolicy::Never,
        ));
        assert_eq!(
            app.state.selected_approval_policy,
            nickel_codex::ApprovalPolicy::Never
        );
        assert_eq!(
            app.state.effective_approval_policy,
            nickel_codex::ApprovalPolicy::Untrusted
        );
        assert!(!settings_path.exists());

        app.accept_approval_policy(nickel_codex::ApprovalPolicy::Never);
        assert_eq!(
            app.state.effective_approval_policy,
            nickel_codex::ApprovalPolicy::Never
        );
        assert_eq!(
            CodexSettings::load(&settings_path).unwrap().approval_policy,
            nickel_codex::ApprovalPolicy::Never
        );
    }

    #[test]
    fn never_ask_does_not_grant_full_access_but_yolo_sends_both_authorities() {
        for (full_access, expected_sandbox) in [
            (false, None),
            (true, Some(nickel_codex::SandboxPolicy::DangerFullAccess)),
        ] {
            let backend = ReplayBackend::from_json(r#"{"name":"sandbox","events":[]}"#).unwrap();
            let backend_probe = backend.clone();
            let mut app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            app.poll_controller();
            app.state.status = ConnectionStatus::Ready;
            app.state.account.authenticated = true;
            app.update(ChatMessage::SelectApprovalPolicy(
                nickel_codex::ApprovalPolicy::Never,
            ));
            if full_access {
                app.update(ChatMessage::SelectSandboxPolicy(expected_sandbox));
            }
            app.state.draft = "check authority".into();
            app.update(ChatMessage::Send);
            for _ in 0..100 {
                app.poll_controller();
                if !backend_probe.started_turns().is_empty() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let turns = backend_probe.started_turns();
            assert_eq!(turns.len(), 1);
            assert_eq!(
                turns[0].approval_policy,
                nickel_codex::ApprovalPolicy::Never
            );
            assert_eq!(turns[0].sandbox_policy, expected_sandbox);
        }
    }

    #[test]
    fn yolo_is_an_explicit_menu_owned_unsandboxed_choice() {
        let backend = ReplayBackend::from_json(r#"{"name":"yolo-menu","events":[]}"#).unwrap();
        let app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        let mut scenario = Scenario::new(app, 900, 640);
        open_run_settings(&mut scenario);
        scenario
            .pointer_activate(&Selector::role_name(
                SemanticRole::Button,
                "Sandbox access selector",
            ))
            .unwrap();
        scenario
            .pointer_activate(&Selector::role_name(
                SemanticRole::MenuItem,
                "YOLO — full access, unsandboxed",
            ))
            .unwrap();
        let state = &scenario.host().application().state;
        assert_eq!(
            state.selected_sandbox_policy,
            Some(nickel_codex::SandboxPolicy::DangerFullAccess)
        );
        assert_eq!(
            state.selected_approval_policy,
            nickel_codex::ApprovalPolicy::Never
        );
    }

    #[test]
    fn every_approval_policy_transition_is_staged_then_persisted_after_acceptance() {
        use nickel_codex::ApprovalPolicy::{Never, OnFailure, OnRequest, Untrusted};

        for (index, (from, to)) in [Untrusted, OnFailure, OnRequest, Never]
            .into_iter()
            .flat_map(|from| [Untrusted, OnFailure, OnRequest, Never].map(|to| (from, to)))
            .enumerate()
        {
            let directory = tempfile::tempdir().unwrap();
            let settings_path = directory.path().join(format!("settings-{index}.toml"));
            let backend = ReplayBackend::from_json(r#"{"name":"policy","events":[]}"#).unwrap();
            let mut settings = CodexSettings::default();
            settings.approval_policy = from;
            let mut app = ChatApplication::with_settings(
                BackendMode::Replay {
                    backend,
                    cwd: directory.path().into(),
                },
                settings,
                Some(settings_path.clone()),
            );

            app.update(ChatMessage::SelectApprovalPolicy(to));
            assert_eq!(app.state.effective_approval_policy, from);
            assert!(!settings_path.exists());
            app.accept_approval_policy(to);
            assert_eq!(app.state.effective_approval_policy, to);
            assert_eq!(
                CodexSettings::load(&settings_path).unwrap().approval_policy,
                to
            );
        }
    }

    #[test]
    fn standalone_and_embedded_hosts_accept_images_and_release_them_on_conversation_change() {
        for embedded in [false, true] {
            let backend = ReplayBackend::from_json(r#"{"name":"images","events":[]}"#).unwrap();
            let app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            let mut app = if embedded {
                app.as_shell_chat(std::path::Path::new("/projects/nickel"))
            } else {
                app
            };
            assert!(nickel_ui::Application::paste_clipboard_image(
                &mut app,
                1,
                1,
                &[1, 2, 3, 255]
            ));
            assert_eq!(app.state.attachments.len(), 1);
            let preview = std::sync::Arc::downgrade(&app.state.attachments[0].preview);
            app.state.new_chat();
            assert!(app.state.attachments.is_empty());
            assert!(preview.upgrade().is_none());
        }
    }

    #[test]
    fn attachment_removal_is_pointer_keyboard_controller_and_accessibility_reachable() {
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ] {
            let backend =
                ReplayBackend::from_json(r#"{"name":"remove-image","events":[]}"#).unwrap();
            let mut app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            app.state.attach_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
            let mut scenario = Scenario::new(app, 900, 640);
            scenario
                .activate_via(
                    via,
                    &Selector::role_name(SemanticRole::Button, "Remove image attachment 1"),
                )
                .unwrap();
            assert!(scenario.host().application().state.attachments.is_empty());
        }
    }

    #[test]
    fn blur_rerender_and_outstanding_approval_do_not_apply_a_staged_policy() {
        let backend = ReplayBackend::from_json(r#"{"name":"policy-focus","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.effective_approval_policy = nickel_codex::ApprovalPolicy::Untrusted;
        app.state.selected_approval_policy = nickel_codex::ApprovalPolicy::Untrusted;
        app.state.pending.push(PendingInteraction::Approval {
            request_id: ServerRequestId("pending".into()),
            approval_type: "commandExecution".into(),
            context: nickel_codex::ApprovalContext::default(),
            summary: "Existing request".into(),
        });
        app.update(ChatMessage::SelectApprovalPolicy(
            nickel_codex::ApprovalPolicy::Never,
        ));
        let mut scenario = Scenario::new(app, 900, 640);
        scenario.host_mut().handle_event(UiEvent::FocusLost);
        let app = scenario.host().application();
        assert_eq!(
            app.state.effective_approval_policy,
            nickel_codex::ApprovalPolicy::Untrusted
        );
        assert_eq!(
            app.state.selected_approval_policy,
            nickel_codex::ApprovalPolicy::Never
        );
        assert_eq!(app.state.pending.len(), 1);
    }

    #[test]
    fn approval_policy_control_works_through_direct_activation_modalities() {
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Keyboard,
            ActivationVia::Touch,
            ActivationVia::Accessibility,
            ActivationVia::Controller,
        ] {
            let backend =
                ReplayBackend::from_json(r#"{"name":"policy-input","events":[]}"#).unwrap();
            let app = ChatApplication::new(BackendMode::Replay {
                backend,
                cwd: "/projects/nickel".into(),
            });
            let mut scenario = Scenario::new(app, 1280, 700);
            if via == ActivationVia::Controller {
                // Enter the main pane before navigating back to menu chrome;
                // the generic spatial route otherwise enters File's submenu.
                scenario
                    .host_mut()
                    .handle_controller_action(nickel_ui::ControllerAction::Down);
                scenario
                    .host_mut()
                    .handle_controller_action(nickel_ui::ControllerAction::Down);
                scenario
                    .activate_via(via, &Selector::id("root/menu-bar/codex-menu"))
                    .unwrap();
                scenario
                    .activate_via(
                        via,
                        &Selector::role_name(SemanticRole::MenuItem, "Run settings"),
                    )
                    .unwrap();
            } else {
                open_run_settings(&mut scenario);
            }
            scenario
                .activate_via(
                    via,
                    &Selector::role_name(SemanticRole::Button, "Approval policy selector"),
                )
                .unwrap();
            if via == ActivationVia::Controller {
                scenario
                    .host_mut()
                    .handle_controller_action(nickel_ui::ControllerAction::Down);
                scenario
                    .host_mut()
                    .handle_controller_action(nickel_ui::ControllerAction::Confirm);
            } else {
                scenario
                    .activate_via(
                        via,
                        &Selector::role_name(
                            SemanticRole::MenuItem,
                            "Never ask — Codex cannot pause to request approval",
                        ),
                    )
                    .unwrap_or_else(|error| panic!("{via:?}: {error:?}"));
            }
            assert_eq!(
                scenario.host().application().state.selected_approval_policy,
                if via == ActivationVia::Controller {
                    nickel_codex::ApprovalPolicy::OnFailure
                } else {
                    nickel_codex::ApprovalPolicy::Never
                }
            );
        }
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
    fn standalone_resume_loads_the_selected_transcript_in_the_same_application() {
        let backend = ReplayBackend::from_json(
            r#"{"name":"resume","threads":[{"id":"recent","title":"Chosen","cwd":"/projects/nickel","last_used_at":9,"turns":[{"id":"old-turn","status":"completed","items":[{"id":"user","item_type":"userMessage","text":"last useful message","command_actions":[]}]}]}],"thread_runtime":{"recent":{"project_id":"nickel","status":"Idle","active_flags":[],"can_accept_direct_input":true}},"events":[]}"#,
        ).unwrap();
        let probe = backend.clone();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        for _ in 0..100 {
            app.poll_controller();
            if app.state.status == ConnectionStatus::Ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.update(ChatMessage::ToggleResumePicker);
        for _ in 0..100 {
            app.poll_controller();
            if !app.resume_picker_loading {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.update(ChatMessage::SelectThread(ThreadId("recent".into())));
        for _ in 0..100 {
            app.poll_controller();
            if app.state.selected_thread.as_ref() == Some(&ThreadId("recent".into())) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(probe.resumed_threads(), [ThreadId("recent".into())]);
        assert_eq!(app.state.selected_thread, Some(ThreadId("recent".into())));
        assert!(!app.resume_picker_open);
        assert!(
            app.state
                .items
                .iter()
                .any(|item| item.text == "last useful message")
        );
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
                turns: (id.0 == "eligible")
                    .then(|| nickel_codex::ThreadHistoryTurn {
                        id: TurnId("history".into()),
                        status: "completed".into(),
                        items: vec![nickel_codex::ThreadHistoryItem {
                            id: "message".into(),
                            item_type: "agentMessage".into(),
                            text: "latest preview text".into(),
                            command_actions: Vec::new(),
                            ..Default::default()
                        }],
                    })
                    .into_iter()
                    .collect(),
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

        app.state.draft = "/".into();
        let commands = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(
            &commands,
            "/review — review uncommitted changes"
        ));

        app.state.draft.clear();
        app.update(ChatMessage::ToggleRunSettings);
        app.update(ChatMessage::ToggleModelPicker);
        let models = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&models, "Reasoning effort selector"));

        app.update(ChatMessage::ToggleModelPicker);
        app.update(ChatMessage::ToggleResumePicker);
        app.resume_picker_loading = false;
        let resume_tree = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&resume_tree, "Already active"));
        assert!(has_accessible_text(&resume_tree, "latest preview text"));
        let mut scenario = Scenario::new(app, 900, 640);
        let button = |name: &str| nickel_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: name.into(),
        };
        assert!(scenario.host().query_unique(&button("eligible")).is_ok());
        // Shell-hosted active threads can focus an existing local writer;
        // the host rejects active writers owned outside this session.
        assert!(scenario.host().query_unique(&button("active")).is_ok());
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
    fn resume_preview_uses_last_message_normalizes_and_bounds_text() {
        let long = format!("  newest\n\t{}  ", "é".repeat(220));
        let thread = Thread {
            id: ThreadId("stable-thread-id".into()),
            title: None,
            cwd: None,
            last_used_at: None,
            turns: vec![nickel_codex::ThreadHistoryTurn {
                id: TurnId("turn".into()),
                status: "completed".into(),
                items: vec![
                    nickel_codex::ThreadHistoryItem {
                        id: "old".into(),
                        item_type: "userMessage".into(),
                        text: "older message".into(),
                        command_actions: Vec::new(),
                        ..Default::default()
                    },
                    nickel_codex::ThreadHistoryItem {
                        id: "new".into(),
                        item_type: "agentMessage".into(),
                        text: long,
                        command_actions: Vec::new(),
                        ..Default::default()
                    },
                ],
            }],
            model: None,
            reasoning_effort: None,
        };
        let preview = super::view::resume_preview(&thread).unwrap();
        assert!(preview.starts_with("newest é"));
        assert!(!preview.contains('\n'));
        assert_eq!(preview.chars().count(), 181);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn thread_snapshots_are_recent_first_deduplicated_and_bounded() {
        let mut state = ChatState::default();
        let make = |id: &str, recency| Thread {
            id: ThreadId(id.into()),
            title: None,
            cwd: None,
            last_used_at: Some(recency),
            turns: Vec::new(),
            model: None,
            reasoning_effort: None,
        };
        let mut runtime = std::collections::HashMap::new();
        runtime.insert(
            ThreadId("old".into()),
            nickel_codex::ThreadRuntime::default(),
        );
        runtime.insert(
            ThreadId("new".into()),
            nickel_codex::ThreadRuntime::default(),
        );
        runtime.insert(
            ThreadId("discarded".into()),
            nickel_codex::ThreadRuntime::default(),
        );
        state.apply(
            1,
            ControllerEvent::Ready {
                provenance: "fixture".into(),
                account: Default::default(),
                models: Vec::new(),
                projects: Vec::new(),
                threads: vec![make("old", 1), make("new", 3), make("old", 9)],
                runtime,
                thread_error: None,
                thread_next_cursor: None,
            },
        );
        assert_eq!(
            state
                .threads
                .iter()
                .map(|thread| thread.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["new", "old"]
        );
        assert!(
            !state
                .thread_runtime
                .contains_key(&ThreadId("discarded".into()))
        );
    }

    #[test]
    fn resume_failure_keeps_picker_recoverable_and_escape_cancels_it() {
        let backend = ReplayBackend::from_json(r#"{"name":"resume","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_chat(std::path::Path::new("/projects/nickel"));
        let thread = ThreadId("eligible".into());
        app.resume_picker_open = true;
        app.update(ChatMessage::SelectThread(thread));
        app.report_resume_rejection("writer lease raced");
        assert!(app.resume_picker_open);
        assert!(app.resume_picker_pending.is_none());
        assert_eq!(
            app.state.diagnostics.back().map(String::as_str),
            Some("writer lease raced")
        );
        assert!(app.shortcut_outcome(Shortcut::Escape).changed);
        assert!(!app.resume_picker_open);
    }

    #[test]
    fn resume_picker_distinguishes_loading_empty_failure_and_retry() {
        let backend = ReplayBackend::from_json(r#"{"name":"resume-states","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.resume_picker_open = true;
        app.resume_picker_loading = true;
        let frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&frame, "Loading recent conversations"));

        app.resume_picker_loading = false;
        let frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(&frame, "No conversations yet"));

        app.state.thread_error = Some("offline".into());
        let frame = UiFrame::layout(
            app.view(nickel_ui::ViewContext::new(
                Rect::new(0.0, 0.0, 900.0, 640.0),
                nickel_ui::InputModality::Keyboard,
            )),
            Rect::new(0.0, 0.0, 900.0, 640.0),
        );
        assert!(has_accessible_text(
            &frame,
            "Could not load conversations: offline"
        ));
        app.update(ChatMessage::RefreshResumePicker);
        assert!(app.resume_picker_open);
        assert!(app.resume_picker_loading);
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
        scenario
            .assert_value(
                &Selector::Role(SemanticRole::TextField),
                &nickel_ui::SemanticValueSnapshot::Text(expected.into()),
            )
            .unwrap();

        let long_draft = (0..30)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        scenario
            .set_value(
                &Selector::Role(SemanticRole::TextField),
                nickel_ui::SemanticValueInput::Text(long_draft.clone()),
            )
            .unwrap()
            .assert_value(
                &Selector::Role(SemanticRole::TextField),
                &nickel_ui::SemanticValueSnapshot::Text(long_draft),
            )
            .unwrap();
        let composer = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Group,
                name: "Message composer".into(),
            })
            .unwrap();
        assert!(
            composer.bounds.size.height <= 140.0,
            "composer bounds: {:?}",
            composer.bounds
        );
    }

    #[test]
    fn ime_composition_survives_live_resize_across_compact_threshold() {
        let backend = ReplayBackend::from_json(r#"{"name":"ime-resize","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        let mut scenario = Scenario::new(app, 1024, 720);
        let composer = scenario
            .host()
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .expect("composer field");
        scenario
            .host_mut()
            .handle_event(UiEvent::AccessibilityFocus(composer.id.clone()));
        scenario
            .host_mut()
            .handle_event(UiEvent::ImePreedit("世".into()));
        assert!(scenario.host().application().state.draft.is_empty());
        scenario.host_mut().resize(1023, 540);
        assert_eq!(scenario.host().inspect().keyboard_focus, Some(composer.id));
        scenario
            .host_mut()
            .handle_event(UiEvent::TextInput("世界".into()));
        assert_eq!(scenario.host().application().state.draft, "世界");
        assert!(
            scenario
                .host()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Send".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn streaming_agent_output_survives_live_resize_without_losing_text() {
        let backend = ReplayBackend::from_json(r#"{"name":"stream-resize","events":[]}"#).unwrap();
        let mut app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        });
        app.state.status = ConnectionStatus::Ready;
        app.state.account.authenticated = true;
        app.state.conversation_viewport_height = 300.0;
        app.state.apply(
            app.state.generation,
            event(
                1,
                EventKind::TurnStarted {
                    thread_id: ThreadId("stream".into()),
                    turn_id: TurnId("turn".into()),
                },
            ),
        );
        app.state.apply(
            app.state.generation,
            event(
                2,
                EventKind::ItemStarted {
                    thread_id: Some(ThreadId("stream".into())),
                    turn_id: Some(TurnId("turn".into())),
                    item_id: "agent".into(),
                    item_type: "agentMessage".into(),
                    command_actions: Vec::new(),
                    initial_text: String::new(),
                },
            ),
        );
        app.state.apply(
            app.state.generation,
            event(
                3,
                EventKind::AgentMessageDelta {
                    item_id: "agent".into(),
                    delta: "First half ".into(),
                },
            ),
        );
        let mut host = UiHost::new(app, 1024, 720);
        host.resize(1023, 540);
        let generation = host.application().state.generation;
        host.application_mut().state.apply(
            generation,
            event(
                4,
                EventKind::AgentMessageDelta {
                    item_id: "agent".into(),
                    delta: "second half".into(),
                },
            ),
        );
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        assert_eq!(
            host.application().state.items[0].text,
            "First half second half"
        );
        assert!(host.commands().iter().any(|command| match command {
            nickel_ui::backend::PaintCommand::Text { text, .. }
            | nickel_ui::backend::PaintCommand::StyledText { text, .. } => {
                text.contains("First half second half")
            }
            _ => false,
        }));
        assert!(host.inspect().diagnostics.is_empty());
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
        first.state.account.authenticated = true;
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
    fn embedded_project_menu_idle_poll_advances_deadline_without_redraw() {
        let backend = ReplayBackend::from_json(r#"{"name":"embedded","events":[]}"#).unwrap();
        let app = ChatApplication::new(BackendMode::Replay {
            backend,
            cwd: "/projects/nickel".into(),
        })
        .as_shell_project_menu();
        let mut host = UiHost::new(app, 520, 680);
        let limit = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while host.application().state.status != ConnectionStatus::Ready
            && std::time::Instant::now() < limit
        {
            host.step(HostBatch {
                now: Some(std::time::Instant::now()),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            std::thread::yield_now();
        }
        assert_eq!(host.application().state.status, ConnectionStatus::Ready);

        let now = std::time::Instant::now();
        let idle = host.step(HostBatch {
            now: Some(now),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });

        assert!(!idle.changed);
        assert!(host.next_deadline().is_some_and(|deadline| deadline > now));
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
            !state.transcript_selection_document().is_materialized(),
            "ordinary virtualized layout must not parse offscreen selection text"
        );
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

        let mut unselected_renderer = SoftwareRenderer::new(1120, 760, 1.0);
        unselected_renderer.render(tree.commands());
        let unselected_pixels = unselected_renderer.pixels().to_vec();

        let selected = UiFrame::layout_with_state(
            view::chat_view(&state),
            Rect::new(0.0, 0.0, 1120.0, 760.0),
            &mut ui_state,
        );
        let mut selected_renderer = SoftwareRenderer::new(1120, 760, 1.0);
        selected_renderer.render(selected.commands());
        assert_ne!(selected_renderer.pixels(), unselected_pixels.as_slice());
        for (width, height) in [(1280.0, 720.0), (640.0, 480.0), (1120.0, 760.0)] {
            let resized = UiFrame::layout_with_state(
                view::chat_view(&state),
                Rect::new(0.0, 0.0, width, height),
                &mut ui_state,
            );
            assert_eq!(
                resized.selected_text(&ui_state).as_deref(),
                Some(copied.as_str())
            );
        }
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
    #[ignore = "requires NICKEL_CODEX_LIVE=1 and an authenticated Codex app-server"]
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

    #[cfg(feature = "authenticated-live-tests")]
    #[test]
    #[ignore = "requires NICKEL_CODEX_LIVE=1 and an installed Codex app-server"]
    fn authenticated_live_file_mention_and_workspace_diff() {
        assert_eq!(
            std::env::var("NICKEL_CODEX_LIVE").as_deref(),
            Ok("1"),
            "set NICKEL_CODEX_LIVE=1 explicitly"
        );
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/main.rs"), "fn original() {}\n").unwrap();
        for args in [
            &["init", "--quiet"][..],
            &["add", "src/main.rs"][..],
            &[
                "-c",
                "user.name=Nickel Test",
                "-c",
                "user.email=nickel@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ][..],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(directory.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(directory.path().join("src/main.rs"), "fn changed() {}\n").unwrap();
        std::fs::write(directory.path().join("notes.txt"), "untracked\n").unwrap();

        let mut app = ChatApplication::new(BackendMode::Live {
            choice: BackendChoice::Installed,
            cwd: directory.path().into(),
        });
        wait_until(&mut app, |state| state.status == ConnectionStatus::Ready);
        app.update(ChatMessage::DraftChanged("@main".into()));
        wait_until(&mut app, |state| {
            !state.file_search_pending
                && state
                    .file_search_matches
                    .iter()
                    .any(|file| file.path.ends_with("src/main.rs"))
        });

        app.update(ChatMessage::DraftChanged("/diff".into()));
        app.update(ChatMessage::Send);
        wait_until(&mut app, |state| {
            state.items.iter().any(|item| {
                item.id.starts_with("local:diff:")
                    && item.text.contains("+fn changed() {}")
                    && item.text.contains("+untracked")
            })
        });
        assert!(
            app.state.diagnostics.is_empty(),
            "{:?}",
            app.state.diagnostics
        );
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
        panic!(
            "application state did not reach the expected condition: status={:?}, file_search_query={:?}, file_search_pending={}, file_search_matches={:?}, file_search_error={:?}, command_feedback={:?}, diagnostics={:?}",
            app.state.status,
            app.state.file_search_query,
            app.state.file_search_pending,
            app.state.file_search_matches,
            app.state.file_search_error,
            app.state.command_feedback,
            app.state.diagnostics,
        );
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
