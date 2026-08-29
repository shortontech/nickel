use std::process::Command;

#[test]
fn help_is_bounded_and_does_not_start_the_ui() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-ui"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "nickel-codex-ui [--backend auto|installed|bundled|ABSOLUTE_PATH] [--cwd PATH] [--thread THREAD_ID]\n\
nickel-codex-ui --replay SCENARIO [--cwd PATH] [--thread THREAD_ID]\n"
    );
}

#[test]
fn invalid_replay_fails_before_sdl_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-ui"))
        .args(["--replay", "/definitely/missing/nickel-scenario.json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("cannot read replay scenario: ")
    );
}
