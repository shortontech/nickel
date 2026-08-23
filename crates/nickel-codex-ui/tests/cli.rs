use std::process::Command;

#[test]
fn help_is_bounded_and_does_not_start_the_ui() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-ui"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("--replay SCENARIO")
    );
}

#[test]
fn invalid_replay_fails_before_sdl_startup() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-ui"))
        .args(["--replay", "/definitely/missing/nickel-scenario.json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("cannot read replay scenario")
    );
}
