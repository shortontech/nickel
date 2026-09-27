#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(target_os = "windows")]
use std::{env, path::PathBuf, process::Command};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nickel-shell-setup: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "windows")]
fn run() -> Result<(), String> {
    let mut arguments = env::args_os().skip(1);
    let operation = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let executable = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }
    match operation.as_str() {
        "enable" => nickel_windows_shell_setup::enable(&executable),
        "activate" => activate(&executable),
        "disable" => nickel_windows_shell_setup::disable(&executable),
        _ => Err(usage()),
    }
}

#[cfg(target_os = "windows")]
fn activate(executable: &std::path::Path) -> Result<(), String> {
    nickel_windows_shell_setup::enable(executable)?;

    let stopped = Command::new("taskkill.exe")
        .args(["/F", "/IM", "explorer.exe"])
        .status()
        .map_err(|error| format!("could not stop Explorer: {error}"))?;
    // taskkill uses 128 when no matching process exists. That is already the desired state.
    if !stopped.success() && stopped.code() != Some(128) {
        return Err(format!(
            "could not stop Explorer: taskkill exited with {stopped}"
        ));
    }

    if let Err(error) = Command::new(executable).spawn() {
        let _ = Command::new("explorer.exe").spawn();
        return Err(format!("could not start Nickel: {error}"));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn run() -> Result<(), String> {
    Err("this helper is available only on Windows".into())
}

#[cfg(target_os = "windows")]
fn usage() -> String {
    "usage: nickel-shell-setup <activate|enable|disable> <absolute-path-to-nickel.exe>".into()
}
