#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{env, path::PathBuf, process::ExitCode};

use nickel_codex::{BackendChoice, ReplayBackend};
use nickel_codex_ui::{BackendMode, ChatApplication};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "nickel-codex-ui [--backend auto|installed|bundled|ABSOLUTE_PATH] [--cwd PATH]\n\
             nickel-codex-ui --replay SCENARIO [--cwd PATH]"
        );
        return ExitCode::SUCCESS;
    }
    let cwd = option(&args, "--cwd")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let mode = if let Some(path) = option(&args, "--replay") {
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
    } else {
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
    match nickel_ui::run(ChatApplication::new(mode)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Nickel Codex UI failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}
