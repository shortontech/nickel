#![cfg(feature = "application")]

use std::{fs, process::Child, process::Command, thread, time::Duration, time::Instant};

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
    assert_eq!(
        String::from_utf8_lossy(&help.stdout),
        "nickel-markdown-ui PATH\n\nOpen one local .md or .markdown file in a read-only Nickel viewer.\n"
    );
    assert!(help.stderr.is_empty());

    let missing = Command::new(executable)
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "usage: nickel-markdown-ui PATH\n"
    );
    assert!(missing.stdout.is_empty());

    let multiple = Command::new(executable)
        .args(["one.md", "two.md"])
        .current_dir(temporary.path())
        .output()
        .unwrap();
    assert_eq!(multiple.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&multiple.stderr),
        "usage: nickel-markdown-ui PATH\n"
    );
    assert!(multiple.stdout.is_empty());
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn valid_and_missing_documents_keep_viewer_alive_without_sidecar_files() {
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
        wait_until_running(&mut child);
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(baseline(), before);
    }
}

fn wait_until_running(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            child.try_wait().unwrap().is_none(),
            "viewer exited before its startup state could be inspected"
        );
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}
