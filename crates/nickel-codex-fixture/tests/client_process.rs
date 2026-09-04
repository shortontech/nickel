use std::{path::Path, time::Duration};

use nickel_codex::{
    BackendChoice, CandidateSource, CodexBackend, CodexClient, EventKind, InteractionResponse,
    ProbeLimits, Selector, ServerRequestId, StartThread, StartTurn, ThreadId, ThreadPage,
};

fn install_fixture(source: &Path, destination: &Path) {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination).unwrap();
    }
    #[cfg(windows)]
    {
        let staged = destination.with_extension("staged");
        std::fs::copy(source, &staged).unwrap();
        std::fs::rename(staged, destination).unwrap();
    }
}

#[test]
fn real_stdio_process_supports_typed_lifecycle_and_streaming() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "basic").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    let events = client.subscribe();
    assert!(!client.account().unwrap().authenticated);
    assert_eq!(client.models().unwrap()[0].id, "fixture-model");
    assert!(
        client
            .list_threads(ThreadPage::default())
            .unwrap()
            .threads
            .is_empty()
    );
    let thread = client
        .start_thread(StartThread {
            cwd: directory.path().into(),
            model: None,
            project_id: None,
            reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(
        client.resume_thread(thread.id.clone()).unwrap().id,
        thread.id
    );
    let turn = client
        .start_turn(StartTurn {
            thread_id: thread.id.clone(),
            text: "hello".into(),
            images: Vec::new(),
            model: None,
            reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(turn.id.0, "fixture-turn");
    let received: Vec<_> =
        std::iter::from_fn(|| events.recv_timeout(Duration::from_millis(50)).ok()).collect();
    assert_eq!(received.len(), 7);
    assert_eq!(
        received
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (2..=8).collect::<Vec<_>>()
    );
    assert!(matches!(
        &received[0].kind,
        EventKind::ThreadStarted { thread_id } if thread_id.0 == "fixture-thread"
    ));
    assert!(matches!(
        &received[1].kind,
        EventKind::TurnStarted { thread_id, turn_id }
            if thread_id.0 == "fixture-thread" && turn_id.0 == "fixture-turn"
    ));
    assert!(matches!(
        &received[2].kind,
        EventKind::ItemStarted { item_id, item_type, .. }
            if item_id == "message-1" && item_type == "agentMessage"
    ));
    assert!(matches!(
        &received[3].kind,
        EventKind::AgentMessageDelta { item_id, delta }
            if item_id == "message-1" && delta == "hello"
    ));
    assert!(matches!(
        &received[4].kind,
        EventKind::ApprovalRequested { request_id, summary, .. }
            if request_id.0 == "71" && summary.as_deref() == Some("fixture approval")
    ));
    assert!(matches!(
        &received[5].kind,
        EventKind::UserInputRequested {
            request_id,
            question_ids,
            ..
        } if request_id.0 == "input-1" && question_ids == &["q1"]
    ));
    assert!(matches!(
        &received[6].kind,
        EventKind::TurnCompleted { thread_id, turn_id, status }
            if thread_id.0 == "fixture-thread" && turn_id.0 == "fixture-turn" && status == "completed"
    ));
    assert_eq!(client.projection().active_turn, None);
    assert_eq!(client.projection().items["message-1"].text, "hello");
    assert!(
        client
            .respond(
                ServerRequestId("71".into()),
                InteractionResponse::UserInput { answers: vec![] }
            )
            .is_err()
    );
    client
        .respond(
            ServerRequestId("71".into()),
            InteractionResponse::CommandApproval {
                decision: nickel_codex::CommandDecision::Decline,
            },
        )
        .unwrap();
    for _ in 0..20 {
        if directory.path().join("fixture-response-71.json").is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let response: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("fixture-response-71.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(response["id"], 71, "numeric request ID was not preserved");
    assert!(
        client
            .respond(
                ServerRequestId("input-1".into()),
                InteractionResponse::UserInput {
                    answers: vec![nickel_codex::UserInputAnswer {
                        question_id: "not-q1".into(),
                        answer: "bad".into(),
                    }],
                },
            )
            .is_err()
    );
    client
        .respond(
            ServerRequestId("input-1".into()),
            InteractionResponse::UserInput {
                answers: vec![nickel_codex::UserInputAnswer {
                    question_id: "q1".into(),
                    answer: "fixture answer".into(),
                }],
            },
        )
        .unwrap();
    assert!(
        received
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    client
        .interrupt_turn(ThreadId("fixture-thread".into()), turn.id)
        .unwrap();
    client.shutdown();
}

#[test]
fn automatic_selection_falls_back_from_incompatible_installed_candidate() {
    let fixture = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    let installed = directory
        .path()
        .join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&installed, b"not an executable fixture").unwrap();
    let path = std::env::join_paths([directory.path()]).unwrap();
    let selection =
        Selector::new(Some(fixture.into())).select_with_path(BackendChoice::Automatic, Some(&path));
    assert_eq!(selection.probes.len(), 2);
    assert!(!selection.probes[0].compatible);
    assert_eq!(selection.selected.unwrap().source, CandidateSource::Bundled);
}

#[test]
fn fixture_candidate_passes_schema_and_handshake_probe() {
    let fixture = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let selection = Selector::new(None).select(BackendChoice::Path(fixture.into()));
    let selected = selection.selected.expect("fixture must pass its own probe");
    assert_eq!(selected.source, CandidateSource::Explicit);
    assert_eq!(selected.path, fixture);
    assert_eq!(selection.probes.len(), 1);
    let probe = &selection.probes[0];
    assert_eq!(probe.candidate, selected);
    assert!(probe.compatible);
    assert_eq!(
        probe.reason,
        "required schema and initialize handshake accepted"
    );
    assert!(
        probe
            .version
            .as_deref()
            .is_some_and(|version| !version.is_empty())
    );
    assert!(probe.executable_sha256.as_deref().is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    }));
    assert!(
        probe
            .generated_schema_sha256
            .as_deref()
            .is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    );
}

#[test]
fn candidate_and_working_directory_support_spaces_and_unicode() {
    let fixture = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::Builder::new()
        .prefix("nickel codex ñ ")
        .tempdir()
        .unwrap();
    let copied = directory.path().join(if cfg!(windows) {
        "codex fixture.exe"
    } else {
        "codex fixture"
    });
    install_fixture(fixture, &copied);
    let selection = Selector::new(None).select(BackendChoice::Path(copied.clone()));
    let selected = selection
        .selected
        .expect("unicode and spaced fixture path must remain executable");
    assert_eq!(selected.source, CandidateSource::Explicit);
    assert_eq!(selected.path, copied);
    assert_eq!(selection.probes.len(), 1);
    assert!(selection.probes[0].compatible);
    let client = CodexClient::spawn(&copied, directory.path()).unwrap();
    assert!(!client.account().unwrap().authenticated);
    client.shutdown();
}

#[test]
fn replacing_candidate_invalidates_compatibility_by_reprobing_content() {
    let fixture = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    let candidate = directory
        .path()
        .join(if cfg!(windows) { "codex.exe" } else { "codex" });
    install_fixture(fixture, &candidate);
    let selector = Selector::new(None);
    let initial = selector.select(BackendChoice::Path(candidate.clone()));
    assert!(initial.selected.is_some(), "{:?}", initial.probes);
    let replacement = directory.path().join("codex-replacement");
    std::fs::write(&replacement, b"replaced incompatible executable").unwrap();
    #[cfg(windows)]
    std::fs::remove_file(&candidate).unwrap();
    std::fs::rename(replacement, &candidate).unwrap();
    assert!(
        selector
            .select(BackendChoice::Path(candidate))
            .selected
            .is_none()
    );
}

#[test]
fn compatibility_probe_bounds_hangs_and_rejects_missing_methods() {
    let fixture = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    for name in ["codex-hang-probe", "codex-missing-method"] {
        let candidate = directory.path().join(name);
        install_fixture(fixture, &candidate);
        let selector = Selector::new(None).with_limits(ProbeLimits {
            command_timeout: Duration::from_millis(200),
            handshake_timeout: Duration::from_millis(200),
        });
        let started = std::time::Instant::now();
        let selection = selector.select(BackendChoice::Path(candidate));
        assert!(selection.selected.is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn bounded_stderr_flood_cannot_block_protocol_progress() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "stderr-flood").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    assert_eq!(client.models().unwrap()[0].id, "fixture-model");
    for _ in 0..20 {
        if client.stderr_snapshot().len() == 65_536 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.stderr_snapshot().len(), 65_536);
    client.shutdown();
}

#[test]
fn duplicate_terminal_notifications_are_idempotent() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "duplicate-terminal").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    let events = client.subscribe();
    client
        .start_turn(StartTurn {
            thread_id: ThreadId("fixture-thread".into()),
            text: "duplicate".into(),
            images: Vec::new(),
            model: None,
            reasoning_effort: None,
        })
        .unwrap();
    for _ in 0..20 {
        if client.projection().active_turn.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        client.projection().threads[&ThreadId("fixture-thread".into())]
            .terminal_turns
            .len(),
        1
    );
    let received: Vec<_> =
        std::iter::from_fn(|| events.recv_timeout(Duration::from_millis(20)).ok()).collect();
    assert!(
        !received
            .iter()
            .any(|event| matches!(event.kind, EventKind::Inconsistency { .. }))
    );
    client.shutdown();
}

#[test]
fn explicit_interrupt_reaches_terminal_interrupted_state() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "wait-for-interrupt").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    let events = client.subscribe();
    let turn = client
        .start_turn(StartTurn {
            thread_id: ThreadId("fixture-thread".into()),
            text: "interrupt".into(),
            images: Vec::new(),
            model: None,
            reasoning_effort: None,
        })
        .unwrap();
    client
        .interrupt_turn(ThreadId("fixture-thread".into()), turn.id)
        .unwrap();
    let received: Vec<_> =
        std::iter::from_fn(|| events.recv_timeout(Duration::from_millis(50)).ok()).collect();
    assert!(received.iter().any(|event| matches!(
        &event.kind,
        EventKind::TurnCompleted { status, .. } if status == "interrupted"
    )));
    client.shutdown();
}

#[test]
fn malformed_exit_hang_and_oversized_frames_fail_boundedly() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    for mode in ["malformed", "exit", "hang", "oversized"] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("fixture-mode"), mode).unwrap();
        let started = std::time::Instant::now();
        let result = CodexClient::spawn_with_timeout(
            executable,
            directory.path(),
            Duration::from_millis(250),
        );
        assert!(result.is_err(), "{mode} unexpectedly initialized");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{mode} was not bounded"
        );
    }
}

#[test]
fn out_of_order_responses_never_cross_request_ids() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "out-of-order").unwrap();
    let client = std::sync::Arc::new(CodexClient::spawn(executable, directory.path()).unwrap());
    let account_client = client.clone();
    let models_client = client.clone();
    let account = std::thread::spawn(move || account_client.account().unwrap());
    let models = std::thread::spawn(move || models_client.models().unwrap());
    assert!(!account.join().unwrap().authenticated);
    assert_eq!(models.join().unwrap()[0].id, "fixture-model");
    client.shutdown();
}

#[test]
fn slow_consumer_is_bounded_and_projected_state_remains_complete() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "flood").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    let _events = client.subscribe();
    client
        .start_turn(StartTurn {
            thread_id: ThreadId("fixture-thread".into()),
            text: "flood".into(),
            images: Vec::new(),
            model: None,
            reasoning_effort: None,
        })
        .unwrap();
    for _ in 0..50 {
        if client
            .projection()
            .items
            .get("command-flood")
            .is_some_and(|item| item.text.len() == 1500)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.projection().items["command-flood"].text.len(), 1500);
    assert!(client.dropped_event_count() > 0);
    client.shutdown();
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_last_client_owner_reaps_the_child() {
    let executable = Path::new(env!("CARGO_BIN_EXE_nickel-codex-fixture"));
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture-mode"), "hang-after-init").unwrap();
    let client = CodexClient::spawn(executable, directory.path()).unwrap();
    let pid = std::fs::read_to_string(directory.path().join("fixture-pid")).unwrap();
    assert!(Path::new("/proc").join(pid.trim()).is_dir());
    drop(client);
    for _ in 0..50 {
        if !Path::new("/proc").join(pid.trim()).exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("fixture process {pid} survived dropping the last client owner");
}
