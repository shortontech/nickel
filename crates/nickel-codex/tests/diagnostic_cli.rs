use std::process::Command;

#[test]
fn replay_stdout_is_deterministic_jsonl() {
    let binary = env!("CARGO_BIN_EXE_nickel-codex-test");
    let scenario = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nickel-codex-fixture/fixtures/basic.json"
    );
    let first = Command::new(binary)
        .args(["replay", scenario])
        .output()
        .unwrap();
    let second = Command::new(binary)
        .args(["replay", scenario])
        .output()
        .unwrap();
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    for line in String::from_utf8(first.stdout).unwrap().lines() {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(value["schema_version"], 1);
    }
}

#[test]
fn invalid_usage_is_machine_readable_and_distinct() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-test"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["kind"], "invalid_usage");
}
