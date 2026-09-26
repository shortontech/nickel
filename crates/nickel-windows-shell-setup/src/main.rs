#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(target_os = "windows")]
use std::{env, path::PathBuf};

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
        "disable" => nickel_windows_shell_setup::disable(&executable),
        _ => Err(usage()),
    }
}

#[cfg(not(target_os = "windows"))]
fn run() -> Result<(), String> {
    Err("this helper is available only on Windows".into())
}

#[cfg(target_os = "windows")]
fn usage() -> String {
    "usage: nickel-shell-setup <enable|disable> <absolute-path-to-nickel.exe>".into()
}
