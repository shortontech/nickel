use std::{path::Path, process::Command};

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
const MAX_COMMAND_SHIM_BYTES: u64 = 64 * 1024;

pub(crate) fn command(executable: &Path) -> Command {
    #[cfg(windows)]
    if matches!(
        executable.extension().and_then(|extension| extension.to_str()),
        Some(extension) if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    ) {
        if let Some((node, entrypoint)) = npm_shim_target(executable) {
            let mut command = Command::new(node);
            command.arg(entrypoint);
            suppress_window(&mut command);
            return command;
        }
        let interpreter = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
        let mut command = Command::new(interpreter);
        command.args(["/D", "/S", "/C", "CALL"]).arg(executable);
        suppress_window(&mut command);
        return command;
    }

    let command = Command::new(executable);
    #[cfg(windows)]
    let command = {
        let mut command = command;
        suppress_window(&mut command);
        command
    };
    command
}

#[cfg(windows)]
fn npm_shim_target(executable: &Path) -> Option<(PathBuf, PathBuf)> {
    let metadata = std::fs::metadata(executable).ok()?;
    if metadata.len() > MAX_COMMAND_SHIM_BYTES {
        return None;
    }
    let source = std::fs::read_to_string(executable).ok()?;
    let relative = source
        .split("\"%dp0%\\")
        .skip(1)
        .filter_map(|suffix| suffix.split('"').next())
        .find(|path| path.to_ascii_lowercase().ends_with(".js"))?;
    let directory = executable.parent()?;
    let entrypoint = directory.join(relative);
    if !entrypoint.is_file() {
        return None;
    }
    let adjacent_node = directory.join("node.exe");
    let node = adjacent_node
        .is_file()
        .then_some(adjacent_node)
        .unwrap_or_else(|| PathBuf::from("node"));
    Some((node, entrypoint))
}

#[cfg(windows)]
fn suppress_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
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

    #[test]
    fn npm_command_shims_resolve_to_node_and_their_javascript_entrypoint() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("codex.cmd");
        let entrypoint = directory
            .path()
            .join("node_modules/@openai/codex/bin/codex.js");
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        fs::write(&entrypoint, "").unwrap();
        fs::write(
            &fixture,
            r#"@ECHO off
"%_prog%" "%dp0%\node_modules\@openai\codex\bin\codex.js" %*
"#,
        )
        .unwrap();

        let (node, resolved) = npm_shim_target(&fixture).unwrap();
        assert_eq!(node, PathBuf::from("node"));
        assert_eq!(resolved, entrypoint);
    }
}
