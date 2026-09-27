// Bounded live acceptance harness for Nickel's nested compositor.
//
// Build all three participating binaries, then run this binary from the same
// target directory. The harness uses an isolated runtime directory, launches
// compositor-owned shell UI, and always asks the compositor to log out before
// its deadline.

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
    let nickel = sibling(directory, "nickel-nested")?;
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
    let plugin = runtime
        .join("config/nickel/plugins/org.example.acceptance-panel");
    fs::create_dir_all(&plugin).map_err(|error| error.to_string())?;
    fs::write(
        plugin.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.acceptance-panel","name":"Acceptance Panel","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":360,"height":96,"bottom_offset":12,"output":"primary"}],"settings":[{"id":"show-label","label":"Show label","kind":"boolean","default":true}]}"#,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        plugin.join("main.js"),
        "function App() { return h(Panel, {}, h(Text, {}, nickel.data.settings['show-label'] ? 'On' : 'Off')); }",
    )
    .map_err(|error| error.to_string())?;
    let capability_file = runtime.join("shell-environment");

    let mut command = Command::new(&nickel);
    command
        .arg("--test-control")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", runtime.join("config"))
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
        command.env("WAYLAND_DISPLAY", host_wayland);
    } else {
        // Winit 0.31 selects Wayland whenever either selector is present; it no longer honors
        // WINIT_UNIX_BACKEND. Remove inherited selectors so DISPLAY authoritatively selects X11.
        command
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET");
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
            "PASS: nested compositor ran bundled UI and an installed panel, changed a live plugin setting, measured plugin UI memory, confirmed launcher fallback and restart through scoped test input, and shut down cleanly"
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
    let plugin_output = checked(test_input, &environment, &["plugins"])?;
    let plugins: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&plugin_output).map_err(|error| error.to_string())?;
    for id in [
        "org.nickel.taskbar",
        "org.nickel.launcher",
        "org.nickel.run",
        "org.nickel.notification",
        "org.nickel.volume-osd",
        "org.nickel.control-center",
        "org.nickel.window-preview",
        "org.nickel.desktop",
    ] {
        let plugin = plugins
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or_else(|| format!("missing bundled plugin {id}"))?;
        if !plugin.desired_enabled
            || plugin.health != nickel_session_protocol::PluginRuntimeHealth::Running
        {
            return Err(format!("bundled plugin {id} is not running: {:?}", plugin.health));
        }
    }
    assert_no_shell_child(compositor.id())?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, true, Duration::from_secs(2))?;
    let launcher_memory = wait_for_plugin_native_memory(
        test_input,
        &environment,
        "org.nickel.launcher",
        Duration::from_secs(2),
    )?;
    if launcher_memory == 0 {
        return Err("rendered launcher reported zero native UI memory".into());
    }
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;
    let disabled = checked(
        test_input,
        &environment,
        &["plugin-set", "org.nickel.launcher", "disabled"],
    )?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let launcher_status = disabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == "org.nickel.launcher")
        .ok_or("launcher missing after disable")?;
    if launcher_status.desired_enabled
        || launcher_status.health != nickel_session_protocol::PluginRuntimeHealth::Disabled
        || launcher_status.memory.native_ui_bytes.is_some()
    {
        return Err("launcher did not retire and clear reported UI memory".into());
    }
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, true, Duration::from_secs(2))?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;
    let enabled = checked(
        test_input,
        &environment,
        &["plugin-set", "org.nickel.launcher", "enabled"],
    )?;
    let enabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&enabled).map_err(|error| error.to_string())?;
    let launcher_status = enabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == "org.nickel.launcher")
        .ok_or("launcher missing after re-enable")?;
    if !launcher_status.desired_enabled
        || launcher_status.health != nickel_session_protocol::PluginRuntimeHealth::Running
    {
        return Err("launcher did not resume after re-enable".into());
    }
    let panel_id = "org.example.acceptance-panel";
    let activated = checked(test_input, &environment, &["plugin-set", panel_id, "enabled"])?;
    let activated: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&activated).map_err(|error| error.to_string())?;
    let panel = activated
        .plugins
        .iter()
        .find(|plugin| plugin.id == panel_id)
        .ok_or("installed panel missing after enable")?;
    if !panel.desired_enabled
        || panel.health != nickel_session_protocol::PluginRuntimeHealth::Running
        || panel.settings.first().map(|setting| &setting.value) != Some(&serde_json::json!(true))
    {
        return Err("installed panel did not start with its declared setting".into());
    }
    wait_for_plugin_native_memory(test_input, &environment, panel_id, Duration::from_secs(5))?;
    let changed = checked(
        test_input,
        &environment,
        &["plugin-setting", panel_id, "show-label", "false"],
    )?;
    let changed: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&changed).map_err(|error| error.to_string())?;
    let panel = changed
        .plugins
        .iter()
        .find(|plugin| plugin.id == panel_id)
        .ok_or("installed panel missing after setting change")?;
    if !panel.desired_enabled
        || panel.health != nickel_session_protocol::PluginRuntimeHealth::Running
        || panel.settings.first().map(|setting| &setting.value) != Some(&serde_json::json!(false))
        || changed.activation_generation <= activated.activation_generation
    {
        return Err("installed panel setting was not applied to the running plugin".into());
    }
    wait_for_plugin_native_memory(test_input, &environment, panel_id, Duration::from_secs(5))?;
    let disabled = checked(test_input, &environment, &["plugin-set", panel_id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let panel = disabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == panel_id)
        .ok_or("installed panel missing after disable")?;
    if panel.desired_enabled || panel.memory.native_ui_bytes.is_some() {
        return Err("installed panel did not release its reported UI memory".into());
    }
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
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;

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

fn wait_for_launcher_visibility(
    test_input: &Path,
    environment: &[(String, String)],
    expected_visible: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let launcher = surfaces
            .lines()
            .find(|line| line.starts_with("Launcher\t"))
            .ok_or("internal launcher disappeared while awaiting visibility")?;
        if launcher.ends_with("hidden") != expected_visible {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "launcher did not become {} before deadline: {launcher}",
                if expected_visible {
                    "visible"
                } else {
                    "hidden"
                }
            ));
        }
        thread::sleep(POLL);
    }
}

fn wait_for_plugin_native_memory(
    test_input: &Path,
    environment: &[(String, String)],
    id: &str,
    timeout: Duration,
) -> Result<u64, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let output = checked(test_input, environment, &["plugins"])?;
        let snapshot: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&output).map_err(|error| error.to_string())?;
        let plugin = snapshot
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or_else(|| format!("missing plugin {id} while awaiting memory"))?;
        if let Some(bytes) = plugin.memory.native_ui_bytes {
            if bytes > 0 && plugin.memory.tracked_peak_bytes.is_some_and(|peak| peak >= bytes) {
                return Ok(bytes);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "plugin {id} did not report rendered UI memory: health={:?}, native={:?}, peak={:?}",
                plugin.health, plugin.memory.native_ui_bytes, plugin.memory.tracked_peak_bytes,
            ));
        }
        thread::sleep(POLL);
    }
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
            "missing {}; build nickel-nested, nickel-test-input, and nickel-nested-acceptance together",
            name
        )
    })
}
