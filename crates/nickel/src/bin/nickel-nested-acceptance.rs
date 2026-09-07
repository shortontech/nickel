//! Bounded live acceptance harness for Nickel's nested compositor.
//!
//! Build all three participating binaries, then run this binary from the same
//! target directory. The harness uses an isolated runtime directory, captures
//! the capability environment passed to the supervised shell, and always asks
//! the compositor to log out before its deadline.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const DEADLINE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    let arguments = env::args_os().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_some_and(|value| value == "--shell-bridge")
    {
        return bridge_shell(&arguments[2..]);
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FAIL: {error}");
            ExitCode::FAILURE
        }
    }
}

fn bridge_shell(arguments: &[std::ffi::OsString]) -> ExitCode {
    let [nickel, environment_file] = arguments else {
        eprintln!("shell bridge requires NICKEL and ENVIRONMENT_FILE");
        return ExitCode::FAILURE;
    };
    let variables = [
        "XDG_RUNTIME_DIR",
        "WAYLAND_DISPLAY",
        "NICKEL_SESSION_CONTROL",
        "NICKEL_SESSION_TOKEN",
        "NICKEL_SHELL_TEST_CONTROL",
        "NICKEL_SHELL_STARTUP_BARRIER",
    ];
    let contents = variables
        .into_iter()
        .filter_map(|name| env::var(name).ok().map(|value| format!("{name}={value}\n")))
        .collect::<String>();
    if let Err(error) = fs::write(environment_file, contents) {
        eprintln!("could not publish nested capability environment: {error}");
        return ExitCode::FAILURE;
    }
    match Command::new(nickel).args(["--role", "shell"]).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!("could not launch nested shell: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let harness = env::current_exe().map_err(|error| error.to_string())?;
    let directory = harness
        .parent()
        .ok_or("acceptance harness has no parent directory")?;
    let nickel = sibling(directory, "nickel")?;
    let test_input = sibling(directory, "nickel-test-input")?;
    let runtime = env::temp_dir().join(format!(
        "nickel-nested-acceptance-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&runtime).map_err(|error| error.to_string())?;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let capability_file = runtime.join("shell-environment");

    let mut compositor = Command::new(&nickel)
        .args(["--backend", "winit", "--test-control", "--command"])
        .arg(&harness)
        .arg("--shell-bridge")
        .arg(&nickel)
        .arg(&capability_file)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("NICKEL_NESTED_SIZE", "960x640")
        // Prefer the host X server when both host protocols are advertised.
        // This keeps the acceptance window independent of the Nickel session
        // under test and avoids nesting its bootstrap inside itself.
        .env("WINIT_UNIX_BACKEND", "x11")
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start nested compositor: {error}"))?;

    let result = exercise(&mut compositor, &test_input, &capability_file);
    if compositor
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_none()
    {
        let environment = read_environment(&capability_file).unwrap_or_default();
        // Logout stops the event loop while the command is being handled, so
        // the datagram client need not receive a final acknowledgement.
        let _ = Command::new(&test_input)
            .args(["session", "logout"])
            .envs(environment)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    let shutdown_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < shutdown_deadline {
        if compositor
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            break;
        }
        thread::sleep(POLL);
    }
    if compositor
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_none()
    {
        let _ = compositor.kill();
        let _ = compositor.wait();
        let _ = fs::remove_dir_all(&runtime);
        return match result {
            Err(error) => Err(error),
            Ok(()) => Err("nested compositor did not stop after the logout request".into()),
        };
    }
    let _ = fs::remove_dir_all(&runtime);
    result?;
    println!(
        "PASS: nested compositor became ready, exposed shell surfaces, accepted input, reported idle diagnostics, and shut down cleanly"
    );
    Ok(())
}

fn exercise(
    compositor: &mut Child,
    test_input: &Path,
    capability_file: &Path,
) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    let environment = loop {
        if let Some(status) = compositor.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "nested compositor exited before readiness: {status}"
            ));
        }
        if let Ok(environment) = read_environment(capability_file) {
            let readiness = invoke(test_input, &environment, &["readiness"])?;
            if readiness.status.success()
                && String::from_utf8_lossy(&readiness.stdout).contains("ready=true")
            {
                break environment;
            }
        }
        if Instant::now() >= deadline {
            return Err("nested compositor did not become ready within 30 seconds".into());
        }
        thread::sleep(POLL);
    };

    let surfaces = checked(test_input, &environment, &["surfaces"])?;
    for role in ["Desktop", "Panel", "Lock", "Launcher"] {
        if !surfaces.contains(role) {
            return Err(format!(
                "surface inventory does not contain {role}: {surfaces:?}"
            ));
        }
    }
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;

    let before = diagnostics(test_input, &environment)?;
    thread::sleep(Duration::from_secs(2));
    let after = diagnostics(test_input, &environment)?;
    let before_wakeups = json_u64(&before, "scheduled_wakeups")?;
    let after_wakeups = json_u64(&after, "scheduled_wakeups")?;
    println!(
        "idle diagnostic: scheduled_wakeups_delta={} over 2s",
        after_wakeups.saturating_sub(before_wakeups)
    );
    Ok(())
}

fn diagnostics(test_input: &Path, environment: &[(String, String)]) -> Result<String, String> {
    checked(test_input, environment, &["runtime-diagnostics"])
}

fn json_u64(document: &str, key: &str) -> Result<u64, String> {
    let value: serde_json::Value = serde_json::from_str(document.trim())
        .map_err(|error| format!("invalid runtime diagnostics: {error}"))?;
    value[key]
        .as_u64()
        .ok_or_else(|| format!("runtime diagnostics omitted {key}"))
}

fn checked(
    test_input: &Path,
    environment: &[(String, String)],
    args: &[&str],
) -> Result<String, String> {
    let output = invoke(test_input, environment, args)?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn invoke(
    test_input: &Path,
    environment: &[(String, String)],
    args: &[&str],
) -> Result<Output, String> {
    Command::new(test_input)
        .args(args)
        .envs(environment.iter().cloned())
        .output()
        .map_err(|error| format!("could not run {}: {error}", args.join(" ")))
}

fn read_environment(path: &Path) -> Result<Vec<(String, String)>, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let environment = contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    if environment
        .iter()
        .any(|(name, _)| name == "NICKEL_SESSION_CONTROL")
        && environment
            .iter()
            .any(|(name, _)| name == "NICKEL_SESSION_TOKEN")
        && environment
            .iter()
            .any(|(name, _)| name == "NICKEL_SHELL_TEST_CONTROL")
    {
        Ok(environment)
    } else {
        Err("shell capability environment is incomplete".into())
    }
}

fn sibling(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let path = directory.join(name);
    path.is_file().then_some(path).ok_or_else(|| {
        format!(
            "missing {}; build nickel, nickel-test-input, and nickel-nested-acceptance together",
            name
        )
    })
}
