#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{env, path::PathBuf, process::ExitCode};

use nickel_codex::{BackendChoice, CodexSettings, ReplayBackend};
use nickel_codex_ui::{BackendMode, ChatApplication, create_managed_workspace};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "nickel-codex-ui [--backend auto|installed|bundled|ABSOLUTE_PATH] [--cwd PATH]\n\
             nickel-codex-ui --replay SCENARIO [--cwd PATH]"
        );
        return ExitCode::SUCCESS;
    }
    let settings_path = match CodexSettings::default_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("cannot locate Nickel Codex host settings: {error}");
            return ExitCode::from(2);
        }
    };
    let settings = match CodexSettings::load(&settings_path) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("cannot load Nickel Codex host settings: {error}");
            return ExitCode::from(2);
        }
    };
    let explicit_cwd = option(&args, "--cwd").map(PathBuf::from);
    let mode = if let Some(path) = option(&args, "--replay") {
        let cwd = match local_cwd(explicit_cwd.clone()) {
            Ok(cwd) => cwd,
            Err(error) => {
                eprintln!("cannot create managed Codex workspace: {error}");
                return ExitCode::from(2);
            }
        };
        let input = match std::fs::read_to_string(path) {
            Ok(input) => input,
            Err(error) => {
                eprintln!("cannot read replay scenario: {error}");
                return ExitCode::from(2);
            }
        };
        let backend = match ReplayBackend::from_json(&input) {
            Ok(backend) => backend,
            Err(error) => {
                eprintln!("invalid replay scenario: {error}");
                return ExitCode::from(2);
            }
        };
        BackendMode::Replay { backend, cwd }
    } else if option(&args, "--backend").is_none() && explicit_cwd.is_none() {
        match selected_mode(&settings) {
            Ok(mode) => mode,
            Err(error) => {
                eprintln!("cannot create managed Codex workspace: {error}");
                return ExitCode::from(2);
            }
        }
    } else {
        let cwd = match local_cwd(explicit_cwd.clone()) {
            Ok(cwd) => cwd,
            Err(error) => {
                eprintln!("cannot create managed Codex workspace: {error}");
                return ExitCode::from(2);
            }
        };
        let choice = match option(&args, "--backend").unwrap_or("auto") {
            "auto" => BackendChoice::Automatic,
            "installed" => BackendChoice::Installed,
            "bundled" => BackendChoice::Bundled,
            path if PathBuf::from(path).is_absolute() => BackendChoice::Path(PathBuf::from(path)),
            value => {
                eprintln!(
                    "invalid backend {value}; use auto, installed, bundled, or an absolute path"
                );
                return ExitCode::from(2);
            }
        };
        BackendMode::Live { choice, cwd }
    };
    match nickel_ui::run(ChatApplication::with_settings(
        mode,
        settings,
        Some(settings_path),
    )) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Nickel Codex UI failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn local_cwd(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    explicit.map(Ok).unwrap_or_else(create_managed_workspace)
}

fn selected_mode(settings: &CodexSettings) -> Result<BackendMode, String> {
    settings.selected_host().map_or_else(
        || {
            Ok(BackendMode::Live {
                choice: BackendChoice::Automatic,
                cwd: local_cwd(None)?,
            })
        },
        |host| Ok(BackendMode::Remote { host: host.clone() }),
    )
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use nickel_codex::RemoteHost;

    use super::*;

    #[test]
    fn persisted_remote_selection_builds_only_a_remote_mode() {
        let mut settings = CodexSettings::default();
        settings.hosts.push(RemoteHost {
            id: "arm_host".into(),
            name: "ARM host".into(),
            endpoint: "ws://127.0.0.1:9999/app-server".into(),
            token_env: None,
            default_cwd: "/srv/nickel".into(),
        });
        settings.selected = "arm_host".into();

        assert!(matches!(
            selected_mode(&settings).unwrap(),
            BackendMode::Remote { host } if host.default_cwd == "/srv/nickel"
        ));
    }
}
