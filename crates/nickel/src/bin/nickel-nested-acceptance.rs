//! Bounded live acceptance harness for Nickel's nested compositor.
//!
//! Build all three participating binaries, then run this binary from the same
//! target directory. The harness uses an isolated runtime directory, launches
//! compositor-owned shell UI, and always asks the compositor to log out before
//! its deadline.

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
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FAIL: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let host_wayland = env::var_os("WAYLAND_DISPLAY").and_then(|display| {
        let display = PathBuf::from(display);
        let path = if display.is_absolute() {
            display
        } else {
            PathBuf::from(env::var_os("XDG_RUNTIME_DIR")?).join(display)
        };
        path.exists().then_some(path)
    });
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

    let mut command = Command::new(&nickel);
    command
        .args(["--backend", "winit", "--test-control"])
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("NICKEL_TEST_CONTROL_ENV_FILE", &capability_file)
        .env("NICKEL_NESTED_SIZE", "960x640")
        // This harness exercises compositor-owned UI, not the independent
        // XWayland startup contract. Avoid letting a host X server delay the
        // control and input assertions.
        .env("NICKEL_DISABLE_XWAYLAND", "1")
        .stdin(Stdio::null());
    if let Some(host_wayland) = host_wayland {
        // Preserve the absolute host socket while isolating the nested
        // compositor's own runtime directory and Wayland listener.
        command
            .env("WINIT_UNIX_BACKEND", "wayland")
            .env("WAYLAND_DISPLAY", host_wayland);
    } else {
        command.env("WINIT_UNIX_BACKEND", "x11");
    }
    let mut compositor = command
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
            break environment;
        }
        if Instant::now() >= deadline {
            return Err("nested compositor did not become ready within 30 seconds".into());
        }
        thread::sleep(POLL);
    };

    // The capability file is published just before the datagram listener is
    // fully dispatchable. Treat that narrow startup window as readiness still
    // pending instead of failing the acceptance run on a transient EAGAIN.
    let readiness = loop {
        match checked(test_input, &environment, &["readiness"]) {
            Ok(readiness) => break readiness,
            Err(error) if Instant::now() < deadline => {
                if let Some(status) = compositor.try_wait().map_err(|error| error.to_string())? {
                    return Err(format!(
                        "nested compositor exited while awaiting readiness: {status}: {error}"
                    ));
                }
                thread::sleep(POLL);
            }
            Err(error) => return Err(format!("readiness failed before deadline: {error}")),
        }
    };
    if !readiness.contains("expected_pid=None authenticated_pid=None") {
        return Err(format!(
            "internal runtime unexpectedly has shell PID authority: {readiness}"
        ));
    }
    let surfaces = checked(test_input, &environment, &["surfaces"])?;
    for role in ["Desktop", "Panel", "Lock", "Launcher"] {
        if !surfaces.contains(role) {
            return Err(format!(
                "surface inventory does not contain {role}: {surfaces:?}"
            ));
        }
    }
    assert_no_shell_child(compositor.id())?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    let toggled = checked(test_input, &environment, &["surfaces"])?;
    let launcher = toggled
        .lines()
        .find(|line| line.starts_with("Launcher\t"))
        .ok_or("internal launcher disappeared after injected Meta input")?;
    if launcher.ends_with("hidden") {
        return Err("injected Meta did not make the internal launcher visible".into());
    }
    // Launcher construction and its first GPU upload are interaction work, not
    // idle work. Let that frame settle before sampling the unchanged runtime.
    thread::sleep(Duration::from_millis(500));
    let before_ticks = process_ticks(compositor.id())?;
    thread::sleep(Duration::from_secs(2));
    let after_ticks = process_ticks(compositor.id())?;
    let idle_ticks = after_ticks.saturating_sub(before_ticks);
    println!(
        "idle diagnostic: compositor_cpu_ticks={} over 2s",
        idle_ticks
    );
    if idle_ticks > 200 {
        return Err(format!(
            "internal runtime consumed {idle_ticks} CPU ticks during bounded idle"
        ));
    }
    Ok(())
}

fn process_ticks(pid: u32) -> Result<u64, String> {
    let stat =
        fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|error| error.to_string())?;
    let fields = stat
        .rsplit_once(") ")
        .ok_or("malformed compositor process stat")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let user = fields.get(11).ok_or("process stat omitted utime")?;
    let system = fields.get(12).ok_or("process stat omitted stime")?;
    Ok(user.parse::<u64>().map_err(|error| error.to_string())?
        + system.parse::<u64>().map_err(|error| error.to_string())?)
}

fn assert_no_shell_child(compositor: u32) -> Result<(), String> {
    for entry in fs::read_dir("/proc").map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let is_child = status
            .lines()
            .any(|line| line == format!("PPid:\t{compositor}"));
        if !is_child {
            continue;
        }
        let command = fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let command = String::from_utf8_lossy(&command).replace('\0', " ");
        if command.contains("--role shell") {
            return Err(format!(
                "unified runtime spawned shell child {pid}: {command}"
            ));
        }
    }
    Ok(())
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
