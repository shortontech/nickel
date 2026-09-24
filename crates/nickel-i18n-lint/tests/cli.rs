use std::{fs, process::Command};

fn lint(arguments: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nickel-i18n-lint"))
        .args(arguments)
        .output()
        .expect("localization lint should run")
}

#[test]
fn violations_have_stable_locations_code_and_failure_status() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("view.rs");
    fs::write(&source, "fn view() { Text::new(\"Visible\"); }\n").unwrap();
    let output = lint(&[source.as_os_str()]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(stderr.contains(":1:23: NIL001 "), "{stderr}");
    assert!(stderr.contains("1 localization violation(s)"), "{stderr}");
}
