#![cfg(feature = "application")]

use std::{fs, process::Command, thread, time::Duration};

#[test]
fn help_and_argument_errors_do_not_create_application_state() {
    let executable = env!("CARGO_BIN_EXE_nickel-markdown-ui");
    let temporary = tempfile::tempdir().unwrap();
    let help = Command::new(executable)
        .arg("--help")
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("read-only Nickel viewer"));

    let missing = Command::new(executable)
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("usage:"));

    let multiple = Command::new(executable)
        .args(["one.md", "two.md"])
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert_eq!(multiple.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&multiple.stderr).contains("usage:"));
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn successful_and_failed_file_startup_create_no_persistent_state() {
    let executable = env!("CARGO_BIN_EXE_nickel-markdown-ui");
    let temporary = tempfile::tempdir().unwrap();
    let markdown = temporary.path().join("guide.md");
    fs::write(&markdown, "# Guide").unwrap();
    let baseline = || {
        let mut entries = fs::read_dir(temporary.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    };
    let before = baseline();
    for path in [markdown, temporary.path().join("missing.md")] {
        let mut child = Command::new(executable)
            .arg(path)
            .env("SDL_VIDEODRIVER", "dummy")
            .current_dir(temporary.path())
            .spawn()
            .unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(
            child.try_wait().unwrap().is_none(),
            "viewer did not stay open"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(baseline(), before);
    }
}
