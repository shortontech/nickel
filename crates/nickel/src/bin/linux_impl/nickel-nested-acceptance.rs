// Bounded live acceptance harness for Nickel's nested compositor.
//
// Build the participating binaries, then run this binary from the same
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

const DEADLINE: Duration = Duration::from_secs(45);
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
    let settings = sibling(directory, "nickel-settings")?;
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
        r#"{"api_version":1,"id":"org.example.acceptance-panel","name":"Acceptance Panel","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":360,"height":96,"bottom_offset":12,"output":"all"}],"settings":[{"id":"show-label","label":"Show label","kind":"boolean","default":true}]}"#,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        plugin.join("main.js"),
        "function App() { return h(FixedWindow, {width: 360, height: 96}, h(Text, {}, nickel.data.settings['show-label'] ? 'On' : 'Off')); }",
    )
    .map_err(|error| error.to_string())?;
    let reserved = runtime
        .join("config/nickel/plugins/org.example.reserved-panel");
    fs::create_dir_all(&reserved).map_err(|error| error.to_string())?;
    fs::write(
        reserved.join("plugin.json"),
        include_str!("../../../../../assets/plugins/example-reserved-panel/plugin.json"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        reserved.join("main.js"),
        include_str!("../../../../../assets/plugins/example-reserved-panel/main.js"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        reserved.join("ui.css"),
        include_str!("../../../../../assets/plugins/example-reserved-panel/ui.css"),
    )
    .map_err(|error| error.to_string())?;
    let window = runtime
        .join("config/nickel/plugins/org.example.component-window");
    fs::create_dir_all(&window).map_err(|error| error.to_string())?;
    fs::write(
        window.join("plugin.json"),
        include_str!("../../../../../assets/plugins/example-window/plugin.json"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        window.join("main.js"),
        include_str!("../../../../../assets/plugins/example-window/main.js"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        window.join("ui.css"),
        include_str!("../../../../../assets/plugins/example-window/ui.css"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        window.join("icon.png"),
        include_bytes!("../../../../../assets/plugins/example-window/icon.png"),
    )
    .map_err(|error| error.to_string())?;
    let windows = runtime
        .join("config/nickel/plugins/org.example.acceptance-windows");
    fs::create_dir_all(&windows).map_err(|error| error.to_string())?;
    fs::write(
        windows.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.acceptance-windows","name":"Acceptance Windows","entry":"main.js","surfaces":[{"id":"first","kind":"window","width":400,"height":240},{"id":"second","kind":"window","width":450,"height":260}]}"#,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        windows.join("main.js"),
        "function App() { return nickel.data.surface.id === 'first' ? h(Window, {width: 360, height: 220}, h(Button, {id: 'reopen', onClick: () => nickel.request({type: 'show-plugin-surface', surfaceId: 'second'})}, 'Reopen second')) : h(Window, {width: 420, height: 240}, h(Button, {id: 'hide', onClick: () => nickel.request({type: 'hide-plugin-surface', surfaceId: 'second'})}, 'Hide second')); }",
    )
    .map_err(|error| error.to_string())?;
    let dialog = runtime
        .join("config/nickel/plugins/org.example.surface-dialog");
    fs::create_dir_all(&dialog).map_err(|error| error.to_string())?;
    fs::write(
        dialog.join("plugin.json"),
        include_str!("../../../../../assets/plugins/example-surface-dialog/plugin.json"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        dialog.join("main.js"),
        include_str!("../../../../../assets/plugins/example-surface-dialog/main.js"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        dialog.join("ui.css"),
        include_str!("../../../../../assets/plugins/example-surface-dialog/ui.css"),
    )
    .map_err(|error| error.to_string())?;
    let overlay = runtime.join("config/nickel/plugins/org.example.overlay");
    fs::create_dir_all(&overlay).map_err(|error| error.to_string())?;
    fs::write(
        overlay.join("plugin.json"),
        include_str!("../../../../../assets/plugins/example-overlay/plugin.json"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        overlay.join("main.js"),
        include_str!("../../../../../assets/plugins/example-overlay/main.js"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        overlay.join("ui.css"),
        include_str!("../../../../../assets/plugins/example-overlay/ui.css"),
    )
    .map_err(|error| error.to_string())?;
    for (id, manifest, source, stylesheet) in [
        (
            "org.example.widget-host",
            include_str!("../../../../../assets/plugins/example-widget-host/plugin.json"),
            include_str!("../../../../../assets/plugins/example-widget-host/main.js"),
            Some(include_str!("../../../../../assets/plugins/example-widget-host/ui.css")),
        ),
        (
            "org.example.widget-contributor",
            include_str!("../../../../../assets/plugins/example-widget-contributor/plugin.json"),
            include_str!("../../../../../assets/plugins/example-widget-contributor/main.js"),
            None,
        ),
        (
            "org.example.action-contributor",
            include_str!("../../../../../assets/plugins/example-action-contributor/plugin.json"),
            include_str!("../../../../../assets/plugins/example-action-contributor/main.js"),
            None,
        ),
    ] {
        let directory = runtime.join("config/nickel/plugins").join(id);
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        fs::write(directory.join("plugin.json"), manifest).map_err(|error| error.to_string())?;
        fs::write(directory.join("main.js"), source).map_err(|error| error.to_string())?;
        if let Some(stylesheet) = stylesheet {
            fs::write(directory.join("ui.css"), stylesheet).map_err(|error| error.to_string())?;
        }
    }
    let capability_file = runtime.join("shell-environment");

    let mut command = Command::new(&nickel);
    command
        .arg("--test-control")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", runtime.join("config"))
        .env("NICKEL_TEST_CONTROL_ENV_FILE", &capability_file)
        .env("NICKEL_NESTED_SIZE", "960x640")
        .env("NICKEL_ON_SCREEN_KEYBOARD", "1")
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

    let result = exercise(&mut compositor, &test_input, &settings, &capability_file);
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
        "PASS: nested compositor ran bundled UI, screenshot plugin input and lifecycle, an installed panel, component and standalone dialogs, a plugin overlay, and sibling windows; checked live component layouts, typed surface hide, owner-close retirement, memory, launcher plugin retirement, and clean shutdown"
    );
    Ok(())
}

fn exercise(
    compositor: &mut Child,
    test_input: &Path,
    settings: &Path,
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
            Ok(readiness)
                if readiness.contains("panels=1")
                    && readiness.contains("output_roles_ready=true") =>
            {
                break readiness;
            }
            Ok(readiness) if Instant::now() >= deadline => {
                let surfaces = checked(test_input, &environment, &["surfaces"])
                    .unwrap_or_else(|error| format!("surface inventory unavailable: {error}"));
                return Err(format!("panel readiness did not settle: {readiness}; {surfaces}"));
            }
            Ok(_) => thread::sleep(POLL),
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
    for role in ["Desktop", "Lock", "Launcher"] {
        if !surfaces.contains(role) {
            return Err(format!(
                "surface inventory does not contain {role}: {surfaces:?}"
            ));
        }
    }
    if !surfaces.lines().any(|line| {
        line.starts_with("Panel\twinit\t")
            && line.ends_with("org.nickel.taskbar/main")
    }) {
        return Err(format!("taskbar plugin surface is missing: {surfaces:?}"));
    }
    if !surfaces.lines().any(|line| {
        line.starts_with("Desktop\twinit\t")
            && line.ends_with("org.nickel.desktop/main")
    }) {
        return Err(format!("desktop plugin surface is missing: {surfaces:?}"));
    }
    if !surfaces.lines().any(|line| {
        line.starts_with("Preview\tunmapped\t")
            && line.ends_with("org.nickel.window-preview/main")
    }) {
        return Err(format!("preview plugin surface is missing: {surfaces:?}"));
    }
    verify_layout_snapshot(test_input, &environment, "org.nickel.taskbar/main")?;
    verify_layout_snapshot(test_input, &environment, "org.nickel.desktop/main")?;
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
        "org.nickel.codex-projects",
        "org.nickel.on-screen-keyboard",
        "org.nickel.window-preview",
        "org.nickel.desktop",
        "org.nickel.screenshot",
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
    verify_layout_snapshot(test_input, &environment, "org.nickel.launcher/main")?;
    let launcher_memory = wait_for_plugin_native_memory(
        test_input,
        &environment,
        "org.nickel.launcher",
        Duration::from_secs(2),
    )?;
    if launcher_memory == 0 {
        return Err("rendered launcher reported zero native UI memory".into());
    }
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
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    thread::sleep(Duration::from_millis(250));
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;
    verify_run_plugin_owns_dialog(test_input, &environment)?;
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
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, true, Duration::from_secs(2))?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, &environment, false, Duration::from_secs(2))?;
    verify_taskbar_plugin_retires(test_input, &environment)?;
    verify_desktop_plugin_retires(test_input, &environment)?;
    verify_bundled_overlay_surface_retires(test_input, &environment, "org.nickel.volume-osd", "VolumeOsd")?;
    verify_bundled_overlay_surface_retires(test_input, &environment, "org.nickel.window-preview", "Preview")?;
    verify_control_plugin_retires(test_input, &environment)?;
    verify_codex_project_plugin_retires(test_input, &environment)?;
    verify_keyboard_plugin_retires(test_input, &environment)?;
    verify_notification_plugin_retires(test_input, &environment)?;
    verify_reserved_panel_stacks_and_reflows(test_input, &environment)?;
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
    wait_for_plugin_panel_on_output(
        test_input,
        &environment,
        "winit",
        true,
        Duration::from_secs(5),
    )?;
    wait_for_plugin_native_memory(test_input, &environment, panel_id, Duration::from_secs(5))?;
    checked(
        test_input,
        &environment,
        &["output-connect", "DP-plugin-test", "1024", "768", "180", "normal"],
    )?;
    let outputs = checked(test_input, &environment, &["outputs"])?;
    if !outputs
        .lines()
        .any(|line| line.starts_with("DP-plugin-test\t") && line.contains("\tscale=180/120\t"))
    {
        return Err(format!("scaled plugin test output is missing: {outputs}"));
    }
    wait_for_plugin_panel_on_output(
        test_input,
        &environment,
        "DP-plugin-test",
        true,
        Duration::from_secs(5),
    )?;
    checked(test_input, &environment, &["output-disconnect", "DP-plugin-test"])?;
    wait_for_plugin_panel_on_output(
        test_input,
        &environment,
        "DP-plugin-test",
        false,
        Duration::from_secs(5),
    )?;
    wait_for_plugin_panel_on_output(
        test_input,
        &environment,
        "winit",
        true,
        Duration::from_secs(5),
    )?;
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
    verify_component_window(test_input, &environment)?;
    verify_generic_widget_slot(test_input, &environment)?;
    verify_sibling_windows(test_input, &environment)?;
    verify_separate_plugin_dialog(test_input, &environment)?;
    verify_separate_plugin_overlay(test_input, &environment)?;
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    let toggled = checked(test_input, &environment, &["surfaces"])?;
    let launcher = toggled
        .lines()
        .find(|line| line.starts_with("Launcher\t"))
        .ok_or("internal launcher disappeared after injected Meta input")?;
    if !launcher_line_visible(launcher) {
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
    verify_settings_memory_report(settings, test_input, &environment)?;
    verify_screenshot_plugin_lifecycle(test_input, &environment)?;
    Ok(())
}

fn press_super_r(test_input: &Path, environment: &[(String, String)]) -> Result<(), String> {
    press_super_key(test_input, environment, "r")
}

fn press_super_key(
    test_input: &Path,
    environment: &[(String, String)],
    key: &str,
) -> Result<(), String> {
    checked(test_input, environment, &["key", "meta", "pressed"])?;
    checked(test_input, environment, &["key", key, "pressed"])?;
    checked(test_input, environment, &["key", key, "released"])?;
    checked(test_input, environment, &["key", "meta", "released"])?;
    Ok(())
}

fn verify_run_plugin_owns_dialog(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.run";
    press_super_r(test_input, environment)?;
    wait_for_launcher_visibility(test_input, environment, true, Duration::from_secs(5))?;
    wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled.plugins.iter().find(|plugin| plugin.id == id).ok_or("Run plugin missing")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled Run plugin retained its UI memory".into());
    }
    wait_for_launcher_visibility(test_input, environment, false, Duration::from_secs(5))?;
    wait_for_role_presence(test_input, environment, "Launcher", false)?;
    press_super_r(test_input, environment)?;
    thread::sleep(Duration::from_millis(250));
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if surfaces
        .lines()
        .any(|line| line.starts_with("Launcher\t") && launcher_line_visible(line))
    {
        return Err(format!("disabled Run opened a fallback dialog: {surfaces}"));
    }
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    wait_for_role_presence(test_input, environment, "Launcher", true)?;
    press_super_r(test_input, environment)?;
    wait_for_launcher_visibility(test_input, environment, true, Duration::from_secs(5))?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_launcher_visibility(test_input, environment, false, Duration::from_secs(5))?;
    Ok(())
}

fn verify_taskbar_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.taskbar";
    wait_for_taskbar_presence(test_input, environment, true, Duration::from_secs(2))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled.plugins.iter().find(|plugin| plugin.id == id).ok_or("taskbar missing")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled taskbar retained native UI memory".into());
    }
    wait_for_taskbar_presence(test_input, environment, false, Duration::from_secs(2))?;
    wait_for_role_presence(test_input, environment, "ContextMenu", false)?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    wait_for_taskbar_presence(test_input, environment, true, Duration::from_secs(2))?;
    wait_for_role_presence(test_input, environment, "ContextMenu", true)?;
    Ok(())
}

fn panel_geometry(surfaces: &str, key: &str) -> Option<(i32, i32, u32, u32)> {
    surface_geometry(surfaces, "Panel", key)
}

fn surface_geometry(surfaces: &str, role: &str, key: &str) -> Option<(i32, i32, u32, u32)> {
    let geometry = surfaces
        .lines()
        .find(|line| line.starts_with(&format!("{role}\twinit\t")) && line.ends_with(key))?
        .split('\t')
        .nth(2)?;
    let (origin, size) = geometry.split_once(' ')?;
    let (x, y) = origin.split_once(',')?;
    let (width, height) = size.split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?, width.parse().ok()?, height.parse().ok()?))
}

fn verify_reserved_panel_stacks_and_reflows(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    const TASKBAR: &str = "org.nickel.taskbar/main";
    const RESERVED: &str = "org.example.reserved-panel/main";
    let before = checked(test_input, environment, &["surfaces"])?;
    let baseline = panel_geometry(&before, TASKBAR).ok_or("taskbar geometry unavailable")?;
    checked(test_input, environment, &["plugin-set", "org.example.reserved-panel", "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if let (Some(taskbar), Some(reserved)) =
            (panel_geometry(&surfaces, TASKBAR), panel_geometry(&surfaces, RESERVED))
        {
            let taskbar_bottom = taskbar.1 + taskbar.3 as i32;
            let reserved_bottom = reserved.1 + reserved.3 as i32;
            if taskbar.0 != reserved.0
                || taskbar.2 != reserved.2
                || !(taskbar_bottom <= reserved.1 || reserved_bottom <= taskbar.1)
                || taskbar_bottom.max(reserved_bottom) != baseline.1 + baseline.3 as i32
            {
                return Err(format!("reserved panels did not stack on one output: {surfaces}"));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("reserved panel did not appear: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    checked(test_input, environment, &["plugin-set", "org.example.reserved-panel", "disabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if panel_geometry(&surfaces, RESERVED).is_none()
            && panel_geometry(&surfaces, TASKBAR) == Some(baseline)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("taskbar did not reflow after reserved panel retirement: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_desktop_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.desktop";
    wait_for_desktop_presence(test_input, environment, true, Duration::from_secs(2))?;
    wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(2))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled.plugins.iter().find(|plugin| plugin.id == id).ok_or("desktop missing")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled desktop retained native UI memory".into());
    }
    wait_for_desktop_presence(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    wait_for_desktop_presence(test_input, environment, true, Duration::from_secs(2))?;
    wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(2))?;
    Ok(())
}

fn wait_for_desktop_presence(
    test_input: &Path,
    environment: &[(String, String)],
    expected_present: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let present = surfaces
            .lines()
            .any(|line| line.starts_with("Desktop\twinit\t"));
        if present == expected_present {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("desktop surface presence did not become {expected_present}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_bundled_overlay_surface_retires(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_id: &str,
    role: &str,
) -> Result<(), String> {
    wait_for_role_presence(test_input, environment, role, true)?;
    checked(test_input, environment, &["plugin-set", plugin_id, "disabled"])?;
    wait_for_role_presence(test_input, environment, role, false)?;
    checked(test_input, environment, &["plugin-set", plugin_id, "enabled"])?;
    wait_for_role_presence(test_input, environment, role, true)
}

fn wait_for_role_presence(
    test_input: &Path,
    environment: &[(String, String)],
    role: &str,
    expected: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let present = surfaces.lines().any(|line| line.starts_with(&format!("{role}\t")));
        if present == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("{role} surface presence did not become {expected}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_control_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.control-center";
    press_super_key(test_input, environment, "a")?;
    wait_for_control_visibility(test_input, environment, true, Duration::from_secs(2))?;
    thread::sleep(Duration::from_millis(250));
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "a")?;
    wait_for_control_visibility(test_input, environment, true, Duration::from_secs(2))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled.plugins.iter().find(|plugin| plugin.id == id).ok_or("control plugin missing")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled Control Center retained native UI memory".into());
    }
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "a")?;
    thread::sleep(Duration::from_millis(250));
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "a")?;
    wait_for_control_visibility(test_input, environment, true, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    checked(test_input, environment, &["semantic", "control-center", "open"])?;
    wait_for_control_visibility(test_input, environment, true, Duration::from_secs(2))?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_control_visibility(test_input, environment, false, Duration::from_secs(2))?;
    Ok(())
}

fn verify_codex_project_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.codex-projects";
    let surface_present = |surfaces: &str| {
        surfaces.lines().any(|line| {
            line.starts_with("PluginSurface\t")
                && line.ends_with("org.nickel.codex-projects/main")
        })
    };
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if surfaces.lines().any(|line| {
        line.starts_with("CodexProjectMenu\t") || line.starts_with("ProjectMenu\t")
    }) {
        return Err(format!("retired Codex menu surface is still present: {surfaces}"));
    }
    if !surface_present(&surfaces) {
        return Err(format!("Codex project plugin surface is missing: {surfaces}"));
    }
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or("Codex project plugin missing after disable")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled Codex project plugin retained native UI memory".into());
    }
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if surface_present(&surfaces) {
        return Err(format!("Codex project plugin surface survived disable: {surfaces}"));
    }
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if !surface_present(&surfaces) {
        return Err(format!("Codex project plugin surface did not return: {surfaces}"));
    }
    Ok(())
}

fn verify_keyboard_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.on-screen-keyboard";
    let surface_present = |surfaces: &str| {
        surfaces.lines().any(|line| {
            line.starts_with("PluginSurface\t")
                && line.ends_with("org.nickel.on-screen-keyboard/main")
        })
    };
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if !surface_present(&surfaces) {
        return Err(format!("keyboard plugin surface is missing: {surfaces}"));
    }
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or("keyboard plugin missing after disable")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled keyboard plugin retained native UI memory".into());
    }
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if surface_present(&surfaces) {
        return Err(format!("keyboard plugin surface survived disable: {surfaces}"));
    }
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if !surface_present(&surfaces) {
        return Err(format!("keyboard plugin surface did not return: {surfaces}"));
    }
    checked(test_input, environment, &["semantic", "keyboard-toggle"])?;
    wait_for_keyboard_visibility(test_input, environment, true, Duration::from_secs(2))?;
    if wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(2))? == 0 {
        return Err("visible keyboard plugin reported zero native UI memory".into());
    }
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if surface_present(&surfaces) {
        return Err(format!("visible keyboard plugin survived disable: {surfaces}"));
    }
    wait_for_keyboard_visibility(test_input, environment, true, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if !surface_present(&surfaces) {
        return Err(format!("visible keyboard plugin did not return: {surfaces}"));
    }
    checked(test_input, environment, &["semantic", "keyboard", "osk-hide"])?;
    wait_for_keyboard_visibility(test_input, environment, false, Duration::from_secs(2))?;
    Ok(())
}

fn wait_for_keyboard_visibility(
    test_input: &Path,
    environment: &[(String, String)],
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = checked(test_input, environment, &["keyboard-status"])?;
        let snapshot: nickel_session_protocol::OnScreenKeyboardSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        if snapshot.visible == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("keyboard visibility did not become {expected}: {status}"));
        }
        thread::sleep(POLL);
    }
}

fn wait_for_control_visibility(
    test_input: &Path,
    environment: &[(String, String)],
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let line = surfaces.lines().find(|line| {
            line.starts_with("PluginSurface\t")
                && line.ends_with("org.nickel.control-center/main")
        });
        let visible = line.is_some_and(|line| line.split('\t').nth(2) != Some("hidden"));
        if visible == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("Control Center visibility stayed {visible}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_notification_plugin_retires(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.nickel.notification";
    press_super_key(test_input, environment, "n")?;
    wait_for_notification_visibility(test_input, environment, true, Duration::from_secs(2))?;
    thread::sleep(Duration::from_millis(250));
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_notification_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "n")?;
    wait_for_notification_visibility(test_input, environment, true, Duration::from_secs(2))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let plugin = disabled.plugins.iter().find(|plugin| plugin.id == id).ok_or("notification plugin missing")?;
    if plugin.desired_enabled || plugin.memory.native_ui_bytes.is_some() {
        return Err("disabled notification plugin retained native UI memory".into());
    }
    wait_for_notification_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "n")?;
    thread::sleep(Duration::from_millis(250));
    wait_for_notification_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    wait_for_notification_visibility(test_input, environment, false, Duration::from_secs(2))?;
    press_super_key(test_input, environment, "n")?;
    wait_for_notification_visibility(test_input, environment, true, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    wait_for_notification_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    Ok(())
}

fn wait_for_notification_visibility(
    test_input: &Path,
    environment: &[(String, String)],
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let line = surfaces.lines().find(|line| {
            line.starts_with("PluginSurface\t")
                && line.ends_with("org.nickel.notification/main")
        });
        let visible = line.is_some_and(|line| line.split('\t').nth(2) != Some("hidden"));
        if visible == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("notification visibility stayed {visible}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn wait_for_screenshot_visibility(
    test_input: &Path,
    environment: &[(String, String)],
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let visible = surfaces.lines().any(|line| {
            line.starts_with("PluginSurface\t")
                && line.ends_with("org.nickel.screenshot/main")
                && line.split('\t').nth(2) != Some("hidden")
        });
        if visible == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("screenshot visibility did not become {expected}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_screenshot_plugin_lifecycle(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let press_print_screen = || -> Result<(), String> {
        checked(test_input, environment, &["key", "print-screen", "pressed"])?;
        checked(test_input, environment, &["key", "print-screen", "released"])?;
        Ok(())
    };
    press_print_screen()?;
    wait_for_screenshot_visibility(test_input, environment, true, Duration::from_secs(5))?;
    if wait_for_plugin_native_memory(
        test_input,
        environment,
        "org.nickel.screenshot",
        Duration::from_secs(2),
    )? == 0
    {
        return Err("rendered screenshot plugin reported zero native UI memory".into());
    }
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    let (x, y, width, height) = surface_geometry(
        &surfaces,
        "PluginSurface",
        "org.nickel.screenshot/main",
    )
    .ok_or_else(|| format!("screenshot plugin geometry is missing: {surfaces}"))?;
    let start = (x + width as i32 / 4, y + height as i32 / 4);
    let end = (x + width as i32 * 3 / 4, y + height as i32 * 3 / 4);
    checked(test_input, environment, &["move", &start.0.to_string(), &start.1.to_string()])?;
    checked(test_input, environment, &["button", "left", "pressed"])?;
    checked(test_input, environment, &["move", &end.0.to_string(), &end.1.to_string()])?;
    checked(test_input, environment, &["button", "left", "released"])?;
    let center = (x + width as i32 / 2, y + height as i32 / 2);
    click_at(test_input, environment, center.0, center.1)?;
    click_at(test_input, environment, center.0, center.1)?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_screenshot_visibility(test_input, environment, false, Duration::from_secs(2))?;

    checked(
        test_input,
        environment,
        &["plugin-set", "org.nickel.screenshot", "disabled"],
    )?;
    press_print_screen()?;
    wait_for_screenshot_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(
        test_input,
        environment,
        &["plugin-set", "org.nickel.screenshot", "enabled"],
    )?;
    press_print_screen()?;
    wait_for_screenshot_visibility(test_input, environment, true, Duration::from_secs(5))?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_screenshot_visibility(test_input, environment, false, Duration::from_secs(2))?;
    Ok(())
}

fn wait_for_taskbar_presence(
    test_input: &Path,
    environment: &[(String, String)],
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let present = surfaces.lines().any(|line| {
            line.starts_with("Panel\t")
                && line.ends_with("org.nickel.taskbar/main")
        });
        if present == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("taskbar surface presence remained {present}: {surfaces}"));
        }
        thread::sleep(POLL);
    }
}

fn verify_generic_widget_slot(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let host = "org.example.widget-host";
    let contributor = "org.example.widget-contributor";
    checked(test_input, environment, &["plugin-set", host, "enabled"])?;
    let base = wait_for_plugin_native_memory(test_input, environment, host, Duration::from_secs(5))?;
    checked(test_input, environment, &["plugin-set", contributor, "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let contributed = loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let provider = status.plugins.iter().find(|plugin| plugin.id == host);
        let extension = status.plugins.iter().find(|plugin| plugin.id == contributor);
        if let (Some(provider), Some(extension)) = (provider, extension)
            && let Some(bytes) = provider.memory.native_ui_bytes
            && bytes > base
            && extension.health == nickel_session_protocol::PluginRuntimeHealth::Running
            && extension.memory.native_ui_bytes.is_some_and(|bytes| bytes > 0)
        {
            break bytes;
        }
        if Instant::now() >= deadline {
            return Err(format!("generic widget did not enter its provider: {status:?}"));
        }
        thread::sleep(POLL);
    };
    checked(
        test_input,
        environment,
        &["plugin-setting", contributor, "unread-count", "12345"],
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let contributed = loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let provider = status.plugins.iter().find(|plugin| plugin.id == host);
        if let Some(bytes) = provider.and_then(|plugin| plugin.memory.native_ui_bytes)
            && bytes > contributed
        {
            break bytes;
        }
        if Instant::now() >= deadline {
            return Err(format!("generic widget setting did not refresh its provider: {status:?}"));
        }
        thread::sleep(POLL);
    };
    checked(test_input, environment, &["plugin-set", contributor, "disabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let provider = status.plugins.iter().find(|plugin| plugin.id == host);
        let extension = status.plugins.iter().find(|plugin| plugin.id == contributor);
        if let (Some(provider), Some(extension)) = (provider, extension)
            && provider.memory.native_ui_bytes.is_some_and(|bytes| bytes < contributed)
            && !extension.desired_enabled
            && extension.memory.native_ui_bytes.is_none()
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("generic widget did not retire from its provider: {status:?}"));
        }
        thread::sleep(POLL);
    }
    let action = "org.example.action-contributor";
    checked(test_input, environment, &["plugin-set", action, "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let provider = status.plugins.iter().find(|plugin| plugin.id == host);
        let extension = status.plugins.iter().find(|plugin| plugin.id == action);
        if let (Some(provider), Some(extension)) = (provider, extension)
            && provider.memory.native_ui_bytes.is_some_and(|bytes| bytes > base)
            && extension.health == nickel_session_protocol::PluginRuntimeHealth::Running
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("generic action did not enter its provider: {status:?}"));
        }
        thread::sleep(POLL);
    }
    let window = wait_for_plugin_window(
        test_input,
        environment,
        "org.example.widget-host",
        "Widget Host Example",
        Duration::from_secs(5),
    )?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.widget-host/main",
        "slot-action-open-launcher",
        (window.1, window.2),
    )?;
    wait_for_launcher_visibility(test_input, environment, true, Duration::from_secs(5))?;
    checked(test_input, environment, &["key", "meta", "pressed"])?;
    checked(test_input, environment, &["key", "meta", "released"])?;
    wait_for_launcher_visibility(test_input, environment, false, Duration::from_secs(2))?;
    checked(test_input, environment, &["plugin-set", action, "disabled"])?;
    checked(test_input, environment, &["plugin-set", host, "disabled"])?;
    Ok(())
}

fn verify_component_window(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.example.component-window";
    let enabled = checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let enabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&enabled).map_err(|error| error.to_string())?;
    let status = enabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or("component window missing after enable")?;
    if !status.desired_enabled
        || status.health != nickel_session_protocol::PluginRuntimeHealth::Running
    {
        return Err("component window plugin did not start".into());
    }
    let initial_bytes =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    if initial_bytes < 287 * 287 * 4 {
        return Err(format!(
            "component window did not account for its decoded plugin image: {initial_bytes} bytes"
        ));
    }
    let first_window = wait_for_component_window(test_input, environment, Duration::from_secs(5))?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.component-window/main",
        "open-dialog",
        (first_window.1, first_window.2),
    )?;
    wait_for_component_dialog_memory(test_input, environment, initial_bytes)?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.component-window/main",
        "show-settings",
        (first_window.1, first_window.2),
    )?;
    wait_for_settings_memory(test_input, environment, true, Duration::from_secs(8))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let settings_window = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if let Some(id) = windows.lines().find_map(|line| {
            line.contains("\tNickel Settings\t")
                .then(|| line.split('\t').next()?.parse::<u64>().ok())
                .flatten()
        }) {
            break id;
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin action launched no Settings window: {windows}"));
        }
        thread::sleep(POLL);
    };
    checked(
        test_input,
        environment,
        &["semantic", "window", &settings_window.to_string(), "click"],
    )?;
    checked(test_input, environment, &["key", "alt", "pressed"])?;
    checked(test_input, environment, &["key", "f4", "pressed"])?;
    checked(test_input, environment, &["key", "f4", "released"])?;
    checked(test_input, environment, &["key", "alt", "released"])?;
    wait_for_settings_memory_expiry(test_input, environment, Duration::from_secs(7))?;
    let disabled = checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    let disabled: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
    let status = disabled
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or("component window missing after disable")?;
    if status.desired_enabled || status.memory.native_ui_bytes.is_some() {
        return Err("component window did not release its reported UI memory".into());
    }
    let windows = checked(test_input, environment, &["windows"])?;
    if windows
        .lines()
        .any(|line| line.contains("\torg.example.component-window\tComponent Window Example\t"))
    {
        return Err(format!("component window remained registered after disable: {windows}"));
    }
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let reopened = wait_for_component_window(test_input, environment, Duration::from_secs(5))?;
    if reopened.0 == first_window.0 {
        return Err("component window reused its retired window identity".into());
    }
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    Ok(())
}

fn wait_for_component_window(
    test_input: &Path,
    environment: &[(String, String)],
    timeout: Duration,
) -> Result<(u64, i32, i32), String> {
    wait_for_plugin_window(
        test_input,
        environment,
        "org.example.component-window",
        "Component Window Example",
        timeout,
    )
}

fn wait_for_plugin_window(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_id: &str,
    title: &str,
    timeout: Duration,
) -> Result<(u64, i32, i32), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if let Some(window) = windows.lines().find_map(|line| {
            (line.contains(&format!("\t{plugin_id}\t{title}\t"))
                && line.contains("\tshown\t")
                && !line.ends_with("\tunmapped"))
            .then(|| {
                let id = line.split('\t').next()?.parse::<u64>().ok()?;
                let location = line.rsplit('\t').next()?.split_whitespace().next()?;
                let (x, y) = location.split_once(',')?;
                Some((id, x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))
            })
            .flatten()
        }) {
            return Ok(window);
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin window {plugin_id} did not enter the window registry: {windows}"));
        }
        thread::sleep(POLL);
    }
}

fn click_at(
    test_input: &Path,
    environment: &[(String, String)],
    x: i32,
    y: i32,
) -> Result<(), String> {
    checked(test_input, environment, &["move", &x.to_string(), &y.to_string()])?;
    checked(test_input, environment, &["button", "left", "pressed"])?;
    checked(test_input, environment, &["button", "left", "released"])?;
    Ok(())
}

fn click_plugin_control(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_surface: &str,
    control_id: &str,
    window_origin: (i32, i32),
) -> Result<(), String> {
    let layouts = checked(test_input, environment, &["layouts"])?;
    let surface = layouts
        .lines()
        .find(|line| line.ends_with(plugin_surface))
        .and_then(|line| line.split('\t').next())
        .ok_or_else(|| format!("{plugin_surface} is absent from layout inventory: {layouts}"))?;
    let layout = checked(test_input, environment, &["layout", surface])?;
    let node = layout
        .lines()
        .find(|line| {
            line.split_whitespace().nth(1).is_some_and(|id| {
                id == control_id || id.strip_suffix(control_id).is_some_and(|prefix| prefix.ends_with('/'))
            })
        })
        .ok_or_else(|| format!("{control_id} is absent from {plugin_surface} layout: {layout}"))?;
    let allocated = node
        .split(" allocated=")
        .nth(1)
        .and_then(|fields| fields.split_whitespace().next())
        .ok_or_else(|| format!("{control_id} has no allocated geometry: {node}"))?;
    let geometry = allocated
        .split(',')
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{control_id} has invalid geometry: {error}"))?;
    let [x, y, width, height] = geometry.as_slice() else {
        return Err(format!("{control_id} has incomplete geometry: {node}"));
    };
    if *width <= 0.0 || *height <= 0.0 {
        return Err(format!("{control_id} has no clickable area: {node}"));
    }
    click_at(
        test_input,
        environment,
        window_origin.0 + (x + width / 2.0).round() as i32,
        window_origin.1 + (y + height / 2.0).round() as i32,
    )
}

fn wait_for_component_dialog_memory(
    test_input: &Path,
    environment: &[(String, String)],
    initial_bytes: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let output = checked(test_input, environment, &["plugins"])?;
        let snapshot: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&output).map_err(|error| error.to_string())?;
        let bytes = snapshot
            .plugins
            .iter()
            .find(|plugin| plugin.id == "org.example.component-window")
            .and_then(|plugin| plugin.memory.native_ui_bytes)
            .ok_or("component window has no native UI memory after dialog click")?;
        if bytes > initial_bytes {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "component dialog did not increase retained UI memory: initial={initial_bytes}, current={bytes}"
            ));
        }
        thread::sleep(POLL);
    }
}

fn verify_sibling_windows(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.example.acceptance-windows";
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (first, second) = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let installed = windows
            .lines()
            .filter(|line| line.contains("\torg.example.acceptance-windows\t"))
            .collect::<Vec<_>>();
        let find = |size: &str| {
            installed.iter().find_map(|line| {
                line.ends_with(size)
                    .then(|| line.split('\t').next()?.parse::<u64>().ok())
                    .flatten()
            })
        };
        if let (Some(first), Some(second)) = (find("360x220"), find("420x240")) {
            break (first, second);
        }
        if Instant::now() >= deadline {
            return Err(format!("sibling plugin windows did not map: {windows}"));
        }
        thread::sleep(POLL);
    };
    let before = wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    checked(
        test_input,
        environment,
        &["semantic", "window", &second.to_string(), "click"],
    )?;
    checked(test_input, environment, &["key", "alt", "pressed"])?;
    checked(test_input, environment, &["key", "f4", "pressed"])?;
    checked(test_input, environment, &["key", "f4", "released"])?;
    checked(test_input, environment, &["key", "alt", "released"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let installed = windows
            .lines()
            .filter(|line| line.contains("\torg.example.acceptance-windows\t"))
            .collect::<Vec<_>>();
        if installed.len() == 1
            && installed[0].starts_with(&format!("{first}\t"))
            && installed[0].ends_with("360x220")
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("closing one window did not preserve its sibling: {windows}"));
        }
        thread::sleep(POLL);
    }
    let status = checked(test_input, environment, &["plugins"])?;
    let status: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&status).map_err(|error| error.to_string())?;
    let plugin = status
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .ok_or("sibling window plugin disappeared after one close")?;
    if !plugin.desired_enabled
        || plugin.health != nickel_session_protocol::PluginRuntimeHealth::Running
        || !plugin
            .memory
            .native_ui_bytes
            .is_some_and(|bytes| bytes > 0 && bytes < before)
    {
        return Err(format!(
            "sibling window plugin lost health or its memory account: before={before}, after={:?}",
            plugin.memory.native_ui_bytes
        ));
    }
    let reduced_bytes = plugin.memory.native_ui_bytes.unwrap();
    let windows = checked(test_input, environment, &["windows"])?;
    let first_line = windows
        .lines()
        .find(|line| line.starts_with(&format!("{first}\t")))
        .ok_or("surviving plugin window disappeared before reopening sibling")?;
    let location = first_line
        .rsplit('\t')
        .next()
        .and_then(|field| field.split_whitespace().next())
        .ok_or("surviving plugin window has no location")?;
    let (x, y) = location
        .split_once(',')
        .ok_or("surviving plugin window has invalid location")?;
    let x: i32 = x.parse().map_err(|_| "invalid plugin window x")?;
    let y: i32 = y.parse().map_err(|_| "invalid plugin window y")?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.acceptance-windows/first",
        "reopen",
        (x, y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let installed = windows
            .lines()
            .filter(|line| line.contains("\torg.example.acceptance-windows\t"))
            .collect::<Vec<_>>();
        if installed.len() == 2
            && installed.iter().any(|line| line.starts_with(&format!("{first}\t")))
            && installed.iter().any(|line| line.ends_with("420x240"))
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin button did not reopen its sibling: {windows}"));
        }
        thread::sleep(POLL);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let bytes = wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(1))?;
        if bytes > reduced_bytes {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "reopened plugin window did not restore memory account: reduced={reduced_bytes}, current={bytes}"
            ));
        }
        thread::sleep(POLL);
    }
    let windows = checked(test_input, environment, &["windows"])?;
    let second_line = windows
        .lines()
        .find(|line| line.contains("\torg.example.acceptance-windows\t") && line.ends_with("420x240"))
        .ok_or("reopened plugin window disappeared before typed hide")?;
    let location = second_line
        .rsplit('\t')
        .next()
        .and_then(|field| field.split_whitespace().next())
        .ok_or("reopened plugin window has no location")?;
    let (x, y) = location
        .split_once(',')
        .ok_or("reopened plugin window has invalid location")?;
    let x: i32 = x.parse().map_err(|_| "invalid reopened plugin window x")?;
    let y: i32 = y.parse().map_err(|_| "invalid reopened plugin window y")?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.acceptance-windows/second",
        "hide",
        (x, y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let installed = windows
            .lines()
            .filter(|line| line.contains("\torg.example.acceptance-windows\t"))
            .collect::<Vec<_>>();
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let plugin = status
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or("sibling window plugin disappeared after typed hide")?;
        if installed.len() == 1
            && installed[0].starts_with(&format!("{first}\t"))
            && plugin.desired_enabled
            && plugin.health == nickel_session_protocol::PluginRuntimeHealth::Running
            && plugin.memory.native_ui_bytes == Some(reduced_bytes)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("typed window hide did not retire only its sibling: {windows}"));
        }
        thread::sleep(POLL);
    }
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    Ok(())
}

fn dialog_surface_line(surfaces: &str) -> Option<&str> {
    surfaces
        .lines()
        .find(|line| line.ends_with("org.example.surface-dialog/confirm"))
}

fn verify_separate_plugin_dialog(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.example.surface-dialog";
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (home_id, home_x, home_y) = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let home = windows.lines().find(|line| {
            line.contains("\torg.example.surface-dialog\tSurface Dialog Example\t")
                && line.ends_with("420x280")
        });
        if let Some(window_id) = home
            .and_then(|line| line.split('\t').next())
            .and_then(|field| field.parse::<u64>().ok())
            && let Some(location) = home
            .and_then(|line| line.rsplit('\t').next())
            .and_then(|field| field.split_whitespace().next())
            && let Some((x, y)) = location.split_once(',')
            && let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>())
        {
            break (window_id, x, y);
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin dialog home window did not map: {windows}"));
        }
        thread::sleep(POLL);
    };
    let home_bytes = wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if dialog_surface_line(&surfaces).is_some() {
        return Err(format!("plugin dialog started open: {surfaces}"));
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.surface-dialog/home",
        "open-dialog",
        (home_x, home_y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (dialog_x, dialog_y) = loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if let Some(location) = dialog_surface_line(&surfaces)
            .and_then(|line| line.split('\t').nth(2))
            .and_then(|field| field.split_whitespace().next())
            && let Some((x, y)) = location.split_once(',')
            && let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>())
        {
            break (x, y);
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin dialog did not map: {surfaces}"));
        }
        thread::sleep(POLL);
    };
    let opened_bytes = wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    if opened_bytes <= home_bytes {
        return Err("plugin dialog did not increase its retained UI memory".into());
    }
    // The dialog is narrower than its owner. Aim inside the owner's button
    // padding, just outside the dialog's left edge.
    let owner_probe_x = dialog_x - 4;
    if owner_probe_x < home_x + 24 || owner_probe_x >= home_x + 420 {
        return Err("dialog leaves no owner button space for the input probe".into());
    }
    click_at(test_input, environment, owner_probe_x, home_y + 119)?;
    thread::sleep(Duration::from_millis(500));
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if dialog_surface_line(&surfaces).is_none()
        || !surfaces.contains("org.example.surface-dialog/home")
    {
        return Err(format!("owned dialog allowed input to close its owner: {surfaces}"));
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.surface-dialog/confirm",
        "dismiss-dialog",
        (dialog_x, dialog_y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if dialog_surface_line(&surfaces).is_none() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin dialog did not dismiss: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let plugin = status
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or("dialog plugin disappeared after dismissal")?;
        if !plugin.desired_enabled
            || plugin.health != nickel_session_protocol::PluginRuntimeHealth::Running
        {
            return Err("dialog plugin stopped after dismissal".into());
        }
        if plugin
            .memory
            .native_ui_bytes
            .is_some_and(|bytes| bytes > 0 && bytes <= home_bytes)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "dialog dismissal retained more than its home memory account: initial={home_bytes}, current={:?}",
                plugin.memory.native_ui_bytes
            ));
        }
        thread::sleep(POLL);
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.surface-dialog/home",
        "open-dialog",
        (home_x, home_y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if dialog_surface_line(&surfaces).is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin dialog did not reopen: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    checked(
        test_input,
        environment,
        &["window", "close", &home_id.to_string()],
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let plugin = status
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or("dialog plugin disappeared after owner close")?;
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if !plugin.desired_enabled
            && plugin.health == nickel_session_protocol::PluginRuntimeHealth::Disabled
            && plugin.memory.native_ui_bytes.is_none()
            && !surfaces.contains("org.example.surface-dialog/")
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("closing dialog owner left plugin surfaces alive: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    Ok(())
}

fn overlay_surface_line(surfaces: &str) -> Option<&str> {
    surfaces
        .lines()
        .find(|line| line.ends_with("org.example.overlay/notice"))
}

fn verify_separate_plugin_overlay(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let id = "org.example.overlay";
    checked(test_input, environment, &["plugin-set", id, "enabled"])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (home_x, home_y) = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let home = windows.lines().find(|line| {
            line.contains("\torg.example.overlay\tOverlay Example\t")
                && line.ends_with("420x250")
        });
        if let Some(location) = home
            .and_then(|line| line.rsplit('\t').next())
            .and_then(|field| field.split_whitespace().next())
            && let Some((x, y)) = location.split_once(',')
            && let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>())
        {
            break (x, y);
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin overlay home window did not map: {windows}"));
        }
        thread::sleep(POLL);
    };
    let home_bytes =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    if overlay_surface_line(&surfaces).is_some() {
        return Err(format!("plugin overlay started open: {surfaces}"));
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.overlay/home",
        "show-overlay",
        (home_x, home_y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (overlay_x, overlay_y) = loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if let Some(location) = overlay_surface_line(&surfaces)
            .and_then(|line| line.split('\t').nth(2))
            .and_then(|field| field.split_whitespace().next())
            && let Some((x, y)) = location.split_once(',')
            && let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.parse::<i32>())
        {
            break (x, y);
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin overlay did not map: {surfaces}"));
        }
        thread::sleep(POLL);
    };
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    let (output_x, output_y, output_width, _) =
        surface_geometry(&surfaces, "Desktop", "org.nickel.desktop/main")
            .ok_or("nested desktop output geometry is unavailable")?;
    let expected = (output_x + output_width as i32 - 300 - 18, output_y + 24);
    if (overlay_x, overlay_y) != expected {
        return Err(format!(
            "anchored plugin overlay is at ({overlay_x}, {overlay_y}), expected {expected:?}: {surfaces}"
        ));
    }
    let opened_bytes =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
    if opened_bytes <= home_bytes {
        return Err("plugin overlay did not increase its retained UI memory".into());
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.overlay/notice",
        "hide-overlay",
        (overlay_x, overlay_y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let status = checked(test_input, environment, &["plugins"])?;
        let status: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&status).map_err(|error| error.to_string())?;
        let plugin = status
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .ok_or("overlay plugin disappeared after dismissal")?;
        if overlay_surface_line(&surfaces).is_none()
            && plugin.desired_enabled
            && plugin.health == nickel_session_protocol::PluginRuntimeHealth::Running
            && plugin.memory.native_ui_bytes == Some(home_bytes)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("plugin overlay did not dismiss: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
    Ok(())
}

fn verify_settings_memory_report(
    settings: &Path,
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let mut process = Command::new(settings)
        .args(["--screen", "plugins"])
        .envs(environment.iter().cloned())
        .spawn()
        .map_err(|error| format!("could not launch nested Settings: {error}"))?;
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = process.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("nested Settings exited before reporting memory: {status}"));
            }
            let output = checked(test_input, environment, &["plugins"])?;
            let snapshot: nickel_session_protocol::PluginStatusSnapshot =
                serde_json::from_str(&output).map_err(|error| error.to_string())?;
            if snapshot.plugins.iter().any(|plugin| {
                plugin.id == "org.nickel.settings"
                    && plugin.memory.native_ui_bytes.is_some_and(|bytes| bytes > 0)
            }) {
                break;
            }
            if Instant::now() >= deadline {
                return Err("nested Settings did not publish plugin UI memory".into());
            }
            thread::sleep(POLL);
        }
        let disabled = checked(
            test_input,
            environment,
            &["plugin-set", "org.nickel.settings", "disabled"],
        )?;
        let disabled: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&disabled).map_err(|error| error.to_string())?;
        if disabled.plugins.iter().all(|plugin| {
            plugin.id != "org.nickel.settings" || plugin.desired_enabled
        }) {
            return Err("shell did not disable the Settings plugin".into());
        }
        wait_for_settings_memory(test_input, environment, false, Duration::from_secs(5))?;
        checked(
            test_input,
            environment,
            &["plugin-set", "org.nickel.settings", "enabled"],
        )?;
        wait_for_settings_memory(test_input, environment, true, Duration::from_secs(5))?;
        Ok(())
    })();
    let _ = process.kill();
    let _ = process.wait();
    result?;
    wait_for_settings_memory_expiry(test_input, environment, Duration::from_secs(7))
}

fn wait_for_settings_memory_expiry(
    test_input: &Path,
    environment: &[(String, String)],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let output = checked(test_input, environment, &["plugins"])?;
        let snapshot: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&output).map_err(|error| error.to_string())?;
        if snapshot.plugins.iter().any(|plugin| {
            plugin.id == "org.nickel.settings"
                && plugin.desired_enabled
                && plugin.memory.native_ui_bytes.is_none()
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Settings plugin memory did not expire after process exit".into());
        }
        thread::sleep(POLL);
    }
}

fn wait_for_settings_memory(
    test_input: &Path,
    environment: &[(String, String)],
    enabled: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let output = checked(test_input, environment, &["plugins"])?;
        let snapshot: nickel_session_protocol::PluginStatusSnapshot =
            serde_json::from_str(&output).map_err(|error| error.to_string())?;
        let plugin = snapshot
            .plugins
            .iter()
            .find(|plugin| plugin.id == "org.nickel.settings")
            .ok_or("Settings disappeared from the shell registry")?;
        if plugin.desired_enabled == enabled
            && plugin.memory.native_ui_bytes.is_some_and(|bytes| bytes > 0) == enabled
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Settings did not follow shell activation: desired={}, memory={:?}",
                plugin.desired_enabled, plugin.memory.native_ui_bytes
            ));
        }
        thread::sleep(POLL);
    }
}

fn wait_for_plugin_panel_on_output(
    test_input: &Path,
    environment: &[(String, String)],
    output: &str,
    expected: bool,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        let present = surfaces.lines().any(|line| {
            line.starts_with(&format!("PluginSurface\t{output}\t"))
                && line.contains("360x96\t")
                && line.ends_with("org.example.acceptance-panel/main")
        });
        if present == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "plugin panel presence on {output} stayed {present}, expected {expected}: {surfaces}"
            ));
        }
        thread::sleep(POLL);
    }
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
            .find(|line| line.starts_with("Launcher\t"));
        if launcher.is_some_and(launcher_line_visible) == expected_visible {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "launcher did not become {} before deadline: {launcher:?}",
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

fn launcher_line_visible(line: &str) -> bool {
    line.split('\t').nth(2) != Some("hidden")
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
            let surfaces = checked(test_input, environment, &["surfaces"])?;
            return Err(format!(
                "plugin {id} did not report rendered UI memory: health={:?}, native={:?}, peak={:?}; surfaces={surfaces}",
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

fn verify_layout_snapshot(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_surface: &str,
) -> Result<(), String> {
    let layouts = checked(test_input, environment, &["layouts"])?;
    let surface = layouts
        .lines()
        .find(|line| line.ends_with(plugin_surface))
        .and_then(|line| line.split('\t').next())
        .ok_or_else(|| format!("{plugin_surface} is absent from layout inventory: {layouts}"))?;
    let layout = checked(test_input, environment, &["layout", surface])?;
    if !layout.lines().any(|line| {
        line.contains(" source=") && line.contains(" allocated=") && line.contains(" children=")
    }) {
        return Err(format!(
            "{plugin_surface} has no computed component layout: {layout}"
        ));
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
            "missing {}; build nickel-nested, nickel-test-input, nickel-settings, and nickel-nested-acceptance together",
            name
        )
    })
}
