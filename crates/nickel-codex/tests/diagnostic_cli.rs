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
    let records: Vec<_> = String::from_utf8(first.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect();
    assert_eq!(records.len(), 5);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record["schema_version"], 1);
        assert_eq!(record["sequence"], index as u64 + 1);
        assert_eq!(record["session"], "local-1");
        assert_eq!(record["kind"], "event");
    }
    assert_eq!(records[0]["value"]["kind"]["type"], "connection");
    assert_eq!(records[4]["value"]["kind"]["type"], "turn_completed");
}

#[test]
fn invalid_usage_is_machine_readable_and_distinct() {
    let output = Command::new(env!("CARGO_BIN_EXE_nickel-codex-test"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["sequence"], 1);
    assert_eq!(value["session"], "local-1");
    assert_eq!(value["kind"], "invalid_usage");
    let message = value["value"]["message"].as_str().unwrap();
    assert!(message.contains("replay SCENARIO.json"));
    assert!(message.contains("interrupt THREAD_ID TURN_ID"));
    assert!(output.stderr.is_empty());
}
