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
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let screenshot_only = match arguments.as_slice() {
        [] => false,
        [mode] if mode == "--screenshot-only" => true,
        _ => return Err("usage: nickel-nested-acceptance [--screenshot-only]".into()),
    };
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
    let windows = runtime.join("config/nickel/plugins/org.example.acceptance-windows");
    fs::create_dir_all(&windows).map_err(|error| error.to_string())?;
    fs::write(
        windows.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.acceptance-windows","name":"Acceptance Windows","entry":"main.js","surfaces":[{"id":"first","kind":"window","width":400,"height":240},{"id":"second","kind":"window","width":450,"height":260}]}"#,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        windows.join("main.js"),
        "function App() { const [compact, setCompact] = useState(false); return nickel.data.surface.id === 'first' ? h(Window, {width: compact ? 300 : 360, height: 220}, h(Column, {}, h(Button, {id: 'reopen', onClick: () => nickel.request({type: 'show-plugin-surface', surfaceId: 'second'})}, 'Reopen second'), h(Button, {id: 'focus', onClick: () => nickel.request({type: 'surface.focus', id: 'second'})}, 'Focus second'), h(Button, {id: 'move', onClick: () => nickel.request({type: 'surface.setPlacement', surfaceId: 'first', anchor: 'top-left', offsetX: 24, offsetY: 24})}, 'Move first'), h(Button, {id: 'resize', onClick: () => setCompact(true)}, 'Resize first'))) : h(Window, {width: 420, height: 240}, h(Button, {id: 'hide', onClick: () => nickel.request({type: 'hide-plugin-surface', surfaceId: 'second'})}, 'Hide second')); }",
    )
    .map_err(|error| error.to_string())?;
    let dialog = runtime.join("config/nickel/plugins/org.example.surface-dialog");
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

    let result = exercise(
        &mut compositor,
        &test_input,
        &capability_file,
        screenshot_only,
    );
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
    if screenshot_only {
        println!(
            "PASS: native screenshot capture, selection, cancellation, reopen, component socket layouts, and clean shutdown"
        );
    } else {
        println!(
            "PASS: shared default shell, optional JSX Settings, scoped Meta input, installed sibling/dialog/overlay lifecycle, native screenshot, component socket layouts, and clean shutdown"
        );
    }
    Ok(())
}

fn exercise(
    compositor: &mut Child,
    test_input: &Path,
    capability_file: &Path,
    screenshot_only: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    let environment = loop {
        if let Some(status) = compositor.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "nested compositor exited before readiness: {status}"
            ));
        }
        if let Ok(environment) = read_environment(capability_file)
            && let Ok(readiness) = checked(test_input, &environment, &["readiness"])
            && readiness.contains("output_roles_ready=true")
        {
            if !readiness.contains("expected_pid=None authenticated_pid=None") {
                return Err(format!("unexpected shell process authority: {readiness}"));
            }
            break environment;
        }
        if Instant::now() >= deadline {
            return Err("nested shell readiness timed out".into());
        }
        thread::sleep(POLL);
    };
    assert_no_shell_child(compositor.id())?;
    let surfaces = checked(test_input, &environment, &["surfaces"])?;
    for role in ["Desktop", "Lock"] {
        if !surfaces
            .lines()
            .any(|line| line.starts_with(&format!("{role}\t")))
        {
            return Err(format!("native {role} is absent: {surfaces}"));
        }
    }
    verify_native_layout_snapshot(test_input, &environment, "Desktop")?;
    if screenshot_only {
        // Exercise capture after the nested backend's three-second startup frame pump ends.
        // A startup repaint must not conceal a missing redraw wakeup on an idle compositor.
        thread::sleep(Duration::from_millis(3500));
        return verify_native_screenshot_lifecycle(test_input, &environment);
    }
    assert_default_package(test_input, &environment, true)?;
    verify_layout_snapshot(test_input, &environment, "nickel-default/taskbar")?;
    // Meta is sent only through the explicitly enabled nested socket.
    checked(test_input, &environment, &["key", "meta", "pressed"])?;
    checked(test_input, &environment, &["key", "meta", "released"])?;
    let launcher = wait_for_plugin_surface(
        test_input,
        &environment,
        "nickel-default/launcher",
        Duration::from_secs(5),
    )?;
    verify_layout_snapshot(test_input, &environment, "nickel-default/launcher")?;
    // The nested window is visible on the host, so incidental host keyboard
    // input can reach the focused launcher while the harness is running. A
    // non-empty query intentionally replaces the dashboard footer. Clear that
    // state through the launcher's production Escape handler before requiring
    // the Settings affordance.
    if plugin_control_geometry(
        test_input,
        &environment,
        "nickel-default/launcher",
        "launcher-settings",
    )
    .is_err()
    {
        checked(test_input, &environment, &["key", "escape", "pressed"])?;
        checked(test_input, &environment, &["key", "escape", "released"])?;
        wait_for_plugin_surface(
            test_input,
            &environment,
            "nickel-default/launcher",
            Duration::from_secs(5),
        )?;
    }
    // The visible nested backend can interleave physical host pointer events
    // with injected input. Showing an already shown singleton is idempotent,
    // so retry the real launcher control until its window reaches the native
    // registry instead of treating one stolen click as a product failure.
    for _ in 0..3 {
        click_plugin_control(
            test_input,
            &environment,
            "nickel-default/launcher",
            "launcher-settings",
            launcher,
        )?;
        let windows = checked(test_input, &environment, &["windows"])?;
        if windows
            .lines()
            .any(|line| line.contains("\tnickel-default\tNickel Settings\t"))
        {
            break;
        }
    }
    let settings = wait_for_plugin_window(
        test_input,
        &environment,
        "nickel-default",
        "Nickel Settings",
        Duration::from_secs(5),
    )?;
    // The launcher is an overlay, so dismiss it before interacting with the
    // ordinary Settings window below it. Settings must remain independently
    // mapped after its launcher surface loses visibility.
    let dismiss_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let surfaces = checked(test_input, &environment, &["surfaces"])?;
        if plugin_surface_origin(&surfaces, "nickel-default/launcher").is_none() {
            break;
        }
        checked(test_input, &environment, &["key", "meta", "pressed"])?;
        checked(test_input, &environment, &["key", "meta", "released"])?;
        if Instant::now() >= dismiss_deadline {
            return Err(format!("launcher remained mapped above Settings: {surfaces}"));
        }
        thread::sleep(POLL);
    }
    verify_layout_snapshot(test_input, &environment, "nickel-default/settings")?;
    let settings_surface_origin = wait_for_plugin_surface(
        test_input,
        &environment,
        "nickel-default/settings",
        Duration::from_secs(5),
    )?;
    for _ in 0..3 {
        click_plugin_control(
            test_input,
            &environment,
            "nickel-default/settings",
            "settings-navigation/destination/nickel-default/appearance",
            settings_surface_origin,
        )?;
        if plugin_control_geometry(
            test_input,
            &environment,
            "nickel-default/settings",
            "appearance-hue",
        )
        .is_ok()
        {
            break;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    for id in [
        "appearance-hue",
        "appearance-intensity",
        "appearance-accent-custom",
    ] {
        let [_, _, width, height] = loop {
            match plugin_control_geometry(test_input, &environment, "nickel-default/settings", id)
            {
                Ok(geometry) => break geometry,
                Err(_) if Instant::now() < deadline => thread::sleep(POLL),
                Err(error) => return Err(error),
            }
        };
        if width <= 0.0 || height <= 0.0 {
            return Err(format!("required Appearance control {id} has no layout"));
        }
    }
    verify_managed_settings_resize(test_input, &environment, settings.0)?;
    for id in [
        "appearance-hue",
        "appearance-intensity",
        "appearance-accent-custom",
    ] {
        let [_, _, width, height] =
            plugin_control_geometry(test_input, &environment, "nickel-default/settings", id)?;
        if width <= 0.0 || height <= 0.0 {
            return Err(format!(
                "Appearance control {id} disappeared after the Settings resize"
            ));
        }
    }
    checked(
        test_input,
        &environment,
        &["window", "close", &settings.0.to_string()],
    )?;
    wait_for_window_absent(test_input, &environment, settings.0)?;
    // Closing optional windows must keep the package and taskbar alive.
    assert_default_package(test_input, &environment, true)?;
    verify_layout_snapshot(test_input, &environment, "nickel-default/taskbar")?;
    checked(
        test_input,
        &environment,
        &["plugin-set", "nickel-default", "disabled"],
    )?;
    assert_default_package(test_input, &environment, false)?;
    let layouts = checked(test_input, &environment, &["layouts"])?;
    if layouts.lines().any(|line| line.contains("nickel-default/")) {
        return Err(format!(
            "disabled shell retained native component surfaces: {layouts}"
        ));
    }
    checked(
        test_input,
        &environment,
        &["plugin-set", "nickel-default", "enabled"],
    )?;
    assert_default_package(test_input, &environment, true)?;
    verify_layout_snapshot(test_input, &environment, "nickel-default/taskbar")?;
    verify_sibling_windows(test_input, &environment)?;
    verify_separate_plugin_dialog(test_input, &environment)?;
    verify_separate_plugin_overlay(test_input, &environment)?;
    verify_native_screenshot_lifecycle(test_input, &environment)?;
    Ok(())
}

fn window_geometry(windows: &str, id: u64) -> Option<(i32, i32, u32, u32)> {
    let geometry = windows
        .lines()
        .find(|line| line.starts_with(&format!("{id}\t")))?
        .rsplit('\t')
        .next()?;
    let mut fields = geometry.split_whitespace();
    let (x, y) = fields.next()?.split_once(',')?;
    let (width, height) = fields.next()?.split_once('x')?;
    Some((
        x.parse().ok()?,
        y.parse().ok()?,
        width.parse().ok()?,
        height.parse().ok()?,
    ))
}

fn verify_managed_settings_resize(
    test_input: &Path,
    environment: &[(String, String)],
    window: u64,
) -> Result<(i32, i32, u32, u32), String> {
    let windows = checked(test_input, environment, &["windows"])?;
    let before = window_geometry(&windows, window)
        .ok_or_else(|| format!("Settings geometry is absent before resize: {windows}"))?;
    let corner = (
        before.0 + i32::try_from(before.2).unwrap_or(i32::MAX) - 2,
        before.1 + i32::try_from(before.3).unwrap_or(i32::MAX) - 2,
    );
    checked(
        test_input,
        environment,
        &["move", &corner.0.to_string(), &corner.1.to_string()],
    )?;
    thread::sleep(POLL);
    checked(test_input, environment, &["button", "left", "pressed"])?;
    thread::sleep(POLL);
    checked(
        test_input,
        environment,
        &[
            "move",
            &(corner.0 - 120).to_string(),
            &(corner.1 - 80).to_string(),
        ],
    )?;
    thread::sleep(POLL);
    checked(test_input, environment, &["button", "left", "released"])?;
    thread::sleep(POLL);

    let deadline = Instant::now() + Duration::from_secs(5);
    let resized = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if let Some(current) = window_geometry(&windows, window)
            && (current.2, current.3) != (before.2, before.3)
        {
            break current;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Settings frame did not resize through production input: {windows}"
            ));
        }
        thread::sleep(POLL);
    };
    let [_, _, root_width, root_height] =
        plugin_control_geometry(test_input, environment, "nickel-default/settings", "settings")?;
    if (root_width - resized.2 as f32).abs() > 1.0
        || (root_height - resized.3 as f32).abs() > 1.0
    {
        return Err(format!(
            "Settings content did not follow its resized native viewport: native={:?}, content={root_width}x{root_height}",
            (resized.2, resized.3)
        ));
    }

    // Ordinary pointer movement may repaint hover state, but it must neither
    // restore the JSX default geometry nor rotate away the current callbacks.
    checked(
        test_input,
        environment,
        &[
            "move",
            &(resized.0 + 24).to_string(),
            &(resized.1 + 64).to_string(),
        ],
    )?;
    thread::sleep(POLL);
    let after_motion = checked(test_input, environment, &["windows"])?;
    if window_geometry(&after_motion, window) != Some(resized) {
        return Err(format!(
            "pointer motion reset the user-owned Settings geometry: before={resized:?}; windows={after_motion}"
        ));
    }
    Ok(resized)
}

fn assert_default_package(
    test_input: &Path,
    environment: &[(String, String)],
    enabled: bool,
) -> Result<(), String> {
    let output = checked(test_input, environment, &["plugins"])?;
    let snapshot: nickel_session_protocol::PluginStatusSnapshot =
        serde_json::from_str(&output).map_err(|error| error.to_string())?;
    let package = snapshot
        .plugins
        .iter()
        .find(|package| package.id == "nickel-default")
        .ok_or("default shell package missing")?;
    let expected = if enabled {
        nickel_session_protocol::PluginRuntimeHealth::Running
    } else {
        nickel_session_protocol::PluginRuntimeHealth::Disabled
    };
    if package.desired_enabled != enabled || package.health != expected {
        return Err(format!("default shell lifecycle differs: {package:?}"));
    }
    Ok(())
}

fn wait_for_window_absent(
    test_input: &Path,
    environment: &[(String, String)],
    id: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if !windows
            .lines()
            .any(|line| line.split('\t').next() == Some(id.to_string().as_str()))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("closed window {id} remains registered: {windows}"));
        }
        thread::sleep(POLL);
    }
}

fn native_surface_geometry(surfaces: &str, role: &str) -> Option<(i32, i32, u32, u32)> {
    let geometry = surfaces
        .lines()
        .find(|line| line.starts_with(&format!("{role}\t")) && line.split('\t').count() == 3)?
        .split('\t')
        .nth(2)?;
    let (origin, size) = geometry.split_once(' ')?;
    let (x, y) = origin.split_once(',')?;
    let (width, height) = size.split_once('x')?;
    Some((
        x.parse().ok()?,
        y.parse().ok()?,
        width.parse().ok()?,
        height.parse().ok()?,
    ))
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
            line.starts_with("Screenshot\t")
                && line.split('\t').count() == 3
                && line.split('\t').nth(2) != Some("hidden")
        });
        if visible == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "screenshot visibility did not become {expected}: {surfaces}"
            ));
        }
        thread::sleep(POLL);
    }
}

fn verify_native_screenshot_lifecycle(
    test_input: &Path,
    environment: &[(String, String)],
) -> Result<(), String> {
    let press_print_screen = || -> Result<(), String> {
        checked(test_input, environment, &["key", "print-screen", "pressed"])?;
        checked(
            test_input,
            environment,
            &["key", "print-screen", "released"],
        )?;
        Ok(())
    };
    press_print_screen()?;
    wait_for_screenshot_visibility(test_input, environment, true, Duration::from_secs(5))?;
    verify_native_layout_snapshot(test_input, environment, "Screenshot")?;
    let surfaces = checked(test_input, environment, &["surfaces"])?;
    let (x, y, width, height) = native_surface_geometry(&surfaces, "Screenshot")
        .ok_or_else(|| format!("native screenshot geometry is missing: {surfaces}"))?;
    let start = (x + width as i32 / 4, y + height as i32 / 4);
    let end = (x + width as i32 * 3 / 4, y + height as i32 * 3 / 4);
    checked(
        test_input,
        environment,
        &["move", &start.0.to_string(), &start.1.to_string()],
    )?;
    checked(test_input, environment, &["button", "left", "pressed"])?;
    checked(
        test_input,
        environment,
        &["move", &end.0.to_string(), &end.1.to_string()],
    )?;
    checked(test_input, environment, &["button", "left", "released"])?;
    let center = (x + width as i32 / 2, y + height as i32 / 2);
    click_at(test_input, environment, center.0, center.1)?;
    click_at(test_input, environment, center.0, center.1)?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_screenshot_visibility(test_input, environment, false, Duration::from_secs(2))?;

    press_print_screen()?;
    wait_for_screenshot_visibility(test_input, environment, true, Duration::from_secs(5))?;
    checked(test_input, environment, &["key", "escape", "pressed"])?;
    checked(test_input, environment, &["key", "escape", "released"])?;
    wait_for_screenshot_visibility(test_input, environment, false, Duration::from_secs(2))?;
    Ok(())
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
            let readiness = checked(test_input, environment, &["readiness"])
                .unwrap_or_else(|error| format!("<unavailable: {error}>"));
            let layouts = checked(test_input, environment, &["layouts"])
                .unwrap_or_else(|error| format!("<unavailable: {error}>"));
            let surfaces = checked(test_input, environment, &["surfaces"])
                .unwrap_or_else(|error| format!("<unavailable: {error}>"));
            let plugins = checked(test_input, environment, &["plugins"])
                .unwrap_or_else(|error| format!("<unavailable: {error}>"));
            return Err(format!(
                "plugin window {plugin_id} did not enter the window registry:\nwindows:\n{windows}\nreadiness:\n{readiness}\nlayouts:\n{layouts}\nsurfaces:\n{surfaces}\nplugins:\n{plugins}"
            ));
        }
        thread::sleep(POLL);
    }
}

fn plugin_surface_origin(surfaces: &str, plugin_surface: &str) -> Option<(i32, i32)> {
    let geometry = surfaces
        .lines()
        .find(|line| line.ends_with(plugin_surface))?
        .split('\t')
        .nth(2)?;
    let origin = geometry.split_whitespace().next()?;
    let (x, y) = origin.split_once(',')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

fn wait_for_plugin_surface(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_surface: &str,
    timeout: Duration,
) -> Result<(i32, i32), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let surfaces = checked(test_input, environment, &["surfaces"])?;
        if let Some(origin) = plugin_surface_origin(&surfaces, plugin_surface) {
            return Ok(origin);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "plugin surface {plugin_surface} did not map: {surfaces}"
            ));
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
    checked(
        test_input,
        environment,
        &["move", &x.to_string(), &y.to_string()],
    )?;
    thread::sleep(POLL);
    checked(test_input, environment, &["button", "left", "pressed"])?;
    thread::sleep(POLL);
    // The visible nested backend also receives the host's physical mouse.
    // Reassert the synthetic pointer target so unrelated host motion cannot
    // split a press/release pair across different controls.
    checked(
        test_input,
        environment,
        &["move", &x.to_string(), &y.to_string()],
    )?;
    thread::sleep(POLL);
    checked(test_input, environment, &["button", "left", "released"])?;
    thread::sleep(POLL);
    Ok(())
}

fn click_plugin_control(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_surface: &str,
    control_id: &str,
    window_origin: (i32, i32),
) -> Result<(), String> {
    let [x, y, width, height] =
        plugin_control_geometry(test_input, environment, plugin_surface, control_id)?;
    if width <= 0.0 || height <= 0.0 {
        return Err(format!("{control_id} has no clickable area"));
    }
    click_at(
        test_input,
        environment,
        window_origin.0 + (x + width / 2.0).round() as i32,
        window_origin.1 + (y + height / 2.0).round() as i32,
    )
}

fn plugin_control_geometry(
    test_input: &Path,
    environment: &[(String, String)],
    plugin_surface: &str,
    control_id: &str,
) -> Result<[f32; 4], String> {
    let layouts = checked(test_input, environment, &["layouts"])?;
    let surface = layouts
        .lines()
        .find(|line| line.ends_with(plugin_surface))
        .and_then(|line| line.split('\t').next())
        .ok_or_else(|| format!("{plugin_surface} is absent from layout inventory: {layouts}"))?;
    let nodes = layout_nodes(test_input, environment, surface)?;
    let node = nodes
        .iter()
        .find(|line| {
            line.split_whitespace().nth(1).is_some_and(|id| {
                id == control_id
                    || id
                        .strip_suffix(control_id)
                        .is_some_and(|prefix| prefix.ends_with('/'))
            })
        })
        .ok_or_else(|| format!("{control_id} is absent from {plugin_surface} layout: {nodes:?}"))?;
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
    Ok([*x, *y, *width, *height])
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
    let before =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
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
            return Err(format!(
                "closing one window did not preserve its sibling: {windows}"
            ));
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
        "move",
        (x, y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (x, y) = loop {
        let windows = checked(test_input, environment, &["windows"])?;
        let moved = windows
            .lines()
            .find(|line| line.starts_with(&format!("{first}\t")))
            .and_then(|line| line.rsplit('\t').next())
            .and_then(|field| field.split_whitespace().next())
            .and_then(|location| location.split_once(','))
            .and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?)));
        if let Some(moved) = moved
            && moved != (x, y)
        {
            break moved;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "typed plugin placement did not move its window: {windows}"
            ));
        }
        thread::sleep(POLL);
    };
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
            && installed
                .iter()
                .any(|line| line.starts_with(&format!("{first}\t")))
            && installed.iter().any(|line| line.ends_with("420x240"))
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "plugin button did not reopen its sibling: {windows}"
            ));
        }
        thread::sleep(POLL);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let reopened_bytes = loop {
        let bytes =
            wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(1))?;
        if bytes > reduced_bytes {
            break bytes;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "reopened plugin window did not restore memory account: reduced={reduced_bytes}, current={bytes}"
            ));
        }
        thread::sleep(POLL);
    };
    let windows = checked(test_input, environment, &["windows"])?;
    let second_line = windows
        .lines()
        .find(|line| {
            line.contains("\torg.example.acceptance-windows\t") && line.ends_with("420x240")
        })
        .ok_or("reopened plugin window disappeared before typed hide")?;
    let second_id = second_line
        .split('\t')
        .next()
        .ok_or("reopened plugin window has no ID")?;
    let focus_bounds = plugin_control_geometry(
        test_input,
        environment,
        "org.example.acceptance-windows/first",
        "focus",
    )?;
    if focus_bounds[0] < 0.0
        || focus_bounds[1] < 0.0
        || focus_bounds[0] + focus_bounds[2] > 360.0
        || focus_bounds[1] + focus_bounds[3] > 220.0
    {
        return Err(format!(
            "surface.focus control is outside its window: {focus_bounds:?}"
        ));
    }
    click_plugin_control(
        test_input,
        environment,
        "org.example.acceptance-windows/first",
        "focus",
        (x, y),
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if windows.lines().any(|line| {
            line.starts_with(&format!("{second_id}\t"))
                && line.split('\t').any(|field| field == "active")
        }) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "surface.focus did not activate the owned window: {windows}; control={focus_bounds:?}"
            ));
        }
        thread::sleep(POLL);
    }
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
            && plugin
                .memory
                .native_ui_bytes
                .is_some_and(|bytes| bytes >= reduced_bytes && bytes < reopened_bytes)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "typed window hide did not retire only its sibling: windows={windows}; surviving_native_ui_floor={reduced_bytes}, pre_hide_native_ui_bytes={reopened_bytes}, actual_native_ui_bytes={:?}",
                plugin.memory.native_ui_bytes
            ));
        }
        thread::sleep(POLL);
    }
    let windows = checked(test_input, environment, &["windows"])?;
    let location = windows
        .lines()
        .find(|line| line.starts_with(&format!("{first}\t")))
        .and_then(|line| line.rsplit('\t').next())
        .and_then(|field| field.split_whitespace().next())
        .and_then(|location| location.split_once(','))
        .and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?)))
        .ok_or("surviving plugin window has no location before JSX resize")?;
    click_plugin_control(
        test_input,
        environment,
        "org.example.acceptance-windows/first",
        "resize",
        location,
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = checked(test_input, environment, &["windows"])?;
        if windows
            .lines()
            .any(|line| line.starts_with(&format!("{first}\t")) && line.ends_with("300x220"))
        {
            break;
        }
        if Instant::now() >= deadline {
            let surfaces = checked(test_input, environment, &["surfaces"])?;
            let layouts = checked(test_input, environment, &["layouts"])?;
            return Err(format!(
                "JSX root resize did not reach its native window: windows={windows}; surfaces={surfaces}; layouts={layouts}"
            ));
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
    let home_bytes =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
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
    let opened_bytes =
        wait_for_plugin_native_memory(test_input, environment, id, Duration::from_secs(5))?;
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
        return Err(format!(
            "owned dialog allowed input to close its owner: {surfaces}"
        ));
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
        if plugin.desired_enabled
            && plugin.health == nickel_session_protocol::PluginRuntimeHealth::Running
            && !surfaces.contains("org.example.surface-dialog/")
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "closing dialog owner left plugin surfaces alive: {surfaces}"
            ));
        }
        thread::sleep(POLL);
    }
    checked(test_input, environment, &["plugin-set", id, "disabled"])?;
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
            line.contains("\torg.example.overlay\tOverlay Example\t") && line.ends_with("420x250")
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
    let (output_x, output_y, output_width, _) = native_surface_geometry(&surfaces, "Desktop")
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
        if let Some(bytes) = plugin.memory.native_ui_bytes
            && bytes > 0
                && plugin
                    .memory
                    .tracked_peak_bytes
                    .is_some_and(|peak| peak >= bytes)
            {
                return Ok(bytes);
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
    let nodes = layout_nodes(test_input, environment, surface)?;
    if !nodes.iter().any(|line| {
        line.contains(" source=") && line.contains(" allocated=") && line.contains(" children=")
    }) {
        return Err(format!(
            "{plugin_surface} has no computed component layout: {nodes:?}"
        ));
    }
    Ok(())
}

fn verify_native_layout_snapshot(
    test_input: &Path,
    environment: &[(String, String)],
    role: &str,
) -> Result<(), String> {
    let layouts = checked(test_input, environment, &["layouts"])?;
    let surface = layouts
        .lines()
        .find(|line| line.split('\t').nth(1) == Some(role) && line.split('\t').count() == 5)
        .and_then(|line| line.split('\t').next())
        .ok_or_else(|| format!("native {role} is absent from layout inventory: {layouts}"))?;
    let nodes = layout_nodes(test_input, environment, surface)?;
    if nodes.is_empty()
        || nodes.iter().any(|line| {
            !line.contains(" source=")
                || !line.contains(" allocated=")
                || !line.contains(" children=")
        })
    {
        return Err(format!(
            "native {role} has no computed component layout: {nodes:?}"
        ));
    }
    Ok(())
}

fn layout_nodes(
    test_input: &Path,
    environment: &[(String, String)],
    surface: &str,
) -> Result<Vec<String>, String> {
    let mut offset = 0;
    let mut total = None;
    let mut all_nodes = Vec::new();
    loop {
        let layout = checked(
            test_input,
            environment,
            &["layout", surface, &offset.to_string()],
        )?;
        let header = layout.lines().next().ok_or("layout page has no header")?;
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 5 || fields[0] != "#" || fields[1] != "layout" {
            return Err(format!(
                "{surface} layout page has an invalid header: {header}"
            ));
        }
        let page_offset = fields[2]
            .strip_prefix("offset=")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or("layout page has no valid offset")?;
        let page_total = fields[3]
            .strip_prefix("total=")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or("layout page has no valid node count")?;
        if page_offset != offset || total.is_some_and(|total| total != page_total) {
            return Err(format!("{surface} layout pages changed during inspection"));
        }
        total = Some(page_total);
        let nodes = layout.lines().skip(1).collect::<Vec<_>>();
        if nodes.is_empty() {
            return Err(format!("{surface} has an empty layout page: {layout}"));
        }
        let next = fields[4]
            .strip_prefix("next=")
            .ok_or("layout page has no next offset")?;
        all_nodes.extend(nodes.iter().map(|line| (*line).to_owned()));
        if next == "end" {
            if all_nodes.len() != page_total {
                return Err(format!(
                    "{surface} layout ends before all nodes were returned"
                ));
            }
            break;
        }
        let next = next
            .parse::<usize>()
            .map_err(|_| "invalid next layout offset")?;
        if next != offset + nodes.len() || next > page_total {
            return Err(format!("{surface} layout page skipped nodes"));
        }
        offset = next;
    }
    Ok(all_nodes)
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
