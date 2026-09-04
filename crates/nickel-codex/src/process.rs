use std::{path::Path, process::Command};

pub(crate) fn command(executable: &Path) -> Command {
    #[cfg(windows)]
    if matches!(
        executable.extension().and_then(|extension| extension.to_str()),
        Some(extension) if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    ) {
        let interpreter = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
        let mut command = Command::new(interpreter);
        command.args(["/D", "/S", "/C", "CALL"]).arg(executable);
        return command;
    }

    Command::new(executable)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn command_scripts_receive_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("codex fixture.cmd");
        fs::write(&fixture, "@echo off\r\necho %1\r\n").unwrap();

        let output = command(&fixture).arg("--version").output().unwrap();

        assert!(
            output.status.success(),
            "status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "--version"
        );
    }
}
