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

#[test]
fn exact_baseline_passes_and_any_finding_change_fails() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("view.rs");
    let baseline = fixture.path().join("baseline.tsv");
    fs::write(&source, "fn view() { Text::new(\"Visible\"); }\n").unwrap();
    let printed = lint(&["--print-baseline".as_ref(), source.as_os_str()]);
    assert!(printed.status.success());
    fs::write(&baseline, printed.stdout).unwrap();
    assert!(
        lint(&[
            "--baseline".as_ref(),
            baseline.as_os_str(),
            source.as_os_str()
        ])
        .status
        .success()
    );

    fs::write(&source, "fn view() { Text::new(\"Changed\"); }\n").unwrap();
    let changed = lint(&[
        "--baseline".as_ref(),
        baseline.as_os_str(),
        source.as_os_str(),
    ]);
    assert!(!changed.status.success());
    assert!(
        String::from_utf8(changed.stderr)
            .unwrap()
            .contains("localization baseline changed")
    );
}
