use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::platform::NotificationSource;
use image::{Rgba, RgbaImage};
use nickel_input::KeyCode;
use nickel_session_protocol::{
    PointerInteraction, PreviewTargetAction, ScreenshotTargetAction, ShellRole,
    ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::{
    ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
    HostTelemetry, InputModality, Point, Rect, SemanticAction, SemanticRole, SemanticSelector,
    SemanticValueInput, SemanticValueSnapshot, Shortcut, UiEvent, UiHost, ViewContext,
};
use nickel_ui_testkit::{Scenario, Selector};

use super::{
    ControlAction, HostRuntimeSamples, LiveShell, desktop_label_foreground, initial_wallpaper,
    panel_status_layout, panel_tray_icons,
    platform::{
        AudioStatus, BluetoothStatus, FeedState, FeedStatus, GlobalShortcut, NetworkStatus,
        SecureStorageState, SystemStatusUpdate,
    },
    preview_refresh_due, retain_unchanged_desktop_icons, secure_storage_status_label,
    session_feed_status_label, shortcut_capability_status, visible_tray_item,
    window_belongs_to_panel,
};

#[test]
fn safe_mode_suppresses_installed_autostart_without_discarding_saved_choice() {
    assert!(super::should_auto_start_installed_plugin(true, false));
    assert!(!super::should_auto_start_installed_plugin(true, true));
    assert!(!super::should_auto_start_installed_plugin(false, false));
}

#[test]
fn settings_status_is_idle_until_the_separate_process_reports_memory() {
    let shell = LiveShell::new().unwrap();
    let snapshot = shell.plugin_status_snapshot();
    let settings = snapshot
        .plugins
        .iter()
        .find(|plugin| plugin.id == crate::settings_plugin_report::ID)
        .unwrap();
    assert!(settings.desired_enabled);
    assert_eq!(
        settings.health,
        nickel_session_protocol::PluginRuntimeHealth::Idle
    );
    assert!(settings.memory.native_ui_bytes.is_none());
    let reported_at = Instant::now();
    let report = crate::settings_plugin_report::SettingsPluginReport::new(
        true,
        nickel_session_protocol::PluginMemorySnapshot {
            native_ui_bytes: Some(4096),
            ..Default::default()
        },
        reported_at,
    )
    .unwrap();
    let mut running = snapshot.clone();
    report.append_to(&mut running, reported_at);
    let settings = running
        .plugins
        .iter()
        .find(|plugin| plugin.id == crate::settings_plugin_report::ID)
        .unwrap();
    assert_eq!(
        settings.health,
        nickel_session_protocol::PluginRuntimeHealth::Running
    );
    assert_eq!(settings.memory.native_ui_bytes, Some(4096));
    let mut closed = snapshot;
    report.append_to(&mut closed, reported_at + Duration::from_secs(6));
    let settings = closed
        .plugins
        .iter()
        .find(|plugin| plugin.id == crate::settings_plugin_report::ID)
        .unwrap();
    assert_eq!(
        settings.health,
        nickel_session_protocol::PluginRuntimeHealth::Idle
    );
    assert!(settings.memory.native_ui_bytes.is_none());
}

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");

#[test]
fn declared_extension_is_reviewable_but_needs_an_installed_package() {
    let mut shell = LiveShell::new().unwrap();
    let manifest = nickel_core::plugins::PluginManifest::from_json(
        r#"{
        "api_version": 1,
        "id": "org.example.badge",
        "name": "Mail badge",
        "entry": "main.js",
        "capabilities": [],
        "contributes": [{
            "target_plugin": "org.nickel.taskbar",
            "target_slot": "task-badge",
            "contract": "badge",
            "mode": "replace"
        }]
    }"#,
    )
    .unwrap();
    let id = manifest.id.clone();
    shell.plugin_registry.register(manifest).unwrap();
    let status = shell.plugin_status_snapshot();
    let extension = status
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .unwrap();
    assert_eq!(
        extension.composition,
        ["replace org.nickel.taskbar/task-badge (badge)"]
    );
    assert!(
        shell
            .set_plugin_enabled(&id, true)
            .unwrap_err()
            .contains("not an installed package")
    );
    assert!(!shell.plugin_registry.get(&id).unwrap().desired_enabled);
}

#[test]
fn installed_badge_extension_composes_into_taskbar_and_retires_on_disable() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.mail-badge");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{
        "api_version":1,"id":"org.example.mail-badge","name":"Mail badge",
        "entry":"main.js","contributes":[{
            "target_plugin":"org.nickel.taskbar","target_slot":"task-badge",
            "contract":"badge","mode":"add","priority":5
        }]
    }"#,
    )
    .unwrap();
    std::fs::write(directory.join("main.js"),
        "function App() { return h(Badge, {item: 'org.example.mail', label: 'Unread mail', count: 7}); }"
    ).unwrap();
    let earlier = root.path().join("org.example.priority-badge");
    std::fs::create_dir(&earlier).unwrap();
    std::fs::write(
        earlier.join("plugin.json"),
        r#"{
        "api_version":1,"id":"org.example.priority-badge","name":"Priority badge",
        "entry":"main.js","contributes":[{
            "target_plugin":"org.nickel.taskbar","target_slot":"task-badge",
            "contract":"badge","mode":"add","priority":-2
        }]
    }"#,
    )
    .unwrap();
    std::fs::write(earlier.join("main.js"),
        "function App() { return h(Badge, {item: 'org.example.mail', label: 'Urgent mail', count: 2}); }"
    ).unwrap();
    for (id, priority, count) in [
        ("org.example.replace-low", 1, 11),
        ("org.example.replace-high", 9, 19),
        ("org.example.replace-tie", 9, 23),
    ] {
        let replacement = root.path().join(id);
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(
            replacement.join("plugin.json"),
            format!(
                r#"{{"api_version":1,"id":"{id}","name":"Replacement badge","entry":"main.js","contributes":[{{"target_plugin":"org.nickel.taskbar","target_slot":"task-badge","contract":"badge","mode":"replace","priority":{priority}}}]}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            replacement.join("main.js"),
            format!(
                "function App() {{ return h(Badge, {{item: 'org.example.mail', label: 'Replacement', count: {count}}}); }}"
            ),
        )
        .unwrap();
    }
    let mut shell = LiveShell::new().unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    for (_, descriptor) in std::mem::take(&mut catalog.packages) {
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(descriptor.manifest.id.clone(), descriptor);
    }
    assert!(
        shell
            .set_plugin_enabled("org.example.mail-badge", true)
            .unwrap()
    );
    assert!(
        shell
            .set_plugin_enabled("org.example.priority-badge", true)
            .unwrap()
    );
    assert_eq!(shell.plugin_slot_hosts.len(), 2);

    shell.windows = vec![crate::model::OpenWindow {
        id: crate::model::WindowId(71),
        application_id: Some(crate::model::ApplicationId::new("org.example.mail")),
        active: true,
        title: "Mail".into(),
        state: crate::model::WindowState::default(),
    }];
    let (live_data, _) = shell.taskbar_plugin_render_data("12:00");
    let live_data: serde_json::Value = serde_json::from_str(&live_data).unwrap();
    assert_eq!(
        live_data["slots"]["task-badge"][0]["item"],
        "org.example.mail"
    );
    assert_eq!(live_data["slots"]["task-badge"][0]["count"], 2);
    let badge_counts = |shell: &mut LiveShell| {
        let (data, _) = shell.taskbar_plugin_render_data("12:00");
        let data: serde_json::Value = serde_json::from_str(&data).unwrap();
        data["slots"]["task-badge"]
            .as_array()
            .unwrap()
            .iter()
            .map(|badge| badge["count"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(badge_counts(&mut shell), [2, 7]);
    let host = UiHost::new(
        crate::plugin_panel::PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::taskbar_manifest(),
            "main.js",
            live_data.to_string(),
        )
        .unwrap(),
        800,
        56,
    );
    assert!(
        host.accessibility_nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some("Unread mail: 7"))
    );
    assert!(
        host.accessibility_nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some("Urgent mail: 2"))
    );

    shell
        .set_plugin_enabled("org.example.replace-low", true)
        .unwrap();
    shell
        .set_plugin_enabled("org.example.replace-high", true)
        .unwrap();
    shell
        .set_plugin_enabled("org.example.replace-tie", true)
        .unwrap();
    let status = shell.plugin_status_snapshot();
    assert!(
        status
            .plugins
            .iter()
            .find(|plugin| plugin.id == "org.example.replace-high")
            .unwrap()
            .composition[0]
            .contains("superseded")
    );
    assert!(
        !status
            .plugins
            .iter()
            .find(|plugin| plugin.id == "org.example.replace-tie")
            .unwrap()
            .composition[0]
            .contains("superseded")
    );
    assert_eq!(badge_counts(&mut shell), [23, 2, 7]);
    shell
        .set_plugin_enabled("org.example.replace-tie", false)
        .unwrap();
    assert_eq!(badge_counts(&mut shell), [19, 2, 7]);
    shell
        .set_plugin_enabled("org.example.replace-high", false)
        .unwrap();
    assert_eq!(badge_counts(&mut shell), [11, 2, 7]);
    shell
        .set_plugin_enabled("org.example.replace-low", false)
        .unwrap();

    shell
        .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
        .unwrap();
    let inactive = shell.plugin_status_snapshot();
    assert!(
        inactive
            .plugins
            .iter()
            .find(|plugin| plugin.id == "org.example.mail-badge")
            .unwrap()
            .composition[0]
            .contains("target inactive")
    );
    shell
        .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, true)
        .unwrap();

    assert!(
        shell
            .set_plugin_enabled("org.example.mail-badge", false)
            .unwrap()
    );
    assert_eq!(shell.plugin_slot_hosts.len(), 1);
    assert!(
        shell
            .set_plugin_enabled("org.example.priority-badge", false)
            .unwrap()
    );
    assert!(shell.plugin_slot_hosts.is_empty());
    assert_eq!(
        shell
            .plugin_registry
            .get("org.example.mail-badge")
            .unwrap()
            .memory,
        nickel_core::plugins::PluginMemory::default()
    );
}

#[test]
fn badge_slot_can_target_an_installed_window_plugin() {
    let root = tempfile::tempdir().unwrap();
    let provider = root.path().join("org.example.badge-host");
    std::fs::create_dir(&provider).unwrap();
    std::fs::write(
        provider.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.badge-host","name":"Badge host",
            "entry":"main.js","surfaces":[{"id":"main","kind":"window","width":320,"height":180}],
            "provides_slots":[{"id":"status","contract":"badge","replaceable":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        provider.join("main.js"),
        "function App() { return h(Window, {width: 320, height: 180}, h(Text, {}, 'Host')); }",
    )
    .unwrap();
    let contributor = root.path().join("org.example.status-badge");
    std::fs::create_dir(&contributor).unwrap();
    std::fs::write(
        contributor.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.status-badge","name":"Status badge",
            "entry":"main.js","contributes":[{"target_plugin":"org.example.badge-host",
            "target_slot":"status","contract":"badge","mode":"add"}]}"#,
    )
    .unwrap();
    std::fs::write(
        contributor.join("main.js"),
        "function App() { return h(Badge, {item: 'mail', label: 'Unread', count: 4}); }",
    )
    .unwrap();

    let mut shell = LiveShell::new().unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    assert!(catalog.failures.is_empty());
    for (_, descriptor) in std::mem::take(&mut catalog.packages) {
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(descriptor.manifest.id.clone(), descriptor);
    }
    shell
        .set_plugin_enabled("org.example.badge-host", true)
        .unwrap();
    shell
        .set_plugin_enabled("org.example.status-badge", true)
        .unwrap();
    let slots = shell
        .plugin_slot_projection("org.example.badge-host")
        .unwrap();
    assert_eq!(slots["status"][0]["item"], "mail");
    assert_eq!(slots["status"][0]["count"], 4);
    shell
        .set_plugin_enabled("org.example.status-badge", false)
        .unwrap();
    assert!(
        shell
            .plugin_slot_projection("org.example.badge-host")
            .unwrap()["status"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn section_slot_can_target_an_installed_window_plugin() {
    let root = tempfile::tempdir().unwrap();
    let provider = root.path().join("org.example.section-host");
    std::fs::create_dir(&provider).unwrap();
    std::fs::write(
        provider.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.section-host","name":"Section host",
            "entry":"main.js","surfaces":[{"id":"main","kind":"window","width":320,"height":180}],
            "provides_slots":[{"id":"status","contract":"section","replaceable":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        provider.join("main.js"),
        "function App() { return h(Window, {width: 320, height: 180}, h(Text, {}, 'Host')); }",
    )
    .unwrap();
    let contributor = root.path().join("org.example.status-section");
    std::fs::create_dir(&contributor).unwrap();
    std::fs::write(
        contributor.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.status-section","name":"Status section",
            "entry":"main.js","capabilities":["launcher-show"],
            "contributes":[{"target_plugin":"org.example.section-host",
            "target_slot":"status","contract":"section","mode":"add"}]}"#,
    )
    .unwrap();
    std::fs::write(
        contributor.join("main.js"),
        "function App() { return h(Section, {id: 'open', label: 'Apps', value: 'Ready', onClick: () => nickel.request('show-launcher')}); }",
    )
    .unwrap();

    let mut shell = LiveShell::new().unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    assert!(catalog.failures.is_empty());
    for (_, descriptor) in std::mem::take(&mut catalog.packages) {
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(descriptor.manifest.id.clone(), descriptor);
    }
    shell
        .set_plugin_enabled("org.example.section-host", true)
        .unwrap();
    shell
        .set_plugin_enabled("org.example.status-section", true)
        .unwrap();
    let slots = shell
        .plugin_slot_projection("org.example.section-host")
        .unwrap();
    assert_eq!(slots["status"][0]["label"], "Apps");
    assert_eq!(slots["status"][0]["value"], "Ready");
    let invoke = || crate::plugin_panel::PluginEffect::InvokePluginSlotSection {
        target_plugin: "org.example.section-host".into(),
        slot_id: "status".into(),
        plugin_id: "org.example.status-section".into(),
        id: "open".into(),
    };
    assert!(shell.apply_plugin_effects(vec![invoke()]));
    assert!(shell.launcher_visible);
    shell
        .set_plugin_enabled("org.example.status-section", false)
        .unwrap();
    assert!(!shell.apply_plugin_effects(vec![invoke()]));
}

#[test]
fn installed_dock_uses_jsx_distance_and_output_within_its_grant() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.placed-dock");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.placed-dock","name":"Placed dock",
            "entry":"main.js","stylesheet":"ui.css","surfaces":[{"id":"main","kind":"dock","width":480,
            "height":80,"bottom_offset":48,"output":"all"}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(FixedWindow, {width: 400, height: 64, output: 'primary', edge: 'bottom', className: 'dock'}, h(Text, {}, 'Dock')); }",
    )
    .unwrap();
    std::fs::write(directory.join("ui.css"), "window.dock { bottom: 20px; }").unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove("org.example.placed-dock").unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(descriptor.manifest.id.clone(), descriptor);
    shell
        .set_plugin_enabled("org.example.placed-dock", true)
        .unwrap();
    let (_, surface) = shell
        .plugin_panels()
        .into_iter()
        .find(|(key, _)| key.plugin_id == "org.example.placed-dock")
        .unwrap();
    assert_eq!(
        (surface.width, surface.height, surface.bottom_offset),
        (400, 64, 20)
    );
    assert_eq!(
        surface.output,
        nickel_core::plugins::PluginOutputScope::Primary
    );
}

#[test]
fn installed_top_overlay_uses_css_distance_within_its_grant() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.top-overlay");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.top-overlay","name":"Top overlay",
            "entry":"main.js","stylesheet":"ui.css","surfaces":[
            {"id":"home","kind":"window","width":320,"height":180},
            {"id":"notice","kind":"overlay","width":240,"height":80,
            "anchor":"top-right","offset_y":36}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { const notice = nickel.data.surface.id === 'notice'; return h(Window, {width: notice ? 240 : 320, height: notice ? 80 : 180, placement: notice ? 'fixed' : 'managed'}, h(Text, {}, notice ? 'Notice' : 'Home')); }",
    )
    .unwrap();
    std::fs::write(directory.join("ui.css"), "window#notice { top: 14px; }").unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove("org.example.top-overlay").unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(descriptor.manifest.id.clone(), descriptor);
    shell
        .set_plugin_enabled("org.example.top-overlay", true)
        .unwrap();
    shell
        .show_plugin_window("org.example.top-overlay", "notice")
        .unwrap();
    let (_, surface) = shell
        .plugin_panels()
        .into_iter()
        .find(|(key, _)| key.plugin_id == "org.example.top-overlay" && key.surface_id == "notice")
        .unwrap();
    assert_eq!(surface.offset_y, 14);
}

#[test]
fn installed_panel_can_be_enabled_measured_and_disabled() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.panel");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.panel","name":"External Panel","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":360,"height":96,"bottom_offset":12,"output":"primary"}],"capabilities":["settings-write"],"settings":[{"id":"show-label","label":"Show label","kind":"boolean","default":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {}, h(Text, {}, nickel.data.settings['show-label'] ? 'External panel' : 'Hidden')); }",
    )
    .unwrap();
    #[cfg(target_os = "linux")]
    let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
        crate::session_host::default_session_host(),
    ));
    #[cfg(target_os = "linux")]
    let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
    #[cfg(not(target_os = "linux"))]
    let mut shell = LiveShell::new().unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    assert!(catalog.failures.is_empty());
    let descriptor = catalog.packages.remove("org.example.panel").unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(descriptor.manifest.id.clone(), descriptor);

    assert!(shell.set_plugin_enabled("org.example.panel", true).unwrap());
    assert!(shell.surface_visible(crate::winit_shell::SurfaceRole::Panel));
    assert_eq!(shell.plugin_panel_surface().width, 360);
    let commands = shell.scene(crate::winit_shell::SurfaceRole::Panel, 360, 96);
    assert!(!commands.is_empty());
    assert!(commands.iter().any(|command| matches!(command,
        nickel_ui::backend::PaintCommand::Text { text, .. } if text == "External panel"
    )));
    let status = shell.plugin_status_snapshot();
    let panel = status
        .plugins
        .iter()
        .find(|plugin| plugin.id == "org.example.panel")
        .unwrap();
    assert!(panel.desired_enabled);
    assert!(panel.memory.native_ui_bytes.unwrap_or(0) > 0);
    #[cfg(target_os = "linux")]
    assert!(host.take_commands().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::PublishPluginStatus { snapshot }
            if snapshot.plugins.iter().any(|plugin|
                plugin.id == "org.example.panel" && plugin.memory.native_ui_bytes.unwrap_or(0) > 0)
    )));
    assert_eq!(panel.settings.len(), 1);
    assert_eq!(panel.settings[0].value, serde_json::json!(true));
    let panel_key = shell.plugin_panels()[0].0.clone();
    let token_before_setting = shell.plugin_panel_change_token(&panel_key).unwrap();

    let generation = status.activation_generation;
    assert!(shell.apply_plugin_effects(vec![
        crate::plugin_panel::PluginEffect::SetPluginSetting {
            plugin_id: "org.example.panel".into(),
            key: "show-label".into(),
            value: serde_json::json!(false),
        },
    ]));
    assert!(
        !shell
            .set_plugin_setting("org.example.panel", "show-label", serde_json::json!(false))
            .unwrap()
    );
    assert!(
        shell
            .set_plugin_setting("org.example.panel", "show-label", serde_json::json!("bad"))
            .is_err()
    );
    let updated = shell.plugin_status_snapshot();
    assert_eq!(updated.activation_generation, generation + 1);
    let panel = updated
        .plugins
        .iter()
        .find(|plugin| plugin.id == "org.example.panel")
        .unwrap();
    assert_eq!(panel.settings[0].value, serde_json::json!(false));
    assert!(panel.memory.native_ui_bytes.unwrap_or(0) > 0);
    assert_ne!(
        shell.plugin_panel_change_token(&panel_key),
        Some(token_before_setting)
    );
    assert!(shell.surface_visible(crate::winit_shell::SurfaceRole::Panel));
    let commands = shell.scene(crate::winit_shell::SurfaceRole::Panel, 360, 96);
    assert!(commands.iter().any(|command| matches!(command,
        nickel_ui::backend::PaintCommand::Text { text, .. } if text == "Hidden"
    )));

    assert!(
        shell
            .set_plugin_enabled("org.example.panel", false)
            .unwrap()
    );
    assert!(!shell.plugin_surface_matches(&panel_key));
    assert!(!shell.apply_plugin_effects(vec![
        crate::plugin_panel::PluginEffect::SetPluginSetting {
            plugin_id: "org.example.panel".into(),
            key: "show-label".into(),
            value: serde_json::json!(true),
        },
    ]));
    assert_eq!(
        shell.plugin_settings["org.example.panel"]["show-label"],
        serde_json::json!(false)
    );
}

#[test]
fn installed_panel_with_windows_read_tracks_the_live_window_list() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.window-list";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.window-list","name":"Window List","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":360,"height":96}],"capabilities":["windows-read","windows-focus"]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {}, h(Column, {}, (nickel.data.windows || []).map(window => h(Button, {key: window.id, id: 'window-' + window.id, onClick: () => nickel.request({type: 'window-action', action: 'activate', window: window.id})}, window.title)))); }",
    )
    .unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    assert!(catalog.failures.is_empty());
    let descriptor = catalog.packages.remove(id).unwrap();
    let session = Arc::new(crate::session_host::StagedSessionHost::new(
        crate::session_host::default_session_host(),
    ));
    let mut shell = LiveShell::new_with_session_host(session.clone()).unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell.external_plugin_packages.insert(id.into(), descriptor);
    shell.set_plugin_enabled(id, true).unwrap();
    shell.windows = vec![crate::model::OpenWindow {
        id: crate::model::WindowId(71),
        application_id: Some(crate::model::ApplicationId::new("org.example.editor")),
        active: true,
        title: "Editor".into(),
        state: crate::model::WindowState::default(),
    }];
    let projected = shell.external_plugin_windows(id).unwrap();
    assert_eq!(projected[0]["id"], "71");
    assert_eq!(projected[0]["applicationId"], "org.example.editor");
    let (key, surface) = shell
        .plugin_panels()
        .into_iter()
        .find(|(key, _)| key.plugin_id == id)
        .unwrap();
    shell
        .plugin_panel_scene(&key, surface.width, surface.height)
        .unwrap();
    let button = shell
        .plugin_panel_host_for(&key)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Editor".into(),
        })
        .unwrap();
    assert!(button.bounds.size.width > 0.0);
    session.take_commands();
    assert!(shell.plugin_panel_host_ui_for(
        &key,
        nickel_ui::UiEvent::AccessibilityActivate(button.id),
        surface.width,
        surface.height,
    ));
    assert!(session.take_commands().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            window: crate::model::WindowId(71),
            action: crate::platform::WindowAction::Activate,
        }
    )));
    shell.windows[0].state.capabilities.activate = false;
    assert!(
        !shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::ActivateWindow(
            crate::model::WindowId(71)
        ),])
    );
    assert!(session.take_commands().is_empty());

    shell.windows[0].state.capabilities.close = false;
    assert!(
        !shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::CloseWindow(
            crate::model::WindowId(71)
        ),])
    );
    assert!(session.take_commands().is_empty());
    shell.windows[0].state.capabilities.close = true;
    assert!(
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::CloseWindow(
            crate::model::WindowId(71)
        ),])
    );
    assert!(session.take_commands().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            window: crate::model::WindowId(71),
            action: crate::platform::WindowAction::Close,
        }
    )));

    shell.windows.clear();
    shell
        .plugin_panel_scene(&key, surface.width, surface.height)
        .unwrap();
    assert!(
        shell
            .plugin_panel_host_for(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Editor".into(),
            })
            .is_err()
    );
}

#[test]
fn keyed_panel_controller_opens_a_component_dialog() {
    let mut shell = LiveShell::new().unwrap();
    let source = include_str!("../../../../assets/plugins/hello-panel/main.js");
    let app = crate::plugin_panel::PluginPanelApplication::new(source).unwrap();
    shell.plugin_panel_host = Some(UiHost::new(app, 360, 96));
    let (key, surface) = shell.plugin_panels().into_iter().next().unwrap();
    shell
        .plugin_panel_scene(&key, surface.width, surface.height)
        .unwrap();
    let host = shell.plugin_panel_host_for(&key).unwrap();
    let open = host
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open dialog".into(),
        })
        .unwrap();
    assert!(host.request_focus(open.id).changed);

    assert!(shell.plugin_panel_host_controller_for(
        &key,
        ControllerAction::Confirm,
        surface.width,
        surface.height,
    ));
    assert!(
        shell
            .plugin_panel_host_for(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Show".into(),
            })
            .is_ok()
    );
}

#[test]
fn taskbar_declaration_joins_active_shell_panel_surfaces() {
    let mut shell = LiveShell::new().unwrap();
    let key = shell.taskbar_surface_key().unwrap();
    let surfaces = shell.shell_panel_surfaces();
    assert_eq!(surfaces[0].0, key);
    let (_, declaration) = surfaces
        .iter()
        .find(|(candidate, _)| candidate == &key)
        .unwrap();
    assert!(declaration.reserve_work_area);
    assert_eq!(
        declaration.output,
        nickel_core::plugins::PluginOutputScope::All
    );

    shell
        .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
        .unwrap();
    assert!(
        shell
            .shell_panel_surfaces()
            .iter()
            .all(|(candidate, _)| candidate != &key)
    );
}

#[test]
fn fixed_shell_surface_keys_follow_bundled_plugin_activation() {
    let mut shell = LiveShell::new().unwrap();
    for (key, id) in [
        (
            crate::plugin_panel::launcher_surface_key(),
            crate::plugin_panel::launcher_manifest().id.clone(),
        ),
        (
            crate::plugin_panel::run_surface_key(),
            crate::plugin_panel::run_manifest().id.clone(),
        ),
        (
            crate::plugin_panel::volume_osd_surface_key(),
            crate::plugin_panel::volume_osd_manifest().id.clone(),
        ),
        (
            crate::plugin_panel::window_preview_surface_key(),
            crate::plugin_panel::window_preview_manifest().id.clone(),
        ),
    ] {
        assert!(shell.shell_fixed_surface_keys().contains(&key));
        shell.set_plugin_enabled(&id, false).unwrap();
        assert!(!shell.shell_fixed_surface_keys().contains(&key));
        shell.set_plugin_enabled(&id, true).unwrap();
        assert!(shell.shell_fixed_surface_keys().contains(&key));
    }
}

#[test]
fn installed_component_window_activates_and_retires_with_its_plugin() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-window");
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let id = package.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);

    assert!(shell.set_plugin_enabled(&id, true).unwrap());
    let panels = shell.plugin_panels();
    assert_eq!(panels.len(), 1);
    let (key, surface) = &panels[0];
    assert_eq!(
        surface.kind,
        nickel_core::plugins::PluginSurfaceKind::Window
    );
    let commands = shell
        .plugin_panel_scene(key, surface.width, surface.height)
        .unwrap();
    assert!(commands.iter().any(|command| matches!(command,
        nickel_ui::backend::PaintCommand::Text { text, .. } if text == "Window plugin"
    )));
    assert!(shell.set_plugin_enabled(&id, false).unwrap());
    assert!(shell.plugin_panels().is_empty());
}

#[test]
fn external_notification_projection_requires_read_capability() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-window");
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let id = descriptor.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    let notification_id =
        shell
            .notification_feed
            .notify_internal(crate::notification::NotificationRequest {
                app_name: "Mail".into(),
                summary: "New mail".into(),
                body: "Hello".into(),
                actions: Vec::new(),
                expire_timeout_ms: 0,
            });
    shell.notification = shell.notification_feed.snapshot();
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);
    assert!(shell.external_plugin_notifications(&id).is_none());

    shell
        .external_plugin_packages
        .get_mut(&id)
        .unwrap()
        .manifest
        .capabilities
        .push(nickel_core::plugins::PluginCapability::NotificationsRead);
    let data = shell.external_plugin_notifications(&id).unwrap();
    assert_eq!(data["notification"]["id"], notification_id);
    assert_eq!(data["notification"]["summary"], "New mail");
    assert!(
        data["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == notification_id)
    );

    shell.remote_lease_notifications.insert(
        notification_id,
        nickel_session_protocol::RemotePendingLease {
            pending_generation: 1,
            client_id: "test".into(),
            client_label: "Test".into(),
            request: nickel_session_protocol::RemoteLeaseRequest {
                renewal: None,
                scope: nickel_session_protocol::RemoteResourceScope::FullSession,
                duration_seconds: None,
                allow_resumption: false,
                full_debug: false,
            },
            resource_label: None,
            changes: nickel_session_protocol::RemoteLeaseRequestChanges::default(),
        },
    );
    let protected = shell.external_plugin_notifications(&id).unwrap();
    assert!(protected["notification"].is_null());
    assert!(protected["history"].as_array().unwrap().is_empty());
}

#[test]
fn external_application_catalog_requires_read_capability() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-window");
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let id = descriptor.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    shell
        .launcher
        .replace_discovered_applications(vec![crate::model::Application::new(
            "org.example.Editor".into(),
            "Editor".into(),
            None,
            None,
            None,
        )]);
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);
    assert!(shell.external_plugin_applications(&id).is_none());

    shell
        .external_plugin_packages
        .get_mut(&id)
        .unwrap()
        .manifest
        .capabilities
        .push(nickel_core::plugins::PluginCapability::ApplicationsRead);
    let data = shell.external_plugin_applications(&id).unwrap();
    assert!(data.as_array().unwrap().iter().any(|application| {
        application["id"] == "org.example.Editor" && application["name"] == "Editor"
    }));
}

#[test]
fn installed_windows_use_jsx_sizes_within_manifest_bounds() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.bounded-windows");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.bounded-windows","name":"Bounded windows","entry":"main.js","surfaces":[{"id":"first","kind":"window","width":400,"height":240},{"id":"second","kind":"window","width":450,"height":260}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { const first = nickel.data.surface.id === 'first'; return h(Window, {width: first ? 360 : 420, height: first ? 220 : 240}, h(Text, null, 'Bounded window')); }",
    )
    .unwrap();
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell
        .external_plugin_packages
        .insert("org.example.bounded-windows".into(), descriptor);
    shell
        .set_plugin_enabled("org.example.bounded-windows", true)
        .unwrap();
    let surfaces = shell.plugin_panels();
    assert_eq!(surfaces.len(), 2);
    assert_eq!((surfaces[0].1.width, surfaces[0].1.height), (360, 220));
    assert_eq!((surfaces[1].1.width, surfaces[1].1.height), (420, 240));
}

#[test]
fn installed_plugin_can_reposition_only_its_open_window() {
    use nickel_core::plugins::PluginSurfaceAnchor;

    let root = tempfile::tempdir().unwrap();
    let id = "org.example.placed-window";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.placed-window","name":"Placed window","entry":"main.js","surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Window, {id: 'main', width: 400, height: 240}, h(Text, null, 'Window')); }",
    )
    .unwrap();
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell.external_plugin_packages.insert(id.into(), descriptor);
    shell.set_plugin_enabled(id, true).unwrap();

    assert!(
        shell
            .set_plugin_window_placement(id, "main", PluginSurfaceAnchor::TopRight, -24, 24)
            .unwrap()
    );
    let surface = &shell.plugin_panels()[0].1;
    assert_eq!(surface.anchor, PluginSurfaceAnchor::TopRight);
    assert_eq!((surface.offset_x, surface.offset_y), (-24, 24));
    assert!(
        !shell
            .set_plugin_window_placement(id, "main", PluginSurfaceAnchor::TopRight, -24, 24)
            .unwrap()
    );
    assert!(
        shell
            .set_plugin_window_placement(id, "other", PluginSurfaceAnchor::Center, 0, 0)
            .is_err()
    );
    assert!(
        shell
            .set_plugin_window_placement(id, "main", PluginSurfaceAnchor::Center, 8193, 0)
            .is_err()
    );
    shell.set_plugin_enabled(id, false).unwrap();
    assert!(
        shell
            .set_plugin_window_placement(id, "main", PluginSurfaceAnchor::Center, 0, 0)
            .is_err()
    );
}

#[test]
fn closing_one_installed_window_preserves_its_sibling_and_memory_account() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.two-windows";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.two-windows","name":"Two windows","entry":"main.js","surfaces":[{"id":"first","kind":"window","width":360,"height":220},{"id":"second","kind":"window","width":400,"height":240}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {}, h(Button, {id: 'reopen', onClick: () => nickel.request({type: 'show-plugin-surface', surfaceId: 'first'})}, 'Reopen first')); }",
    )
    .unwrap();
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell.external_plugin_packages.insert(id.into(), descriptor);
    shell.set_plugin_enabled(id, true).unwrap();
    let panels = shell.plugin_panels();
    assert_eq!(panels.len(), 2);
    for (key, surface) in &panels {
        shell.plugin_panel_scene(key, surface.width, surface.height);
    }
    let both_bytes = shell
        .plugin_registry
        .get(id)
        .unwrap()
        .memory
        .native_ui_bytes
        .unwrap();
    assert!(both_bytes > 0);

    assert!(shell.close_plugin_window(&panels[0].0).unwrap());
    let remaining = shell.plugin_panels();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, panels[1].0);
    let status = shell.plugin_registry.get(id).unwrap();
    assert!(status.desired_enabled);
    assert_eq!(status.health, nickel_core::plugins::PluginHealth::Running);
    assert!(status.memory.native_ui_bytes.unwrap() < both_bytes);
    assert!(
        shell
            .plugin_panel_scene(&remaining[0].0, remaining[0].1.width, remaining[0].1.height)
            .is_some()
    );

    let button = shell
        .plugin_panel_host_for(&remaining[0].0)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Reopen first".into(),
        })
        .unwrap();
    let point = nickel_input::Point {
        x: f64::from(button.bounds.origin.x + button.bounds.size.width / 2.0),
        y: f64::from(button.bounds.origin.y + button.bounds.size.height / 2.0),
    };
    assert!(shell.plugin_panel_host_input_for(
        &remaining[0].0,
        nickel_input::InputEvent::FocusLost {
            order: nickel_input::EventOrder(1),
        },
        remaining[0].1.width,
        remaining[0].1.height,
    ));
    assert!(
        !shell
            .plugin_panel_host_for(&remaining[0].0)
            .unwrap()
            .inspect()
            .window_focused
    );
    assert!(shell.plugin_panel_host_input_for(
        &remaining[0].0,
        nickel_input::InputEvent::FocusGained {
            order: nickel_input::EventOrder(2),
        },
        remaining[0].1.width,
        remaining[0].1.height,
    ));
    assert!(
        shell
            .plugin_panel_host_for(&remaining[0].0)
            .unwrap()
            .inspect()
            .window_focused
    );
    let pointer = |edge, order| {
        nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            device: nickel_input::DeviceId(1),
            order: nickel_input::EventOrder(order),
            position: Some(point),
            button: nickel_input::PointerButton::Primary,
            edge,
        })
    };
    shell.plugin_panel_host_input_for(
        &remaining[0].0,
        pointer(nickel_input::KeyEdge::Pressed, 3),
        remaining[0].1.width,
        remaining[0].1.height,
    );
    assert!(shell.plugin_panel_host_input_for(
        &remaining[0].0,
        pointer(nickel_input::KeyEdge::Released, 4),
        remaining[0].1.width,
        remaining[0].1.height,
    ));
    assert_eq!(shell.plugin_panels().len(), 2);
    assert!(shell.plugin_surface_matches(&panels[0].0));
    assert!(!shell.show_plugin_window(id, "first").unwrap());
    assert!(shell.show_plugin_window(id, "missing").is_err());

    shell.set_plugin_enabled(id, false).unwrap();
    shell.set_plugin_enabled(id, true).unwrap();
    assert_eq!(shell.plugin_panels().len(), 2);
}

#[test]
fn declared_dialog_starts_closed_and_dismisses_without_retiring_its_plugin() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-surface-dialog");
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let id = package.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);

    shell.set_plugin_enabled(&id, true).unwrap();
    let status = shell.plugin_status_snapshot();
    let reviewed = status
        .plugins
        .iter()
        .find(|plugin| plugin.id == id)
        .unwrap();
    assert!(
        reviewed
            .surfaces
            .iter()
            .any(|surface| surface == "confirm: dialog (owned by home)")
    );
    let home = shell.plugin_panels();
    assert_eq!(home.len(), 1);
    assert_eq!(home[0].0.surface_id, "home");
    shell
        .plugin_panel_scene(&home[0].0, home[0].1.width, home[0].1.height)
        .unwrap();
    let home_bytes = shell
        .plugin_registry
        .get(&id)
        .unwrap()
        .memory
        .native_ui_bytes
        .unwrap();
    let open = shell
        .plugin_panel_host_for(&home[0].0)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Open dialog".into(),
        })
        .unwrap();
    assert!(shell.plugin_panel_host_ui_for(
        &home[0].0,
        nickel_ui::UiEvent::AccessibilityActivate(open.id),
        home[0].1.width,
        home[0].1.height,
    ));
    let opened = shell.plugin_panels();
    assert_eq!(opened.len(), 2);
    let dialog = opened
        .iter()
        .find(|(key, _)| key.surface_id == "confirm")
        .unwrap();
    assert_eq!(
        dialog.1.kind,
        nickel_core::plugins::PluginSurfaceKind::Dialog
    );
    shell
        .plugin_panel_scene(&dialog.0, dialog.1.width, dialog.1.height)
        .unwrap();
    assert!(
        shell
            .plugin_registry
            .get(&id)
            .unwrap()
            .memory
            .native_ui_bytes
            .unwrap()
            > home_bytes
    );
    let dismiss = shell
        .plugin_panel_host_for(&dialog.0)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Dismiss".into(),
        })
        .unwrap();
    assert!(shell.plugin_panel_host_ui_for(
        &dialog.0,
        nickel_ui::UiEvent::AccessibilityActivate(dismiss.id),
        dialog.1.width,
        dialog.1.height,
    ));
    assert_eq!(shell.plugin_panels(), home);
    let status = shell.plugin_registry.get(&id).unwrap();
    assert!(status.desired_enabled);
    assert_eq!(status.health, nickel_core::plugins::PluginHealth::Running);
    assert_eq!(status.memory.native_ui_bytes, Some(home_bytes));

    assert!(shell.show_plugin_window(&id, "confirm").unwrap());
    assert_eq!(shell.plugin_panels().len(), 2);
    assert!(shell.close_plugin_window(&home[0].0).unwrap());
    assert!(shell.plugin_panels().is_empty());
    let status = shell.plugin_registry.get(&id).unwrap();
    assert!(!status.desired_enabled);
    assert_eq!(status.health, nickel_core::plugins::PluginHealth::Disabled);
    assert_eq!(status.memory.native_ui_bytes, None);
}

#[test]
fn closing_dialog_owner_retires_its_dialog_but_preserves_sibling_window() {
    let directory = tempfile::tempdir().unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../assets/plugins/example-surface-dialog/plugin.json"
    ))
    .unwrap();
    manifest["surfaces"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "spare",
            "kind": "window",
            "width": 420,
            "height": 280
        }));
    std::fs::write(
        directory.path().join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("main.js"),
        include_str!("../../../../assets/plugins/example-surface-dialog/main.js"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("ui.css"),
        include_str!("../../../../assets/plugins/example-surface-dialog/ui.css"),
    )
    .unwrap();
    let package = nickel_core::plugins::PluginPackage::load(directory.path()).unwrap();
    let id = package.manifest.id.clone();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory: directory.path().to_path_buf(),
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);
    shell.set_plugin_enabled(&id, true).unwrap();
    assert!(shell.show_plugin_window(&id, "confirm").unwrap());
    assert_eq!(shell.plugin_panels().len(), 3);
    let home = shell
        .plugin_panels()
        .into_iter()
        .find(|(key, _)| key.surface_id == "home")
        .unwrap();
    assert!(shell.close_plugin_window(&home.0).unwrap());
    let remaining = shell.plugin_panels();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0.surface_id, "spare");
    assert!(shell.show_plugin_window(&id, "confirm").is_err());
    let status = shell.plugin_registry.get(&id).unwrap();
    assert!(status.desired_enabled);
    assert_eq!(status.health, nickel_core::plugins::PluginHealth::Running);
}

#[test]
fn declared_overlay_opens_and_hides_without_retiring_its_plugin() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-overlay");
    let package = nickel_core::plugins::PluginPackage::load(&directory).unwrap();
    let descriptor = nickel_core::plugins::PluginPackageDescriptor {
        directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let id = package.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    shell.plugin_registry.register(package.manifest).unwrap();
    shell
        .external_plugin_packages
        .insert(id.clone(), descriptor);
    shell.set_plugin_enabled(&id, true).unwrap();
    let home = shell.plugin_panels();
    assert_eq!(home.len(), 1);
    assert_eq!(home[0].0.surface_id, "home");
    shell
        .plugin_panel_scene(&home[0].0, home[0].1.width, home[0].1.height)
        .unwrap();
    let home_bytes = shell
        .plugin_registry
        .get(&id)
        .unwrap()
        .memory
        .native_ui_bytes
        .unwrap();
    let show = shell
        .plugin_panel_host_for(&home[0].0)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Show overlay".into(),
        })
        .unwrap();
    assert!(shell.plugin_panel_host_ui_for(
        &home[0].0,
        nickel_ui::UiEvent::AccessibilityActivate(show.id),
        home[0].1.width,
        home[0].1.height,
    ));
    let opened = shell.plugin_panels();
    assert_eq!(opened.len(), 2);
    let overlay = opened
        .iter()
        .find(|(key, _)| key.surface_id == "notice")
        .unwrap();
    assert_eq!(
        overlay.1.kind,
        nickel_core::plugins::PluginSurfaceKind::Overlay
    );
    shell
        .plugin_panel_scene(&overlay.0, overlay.1.width, overlay.1.height)
        .unwrap();
    assert!(
        shell
            .plugin_registry
            .get(&id)
            .unwrap()
            .memory
            .native_ui_bytes
            .unwrap()
            > home_bytes
    );
    let hide = shell
        .plugin_panel_host_for(&overlay.0)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Close overlay".into(),
        })
        .unwrap();
    assert!(shell.plugin_panel_host_ui_for(
        &overlay.0,
        nickel_ui::UiEvent::AccessibilityActivate(hide.id),
        overlay.1.width,
        overlay.1.height,
    ));
    assert_eq!(shell.plugin_panels(), home);
    let status = shell.plugin_registry.get(&id).unwrap();
    assert!(status.desired_enabled);
    assert_eq!(status.health, nickel_core::plugins::PluginHealth::Running);
    assert_eq!(status.memory.native_ui_bytes, Some(home_bytes));
}

#[test]
fn installed_dock_uses_declared_offset_and_translucent_panel() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.dock");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.dock","name":"Example Dock","entry":"main.js","surfaces":[{"id":"main","kind":"dock","width":420,"height":80,"bottom_offset":32,"output":"primary"}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {height: 80, background: 0x80202020}, h(Text, {}, 'Dock')); }",
    )
    .unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove("org.example.dock").unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(descriptor.manifest.id.clone(), descriptor);

    assert!(shell.set_plugin_enabled("org.example.dock", true).unwrap());
    let dock_key = shell.plugin_panels()[0].0.clone();
    assert_eq!(
        shell.plugin_panel_surface().kind,
        nickel_core::plugins::PluginSurfaceKind::Dock
    );
    assert_eq!(shell.plugin_panel_surface().bottom_offset, 32);
    let commands = shell.scene(crate::winit_shell::SurfaceRole::Panel, 420, 80);
    assert!(commands.iter().any(|command| matches!(command,
        nickel_ui::backend::PaintCommand::Text { text, .. } if text == "Dock"
    )));
    assert!(commands.iter().any(|command| matches!(command,
        nickel_ui::backend::PaintCommand::Fill { color, .. }
        | nickel_ui::backend::PaintCommand::RoundedFill { color, .. }
        if *color == 0x80202020
    )));
    assert!(shell.set_plugin_enabled("org.example.dock", false).unwrap());
    assert!(!shell.plugin_surface_matches(&dock_key));
}

#[test]
fn two_installed_panels_render_and_retire_independently() {
    let root = tempfile::tempdir().unwrap();
    for (id, label, offset) in [
        ("org.example.clock", "Clock", 12),
        ("org.example.mail", "Mail", 36),
    ] {
        let directory = root.path().join(id);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("plugin.json"),
            format!(
                r#"{{"api_version":1,"id":"{id}","name":"{label}","entry":"main.js","surfaces":[{{"id":"main","kind":"panel","width":360,"height":64,"bottom_offset":{offset}}}]}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            directory.join("main.js"),
            format!("function App() {{ return h(Panel, {{}}, h(Text, {{}}, '{label}')); }}"),
        )
        .unwrap();
    }
    let mut shell = LiveShell::new().unwrap();
    let catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    assert!(catalog.failures.is_empty());
    for (id, descriptor) in catalog.packages {
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), descriptor);
        assert!(shell.set_plugin_enabled(&id, true).unwrap());
    }
    let panels = shell.plugin_panels();
    assert_eq!(panels.len(), 2);
    assert_eq!(shell.plugin_panel_bottom_offset(&panels[0].0), Some(12));
    assert_eq!(shell.plugin_panel_bottom_offset(&panels[1].0), Some(36));
    for (key, surface) in &panels {
        assert!(shell.plugin_surface_matches(key));
        let commands = shell
            .plugin_panel_scene(key, surface.width, surface.height)
            .unwrap();
        let label = if key.plugin_id.ends_with("clock") {
            "Clock"
        } else {
            "Mail"
        };
        assert!(commands.iter().any(|command| matches!(command,
            nickel_ui::backend::PaintCommand::Text { text, .. } if text == label
        )));
        assert!(
            shell
                .plugin_status_snapshot()
                .plugins
                .iter()
                .find(|plugin| plugin.id == key.plugin_id)
                .unwrap()
                .memory
                .native_ui_bytes
                .is_some_and(|bytes| bytes > 0)
        );
    }
    assert!(
        shell
            .set_plugin_enabled("org.example.clock", false)
            .unwrap()
    );
    assert_eq!(shell.plugin_panels().len(), 1);
    assert!(!shell.plugin_surface_matches(&panels[0].0));
    assert!(shell.plugin_surface_matches(&panels[1].0));
    assert!(shell.plugin_panel_scene(&panels[0].0, 360, 64).is_none());
    assert!(shell.plugin_panel_scene(&panels[1].0, 360, 64).is_some());
}

#[test]
fn one_installed_package_runs_two_surfaces_and_updates_both_settings_views() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.two-surfaces";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.two-surfaces","name":"Two surfaces","entry":"main.js","surfaces":[{"id":"clock","kind":"panel","width":360,"height":64},{"id":"dock","kind":"dock","width":420,"height":72,"bottom_offset":20}],"settings":[{"id":"show-label","label":"Show label","kind":"boolean","default":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {height: nickel.data.surface.height}, h(Text, {}, nickel.data.surface.id + ':' + (nickel.data.settings['show-label'] ? 'on' : 'off'))); }",
    )
    .unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove(id).unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell.external_plugin_packages.insert(id.into(), descriptor);
    shell.plugin_settings.insert(
        id.into(),
        std::collections::BTreeMap::from([("show-label".into(), serde_json::json!(true))]),
    );
    assert!(shell.set_plugin_enabled(id, true).unwrap());
    let panels = shell.plugin_panels();
    assert_eq!(panels.len(), 2);
    let mut first_bytes = 0;
    for (index, (key, surface)) in panels.iter().enumerate() {
        let commands = shell
            .plugin_panel_scene(key, surface.width, surface.height)
            .unwrap();
        assert!(commands.iter().any(|command| matches!(command,
            nickel_ui::backend::PaintCommand::Text { text, .. } if text == &format!("{}:on", key.surface_id)
        )));
        if index == 0 {
            first_bytes = shell
                .plugin_registry
                .get(id)
                .unwrap()
                .memory
                .native_ui_bytes
                .unwrap();
        }
    }
    let both_bytes = shell
        .plugin_registry
        .get(id)
        .unwrap()
        .memory
        .native_ui_bytes
        .unwrap();
    assert!(first_bytes > 0 && both_bytes > first_bytes);
    assert!(
        shell
            .set_plugin_setting(id, "show-label", serde_json::json!(false))
            .unwrap()
    );
    assert!(
        shell
            .plugin_registry
            .get(id)
            .unwrap()
            .memory
            .native_ui_bytes
            .unwrap_or(0)
            > 0
    );
    for (key, surface) in &panels {
        let commands = shell
            .plugin_panel_scene(key, surface.width, surface.height)
            .unwrap();
        assert!(commands.iter().any(|command| matches!(command,
            nickel_ui::backend::PaintCommand::Text { text, .. } if text == &format!("{}:off", key.surface_id)
        )));
    }
    assert!(shell.set_plugin_enabled(id, false).unwrap());
    assert!(shell.plugin_panels().is_empty());
    assert_eq!(
        shell.plugin_registry.get(id).unwrap().memory,
        nickel_core::plugins::PluginMemory::default()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn internal_shell_presents_two_surfaces_from_one_package_on_one_output() {
    struct Host;
    impl crate::session_host::SessionHost for Host {
        fn dispatch(
            &self,
            _command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, crate::platform::SessionRequestError>
        {
            Ok(crate::platform::SecureStorageState::Ready)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.multi");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.multi","name":"Multi","entry":"main.js","surfaces":[{"id":"clock","kind":"panel","width":300,"height":64},{"id":"mail","kind":"panel","width":340,"height":68}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {}, h(Text, {}, nickel.data.surface.id)); }",
    )
    .unwrap();
    let mut coordinator = crate::internal_shell::InternalShellCoordinator::new(
        Arc::new(Host),
        crate::winit_shell::PanelEdge::Bottom,
    )
    .unwrap();
    let catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    for (id, descriptor) in catalog.packages {
        let shell = coordinator.shell_mut();
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), descriptor);
        shell.set_plugin_enabled(&id, true).unwrap();
    }
    coordinator.set_outputs(&[crate::internal_shell::InternalOutput {
        x: 0,
        y: 0,
        name: "test".into(),
        width: 1280,
        height: 720,
        scale: 1.0,
    }]);
    let panels = coordinator
        .surfaces()
        .iter()
        .filter(|surface| {
            surface.role == crate::winit_shell::SurfaceRole::Panel
                && surface
                    .plugin
                    .as_ref()
                    .is_some_and(|key| key.plugin_id == "org.example.multi")
        })
        .map(|surface| {
            (
                surface.id,
                surface.plugin.as_ref().unwrap().surface_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(panels.len(), 2);
    for (id, label) in panels {
        assert!(coordinator.visible(id));
        assert!(
            coordinator
                .scene(id)
                .unwrap()
                .iter()
                .any(|command| matches!(command,
                    nickel_ui::backend::PaintCommand::Text { text, .. } if text == &label
                ))
        );
    }
}

#[test]
fn installed_panel_start_failure_is_visible_until_disabled() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("org.example.broken");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.broken","name":"Broken Panel","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":360,"height":96}]}"#,
    )
    .unwrap();
    std::fs::write(directory.join("main.js"), "function App() {}").unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove("org.example.broken").unwrap();
    // Discovery records the declaration, while activation loads its entry again.
    std::fs::write(directory.join("main.js"), "function App( {").unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(descriptor.manifest.id.clone(), descriptor);

    assert!(
        shell
            .set_plugin_enabled("org.example.broken", true)
            .is_err()
    );
    let status = shell.plugin_status_snapshot();
    let panel = status
        .plugins
        .iter()
        .find(|plugin| plugin.id == "org.example.broken")
        .unwrap();
    assert!(panel.desired_enabled);
    assert!(matches!(
        panel.health,
        nickel_session_protocol::PluginRuntimeHealth::Failed(_)
    ));
    assert!(
        shell
            .plugin_panels()
            .iter()
            .all(|(key, _)| key.plugin_id != "org.example.broken")
    );
    assert!(
        shell
            .set_plugin_enabled("org.example.broken", false)
            .unwrap()
    );
}

#[test]
fn installed_plugin_callback_failure_retires_all_surfaces_without_changing_desired_state() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.callback-failure";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.callback-failure","name":"Callback Failure","entry":"main.js","surfaces":[{"id":"left","kind":"panel","width":360,"height":96},{"id":"right","kind":"dock","width":360,"height":96}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { return h(Panel, {}, h(Button, {id: 'crash', onClick: () => { throw Error('callback exploded'); }}, 'Crash plugin')); }",
    )
    .unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove(id).unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell.external_plugin_packages.insert(id.into(), descriptor);
    shell.set_plugin_enabled(id, true).unwrap();
    let panels = shell.plugin_panels();
    assert_eq!(panels.len(), 2);
    for (key, surface) in &panels {
        shell
            .plugin_panel_scene(key, surface.width, surface.height)
            .unwrap();
    }
    let (key, surface) = &panels[0];
    let crash = shell
        .plugin_panel_host_for(key)
        .unwrap()
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Crash plugin".into(),
        })
        .unwrap();
    assert!(shell.plugin_panel_host_ui_for(
        key,
        nickel_ui::UiEvent::AccessibilityActivate(crash.id),
        surface.width,
        surface.height,
    ));
    assert!(
        shell
            .plugin_panels()
            .iter()
            .all(|(key, _)| key.plugin_id != id)
    );
    let entry = shell.plugin_registry.get(id).unwrap();
    assert!(entry.desired_enabled);
    assert!(
        matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("callback exploded"))
    );
    assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
    assert!(shell.taskbar_surface_key().is_some());
    assert!(shell.set_plugin_enabled(id, false).unwrap());
    assert!(shell.set_plugin_enabled(id, true).unwrap());
    assert_eq!(shell.plugin_panels().len(), 2);
}

#[test]
fn failing_slot_projection_retires_provider_without_stopping_contributor() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.widget-host";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.widget-host","name":"Widget Host","entry":"main.js","surfaces":[{"id":"main","kind":"window","width":420,"height":280}],"provides_slots":[{"id":"metrics","contract":"widget","replaceable":true}]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        "function App() { if ((nickel.data.slots.metrics || []).length) throw Error('projection exploded'); return h(Panel, {}, h(Text, {}, 'Provider ready')); }",
    )
    .unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let provider = catalog.packages.remove(id).unwrap();
    let contributor_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/plugins/example-widget-contributor");
    let package = nickel_core::plugins::PluginPackage::load(&contributor_directory).unwrap();
    let contributor = nickel_core::plugins::PluginPackageDescriptor {
        directory: contributor_directory,
        manifest: package.manifest.clone(),
        source_digest: package.source_digest(),
    };
    let contributor_id = contributor.manifest.id.clone();
    let mut shell = LiveShell::new().unwrap();
    for descriptor in [provider, contributor] {
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(descriptor.manifest.id.clone(), descriptor);
    }
    shell.set_plugin_enabled(id, true).unwrap();
    let (key, surface) = shell.plugin_panels().pop().unwrap();
    shell
        .plugin_panel_scene(&key, surface.width, surface.height)
        .unwrap();
    assert!(shell.set_plugin_enabled(&contributor_id, true).unwrap());
    let provider = shell.plugin_registry.get(id).unwrap();
    assert!(provider.desired_enabled);
    assert!(
        matches!(&provider.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("projection exploded"))
    );
    assert_eq!(
        provider.memory,
        nickel_core::plugins::PluginMemory::default()
    );
    assert!(!shell.plugin_surface_matches(&key));
    assert_eq!(
        shell.plugin_registry.get(&contributor_id).unwrap().health,
        nickel_core::plugins::PluginHealth::Running
    );
}
include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");

#[test]
fn pointer_opened_control_center_does_not_paint_initial_keyboard_focus() {
    let mut shell = LiveShell::new().expect("live shell");
    assert_eq!(
        shell.control_host.inspect().modality,
        InputModality::Keyboard
    );
    assert!(
        shell
            .plugin_taskbar_host
            .as_mut()
            .unwrap()
            .adopt_input_modality(InputModality::Pointer)
    );

    shell.apply_panel_action(super::TaskbarAction::Control);

    assert_eq!(
        shell.control_host.inspect().modality,
        InputModality::Pointer
    );
}

#[test]
fn codex_approval_notification_revises_in_place_and_retires_on_resolution() {
    use nickel_codex::{ApprovalContext, ServerRequestId, ThreadId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_session_launcher_visibility(true);
    shell.scene(SurfaceRole::Launcher, 920, 680);
    let typing_focus = shell
        .plugin_launcher_host
        .as_ref()
        .expect("bundled launcher")
        .inspect()
        .keyboard_focus
        .clone();
    assert!(typing_focus.is_some());
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let snapshot = |root: &str| CodexApprovalNotification {
        connection_generation: 1,
        request_revision: if root == "/safe" { 1 } else { 2 },
        thread_id: Some(ThreadId("thread".into())),
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("request".into()),
            approval_type: "item/fileChange/requestApproval".into(),
            summary: "Write files".into(),
            context: ApprovalContext {
                grant_root: Some(root.into()),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Change files".into(),
            scope: Some(root.into()),
            duration: None,
            warning: None,
            detail: Some(format!(
                "Requested files under {root}; Bearer fixture-private-token"
            )),
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, snapshot("/safe"))]);
    shell.refresh_fast();
    assert_eq!(
        shell
            .plugin_launcher_host
            .as_ref()
            .unwrap()
            .inspect()
            .keyboard_focus,
        typing_focus
    );
    let first = shell
        .notification_feed
        .snapshot()
        .expect("pending notification");
    assert_eq!(first.actions.len(), 3);
    assert!(first.body.contains("/safe"));
    assert!(first.body.contains("Review the full operation in Codex"));
    assert!(!first.body.contains("fixture-private-token"));
    assert!(!first.body.contains("Requested files under /safe"));

    shell.sync_codex_approval_notifications(vec![(owner, snapshot("/broader"))]);
    let revised = shell
        .notification_feed
        .snapshot()
        .expect("revised notification");
    assert_eq!(revised.id, first.id);
    assert!(revised.body.contains("/broader"));
    assert_eq!(shell.codex_approval_notifications.len(), 1);

    shell.notification = Some(revised.clone());
    shell.sync_notification_host(420, 180);
    let cancel = shell
        .notification_host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Cancel".into(),
        })
        .expect("cancel decision");
    shell
        .notification_host
        .perform_semantic_action(cancel.id, SemanticAction::Invoke(ActionKind::Activate));
    shell.apply_notification_effects();
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(
            owner,
            snapshot("/broader"),
            nickel_codex_ui::CodexApprovalChoice::Cancel,
        )]
    );
    assert!(
        shell
            .notification_feed
            .history()
            .into_iter()
            .find(|item| item.id == revised.id)
            .unwrap()
            .actions
            .is_empty(),
        "a submitted action must not be offered again"
    );
    shell.dismiss_notification_transport(revised.id);
    assert!(shell.notification.is_none());
    assert!(
        shell
            .dismissed_codex_approval_notifications
            .contains(&revised.id)
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == revised.id)
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .all(|item| !item.body.contains("fixture-private-token"))
    );
    shell.refresh_fast();
    assert!(
        shell.notification.is_none(),
        "dismissal must not re-toast a pending request"
    );

    shell.sync_codex_approval_notifications(Vec::new());
    assert!(shell.codex_approval_notifications.is_empty());
    assert!(shell.dismissed_codex_approval_notifications.is_empty());
    assert!(shell.notification_feed.snapshot().is_none());
}

#[test]
fn codex_notification_reviews_large_source_decision_set_without_truncating_it() {
    use nickel_codex::{ApprovalContext, CommandDecision, ServerRequestId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let snapshot = CodexApprovalNotification {
        connection_generation: 1,
        request_revision: 1,
        thread_id: None,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("choices".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run a command".into(),
            context: ApprovalContext {
                available_decisions: Some(vec![
                    CommandDecision::Accept,
                    CommandDecision::AcceptForSession,
                    CommandDecision::Decline,
                    CommandDecision::Cancel,
                ]),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Run a command".into(),
            scope: Some("/projects/nickel".into()),
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, snapshot)]);
    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.actions.len(), 1);
    assert_eq!(notification.actions[0].key, "review");
    shell.notification = Some(notification);
    shell.sync_notification_host(420, 180);
    let review = shell
        .notification_host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Review in Codex".into(),
        })
        .unwrap();
    shell
        .notification_host
        .perform_semantic_action(review.id, SemanticAction::Invoke(ActionKind::Activate));
    shell.apply_notification_effects();
    assert_eq!(shell.take_codex_approval_reviews(), vec![owner]);
    assert!(shell.take_codex_approval_decisions().is_empty());
}

#[test]
fn simultaneous_codex_and_remote_approvals_keep_distinct_request_owners() {
    use nickel_codex::{ApprovalContext, ServerRequestId};
    use nickel_codex_ui::{CodexApprovalNotification, PendingInteraction};
    use nickel_session_protocol::{
        RemoteLeaseRequest, RemoteLeaseRequestChanges, RemotePendingLease, RemoteResourceScope,
    };
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let remote = RemotePendingLease {
        pending_generation: 7,
        client_id: "remote-client".into(),
        client_label: "Remote controller".into(),
        request: RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::FullSession,
            duration_seconds: Some(600),
            allow_resumption: false,
            full_debug: false,
        },
        resource_label: None,
        changes: RemoteLeaseRequestChanges::default(),
    };
    shell.sync_remote_lease_notifications_from(vec![remote.clone()]);
    let remote_id = *shell.remote_lease_notifications.keys().next().unwrap();

    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let surface = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(surface);
    let codex = |scope: &str| CodexApprovalNotification {
        connection_generation: 2,
        request_revision: if scope == "/first" { 1 } else { 2 },
        thread_id: None,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("codex-request".into()),
            approval_type: "item/fileChange/requestApproval".into(),
            summary: "Change files".into(),
            context: ApprovalContext {
                grant_root: Some(scope.into()),
                ..Default::default()
            },
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Change files".into(),
            scope: Some(scope.into()),
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, codex("/first"))]);
    assert_eq!(shell.remote_lease_notifications.len(), 1);
    assert_eq!(shell.codex_approval_notifications.len(), 1);
    let codex_id = *shell.codex_approval_notifications.keys().next().unwrap();
    assert_ne!(remote_id, codex_id);

    // A revision of one source must not replace or retire the other source's
    // independently pending authority request.
    shell.sync_codex_approval_notifications(vec![(owner, codex("/revised"))]);
    assert!(shell.remote_lease_notifications.contains_key(&remote_id));
    assert!(shell.codex_approval_notifications.contains_key(&codex_id));
    assert_eq!(shell.notification_feed.history().len(), 2);
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == codex_id && item.body.contains("/revised"))
    );
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == remote_id && item.body.contains("full desktop"))
    );
}

#[test]
fn codex_approval_feed_overflow_queues_one_explicit_decline() {
    use nickel_codex::{ApprovalContext, CommandDecision, ServerRequestId, ThreadId};
    use nickel_codex_ui::{CodexApprovalChoice, CodexApprovalNotification, PendingInteraction};
    use nickel_ui::approval::{ApprovalPresentation, RequesterIdentity};

    let mut shell = LiveShell::new().expect("live shell");
    let mut surfaces = nickel_ui::InternalSurfaceSet::new();
    let id = surfaces.insert(
        crate::notification_view::NotificationApp::new(shell.palette),
        1,
        1,
    );
    let owner = super::CodexApprovalOwner::Internal(id);
    let fixture_ids = (0..crate::notification::MAX_NOTIFICATIONS)
        .map(|_| {
            shell
                .notification_feed
                .notify_internal(crate::notification::NotificationRequest {
                    app_name: "Persistent fixture".into(),
                    summary: "Fixture".into(),
                    body: String::new(),
                    actions: Vec::new(),
                    expire_timeout_ms: 0,
                })
        })
        .collect::<Vec<_>>();
    assert!(fixture_ids.iter().all(|id| *id != 0));
    let pending = CodexApprovalNotification {
        connection_generation: 1,
        request_revision: 1,
        thread_id: Some(ThreadId("thread".into())),
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("request".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext::default(),
        },
        presentation: ApprovalPresentation {
            requester: "Codex".into(),
            identity: RequesterIdentity::BackendReported,
            action: "Run a command".into(),
            scope: None,
            duration: None,
            warning: None,
            detail: None,
        },
        actionable: true,
        submitting: false,
        unconfirmed: false,
    };
    shell.sync_codex_approval_notifications(vec![(owner, pending.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, pending.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(owner, pending.clone(), CodexApprovalChoice::Decline)]
    );
    let revised = CodexApprovalNotification {
        request_revision: 2,
        ..pending.clone()
    };
    shell.sync_codex_approval_notifications(vec![(owner, revised.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, revised.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(owner, revised, CodexApprovalChoice::Decline)]
    );

    let with_decisions = |id: &str, decisions| CodexApprovalNotification {
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId(id.into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext {
                available_decisions: Some(decisions),
                ..Default::default()
            },
        },
        ..pending.clone()
    };
    let no_refusal = with_decisions("no-refusal", vec![CommandDecision::AcceptForSession]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    assert!(shell.take_codex_approval_decisions().is_empty());
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, no_refusal.clone(), false)]
    );
    shell.notification_feed.close_internal(fixture_ids[0]);
    shell.sync_codex_approval_notifications(vec![(owner, no_refusal.clone())]);
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, no_refusal, true)]
    );
    shell.sync_codex_approval_notifications(Vec::new());
    assert_eq!(
        shell.notification_feed.history().len(),
        crate::notification::MAX_NOTIFICATIONS - 1
    );
    assert_ne!(
        shell
            .notification_feed
            .notify_internal(crate::notification::NotificationRequest {
                app_name: "Persistent fixture".into(),
                summary: "Fixture".into(),
                body: String::new(),
                actions: Vec::new(),
                expire_timeout_ms: 0,
            }),
        0
    );
    let cancel = with_decisions("cancel-only", vec![CommandDecision::Cancel]);
    shell.sync_codex_approval_notifications(vec![(owner, cancel.clone())]);
    assert_eq!(
        shell.take_codex_approval_decisions(),
        vec![(
            owner,
            cancel,
            CodexApprovalChoice::Command(CommandDecision::Cancel),
        )]
    );
    let inactive = CodexApprovalNotification {
        actionable: false,
        interaction: PendingInteraction::Approval {
            request_id: ServerRequestId("disconnected".into()),
            approval_type: "item/commandExecution/requestApproval".into(),
            summary: "Run command".into(),
            context: ApprovalContext::default(),
        },
        ..pending
    };
    shell.sync_codex_approval_notifications(vec![(owner, inactive.clone())]);
    assert!(shell.take_codex_approval_decisions().is_empty());
    assert_eq!(
        shell.take_codex_approval_delivery_updates(),
        vec![(owner, inactive, false)]
    );
}

#[test]
fn in_process_system_feed_propagates_audio_network_and_bluetooth() {
    let mut shell = LiveShell::new().expect("live shell");
    let network = NetworkStatus {
        available: true,
        enabled: true,
        connected: true,
        name: "Nickel Lab".into(),
        signal_percent: 82,
        networks: Vec::new(),
    };
    let bluetooth = BluetoothStatus {
        available: true,
        powered: true,
        discovering: false,
        devices: Vec::new(),
    };
    let audio = AudioStatus {
        available: true,
        devices: Vec::new(),
        volume_percent: 64,
        muted: false,
    };

    assert!(shell.apply_system_status_update(SystemStatusUpdate::Network(network.clone())));
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Bluetooth(bluetooth.clone())));
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Audio(audio.clone())));
    assert_eq!(shell.network, network);
    assert_eq!(shell.bluetooth, bluetooth);
    assert_eq!(shell.audio, audio);
}

#[test]
fn unchanged_system_feed_events_are_idle_and_do_not_schedule_polling() {
    let mut shell = LiveShell::new().expect("live shell");
    let status = shell.audio.clone();
    let before = shell.next_host_deadline();

    assert!(!shell.apply_system_status_update(SystemStatusUpdate::Audio(status)));
    assert_eq!(shell.next_host_deadline(), before);
}

#[test]
fn launcher_focus_loss_dismisses_the_ephemeral_surface() {
    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_session_launcher_visibility(true);

    assert!(shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::Launcher));
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
    assert!(!shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::Launcher));
}

#[test]
fn control_center_focus_loss_dismisses_the_ephemeral_surface() {
    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_control_visibility(true);

    assert!(shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
}

#[test]
fn rejected_launcher_focus_request_does_not_project_internal_focus() {
    struct RejectingHost;

    impl crate::session_host::SessionHost for RejectingHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Err(crate::platform::SessionRequestError::Send)
        }
    }

    let mut shell = LiveShell::new_with_session_host(Arc::new(RejectingHost)).expect("live shell");

    assert!(!shell.request_launcher_toggle());
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
    assert!(
        shell
            .plugin_launcher_host
            .as_ref()
            .unwrap()
            .inspect()
            .keyboard_focus
            .is_none()
    );
}

#[test]
fn successful_launcher_retry_clears_transient_update_error() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct RecoveringHost {
        reject: AtomicBool,
    }

    impl crate::session_host::SessionHost for RecoveringHost {
        fn dispatch(
            &self,
            command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            if matches!(command, crate::platform::ShellCommand::ShowFromController)
                && self.reject.swap(false, Ordering::AcqRel)
            {
                Err(crate::platform::SessionRequestError::Send)
            } else {
                Ok(())
            }
        }
    }

    let mut shell = LiveShell::new_with_session_host(Arc::new(RecoveringHost {
        reject: AtomicBool::new(true),
    }))
    .expect("live shell");

    assert!(!shell.request_launcher_toggle());
    assert_eq!(
        shell.launcher_status.as_deref(),
        Some("Nickel could not update the launcher.")
    );

    assert!(shell.request_launcher_toggle());
    assert!(shell.launcher_status.is_none());
    assert!(shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
}

#[test]
fn pending_remote_lease_becomes_persistent_shell_notification() {
    use nickel_session_protocol::{
        RemoteLeaseRequest, RemoteLeaseRequestChanges, RemotePendingLease, RemoteResourceScope,
    };
    use std::sync::Mutex;

    struct PendingLeaseHost {
        pending: Mutex<Vec<RemotePendingLease>>,
        #[cfg(target_os = "linux")]
        decisions: Mutex<Vec<bool>>,
    }
    impl crate::session_host::SessionHost for PendingLeaseHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
        fn remote_pending_leases(&self) -> Vec<RemotePendingLease> {
            self.pending.lock().unwrap().clone()
        }
        fn decide_remote_lease(
            &self,
            _: &RemotePendingLease,
            allow: bool,
        ) -> Result<(), crate::platform::SessionRequestError> {
            #[cfg(target_os = "linux")]
            self.decisions.lock().unwrap().push(allow);
            #[cfg(not(target_os = "linux"))]
            let _ = allow;
            Ok(())
        }
    }

    let pending = RemotePendingLease {
        pending_generation: 4,
        client_id: "agent-1".into(),
        client_label: "Codex".into(),
        request: RemoteLeaseRequest {
            renewal: None,
            scope: RemoteResourceScope::FullSession,
            duration_seconds: Some(1_200),
            allow_resumption: false,
            full_debug: true,
        },
        resource_label: None,
        changes: RemoteLeaseRequestChanges::default(),
    };
    let host = Arc::new(PendingLeaseHost {
        pending: Mutex::new(vec![pending.clone()]),
        #[cfg(target_os = "linux")]
        decisions: Mutex::new(Vec::new()),
    });
    let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
    shell
        .set_plugin_enabled(&crate::plugin_panel::notification_manifest().id, false)
        .unwrap();

    shell.sync_remote_lease_notifications_from(host.pending.lock().unwrap().clone());

    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.summary, "Codex requests approval");
    for expected in [
        "Scope: the full desktop",
        "Duration: 20 minutes",
        "Requester label is self-reported",
        "Full Nickel debugging access is requested",
        "pointer and keyboard input",
        "Protected surfaces and clipboard or filesystem transfer are excluded",
    ] {
        assert!(notification.body.contains(expected), "{expected}");
    }
    assert_eq!(notification.actions[0].key, "deny");
    assert_eq!(notification.actions[1].key, "approve");
    assert_eq!(shell.remote_lease_notifications.len(), 1);

    let notification_id = notification.id;
    shell.notification = Some(notification);
    assert!(shell.surface_visible(crate::winit_shell::SurfaceRole::Notification));
    assert!(
        !shell
            .scene(crate::winit_shell::SurfaceRole::Notification, 420, 180)
            .is_empty()
    );
    shell.sync_notification_host(420, 180);
    let approve = shell
        .notification_host
        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Approve".into(),
        })
        .unwrap();
    let point = nickel_input::Point {
        x: f64::from(approve.bounds.origin.x + approve.bounds.size.width / 2.0),
        y: f64::from(approve.bounds.origin.y + approve.bounds.size.height / 2.0),
    };
    let event = |edge| {
        nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            device: nickel_input::DeviceId(1),
            order: nickel_input::EventOrder(1),
            position: Some(point),
            button: nickel_input::PointerButton::Primary,
            edge,
        })
    };
    shell.notification_host_input(event(nickel_input::KeyEdge::Pressed), 420, 180);
    shell.notification_host_input(event(nickel_input::KeyEdge::Released), 420, 180);
    #[cfg(target_os = "linux")]
    assert_eq!(*host.decisions.lock().unwrap(), vec![true]);
    #[cfg(target_os = "windows")]
    assert_eq!(
        shell.take_remote_lease_decisions(),
        vec![(pending.clone(), true)]
    );
    assert_eq!(
        shell.remote_lease_notifications.len(),
        1,
        "dispatch is not resolution"
    );
    assert!(shell.remote_lease_submitting.contains(&notification_id));
    let submitted = shell
        .notification_feed
        .history()
        .into_iter()
        .find(|item| item.id == notification_id)
        .unwrap();
    assert!(submitted.actions.is_empty());
    assert!(submitted.body.contains("unconfirmed"));
    shell.dismiss_notification_transport(notification_id);
    assert!(shell.notification.is_none());
    assert!(
        shell
            .notification_feed
            .history()
            .iter()
            .any(|item| item.id == notification_id)
    );
    shell.refresh_fast();
    assert!(
        shell.notification.is_none(),
        "dismissed pending request must not re-toast"
    );

    host.pending.lock().unwrap().clear();
    shell.sync_remote_lease_notifications_from(Vec::new());
    assert!(shell.remote_lease_notifications.is_empty());
    assert!(shell.remote_lease_submitting.is_empty());
    assert!(shell.dismissed_remote_lease_notifications.is_empty());
    assert!(shell.notification_feed.snapshot().is_none());

    let mut changed = pending.clone();
    changed.pending_generation = 5;
    changed.request.allow_resumption = true;
    changed.request.renewal = Some(nickel_session_protocol::RemoteLeaseRenewal {
        lease_id: 2,
        generation: 1,
    });
    changed.changes.access_changed = true;
    changed.changes.duration_increased = true;
    shell.sync_remote_lease_notifications_from(vec![changed]);
    let changed_body = shell.notification_feed.snapshot().unwrap().body;
    for warning in [
        "may resume after the client reconnects",
        "broadens access",
        "increases the duration",
        "renews an existing lease",
    ] {
        assert!(changed_body.contains(warning), "{warning}");
    }
    shell.sync_remote_lease_notifications_from(Vec::new());

    let mut oversized = pending;
    oversized.pending_generation = 5;
    oversized.client_label = "x".repeat(nickel_ui::approval::MAX_APPROVAL_PRESENTATION_BYTES);
    let mut overflow = oversized.clone();
    overflow.pending_generation = 6;
    shell.sync_remote_lease_notifications_from(vec![oversized]);
    let notification = shell.notification_feed.snapshot().unwrap();
    assert_eq!(notification.actions.len(), 1);
    assert_eq!(notification.actions[0].key, "deny");
    assert!(notification.body.contains("Approval is unavailable"));

    shell.sync_remote_lease_notifications_from(Vec::new());
    for _ in 0..crate::notification::MAX_NOTIFICATIONS {
        assert_ne!(
            shell
                .notification_feed
                .notify_internal(crate::notification::NotificationRequest {
                    app_name: "Persistent fixture".into(),
                    summary: "Fixture".into(),
                    body: String::new(),
                    actions: Vec::new(),
                    expire_timeout_ms: 0,
                }),
            0
        );
    }
    shell.sync_remote_lease_notifications_from(vec![overflow.clone()]);
    shell.sync_remote_lease_notifications_from(vec![overflow]);
    #[cfg(target_os = "linux")]
    assert_eq!(*host.decisions.lock().unwrap(), vec![true, false]);
    #[cfg(target_os = "windows")]
    assert_eq!(shell.take_remote_lease_decisions().len(), 1);
    assert!(shell.remote_lease_notifications.is_empty());
    assert_eq!(shell.remote_lease_overflow_rejections.len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn queued_keyboard_auto_show_does_not_recursively_query_the_unacknowledged_snapshot() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct PendingKeyboardHost(AtomicUsize);
    impl crate::session_host::SessionHost for PendingKeyboardHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
        fn keyboard_snapshot(
            &self,
        ) -> Result<
            nickel_session_protocol::OnScreenKeyboardSnapshot,
            crate::platform::SessionRequestError,
        > {
            Ok(nickel_session_protocol::OnScreenKeyboardSnapshot {
                enabled: true,
                auto_show_requested: true,
                epoch: 19,
                generation: 1,
                height: 320,
                recipient: Some(nickel_session_protocol::WindowId(7)),
                ..Default::default()
            })
        }
        fn configure_keyboard(
            &self,
            _: bool,
            _: bool,
            _: u64,
            _: bool,
            _: bool,
            _: u32,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    let host = Arc::new(PendingKeyboardHost(AtomicUsize::new(0)));
    let mut shell = LiveShell::new().unwrap();
    shell.session_host = host.clone();
    shell.keyboard_override = nickel_core::on_screen_keyboard::KeyboardOverride::Enabled;
    assert!(shell.refresh_keyboard());
    assert!(shell.keyboard_enabled);
    assert!(shell.keyboard_visible);
    assert!(
        host.0.load(Ordering::SeqCst) <= 2,
        "no recursive auto-show requests before authority acknowledgement"
    );
}

#[test]
fn coalesced_audio_feedback_uses_latest_state_and_suppresses_reconnect_only_changes() {
    let mut shell = LiveShell::new().unwrap();
    let status = |available, volume_percent, muted| AudioStatus {
        available,
        volume_percent,
        muted,
        devices: Vec::new(),
    };
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status(true, 31, false)));
    let (sender, receiver) = crate::platform::status_mailbox::channel();
    for snapshot in [status(true, 36, false), status(true, 31, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    let updates = receiver.drain();
    assert_eq!(updates.len(), 1);
    assert!(shell.apply_system_status_update(updates.into_iter().next().unwrap()));
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    shell.volume_osd_scene(320, 88);
    assert!(
        shell
            .volume_osd_projection()
            .label
            .starts_with("Volume 31%")
    );
    assert!(
        shell
            .plugin_volume_osd_host
            .as_ref()
            .unwrap()
            .accessibility_nodes()
            .iter()
            .any(|node| {
                node.semantic_role == Some(SemanticRole::Text)
                    && node
                        .label
                        .as_deref()
                        .is_some_and(|label| label.starts_with("Volume 31%"))
            })
    );
    // A hidden unavailable/available transition must not turn a different device's
    // initial volume into apparent user feedback.
    for snapshot in [status(false, 0, false), status(true, 50, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    for update in receiver.drain() {
        shell.apply_system_status_update(update);
    }
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    for snapshot in [status(true, 50, true), status(true, 50, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    for update in receiver.drain() {
        assert!(shell.apply_system_status_update(update));
    }
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    assert!(!shell.audio.muted);
}

#[test]
fn audio_feedback_ignores_startup_metadata_and_reconnect_but_shows_value_changes() {
    let mut shell = LiveShell::new().unwrap();
    let mut status = AudioStatus {
        available: true,
        volume_percent: 31,
        muted: false,
        devices: Vec::new(),
    };
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.devices.push(crate::platform::AudioDeviceStatus {
        id: "sink".into(),
        name: "Speaker".into(),
        is_default: true,
    });
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.volume_percent = 36;
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone())));
    assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
    shell.volume_osd_scene(320, 88);
    assert!(
        shell
            .volume_osd_projection()
            .label
            .starts_with("Volume 36%")
    );
    let first = shell.volume_osd_until.unwrap();
    status.muted = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(shell.volume_osd_until.unwrap() >= first);
    let outcome = shell.poll_deadlines(Instant::now() + Duration::from_secs(2));
    assert!(outcome.visibility_changed);
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    status.available = false;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    status.available = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
}

#[test]
fn settings_transition_reprojects_light_and_dark_appearance() {
    use nickel_core::{shell_settings::ThemePreference, theme::ThemePalette};

    let mut shell = LiveShell::new().expect("live shell");
    let mut settings = nickel_core::shell_settings::ShellSettings {
        theme: ThemePreference::Light,
        ..Default::default()
    };
    assert!(shell.apply_shell_settings(settings.clone()));
    let light = shell.palette;
    assert_eq!(
        light,
        ThemePalette::from_appearance(settings.resolve_appearance(Default::default()))
    );

    settings.theme = ThemePreference::Dark;
    assert!(shell.apply_shell_settings(settings.clone()));
    assert_ne!(shell.palette, light);
    assert_eq!(
        shell.palette,
        ThemePalette::from_appearance(settings.resolve_appearance(Default::default()))
    );
}

#[test]
fn injected_session_host_receives_shell_commands_without_platform_transport() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingHost(AtomicUsize);

    impl crate::session_host::SessionHost for RecordingHost {
        fn dispatch(
            &self,
            _command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<SecureStorageState, crate::platform::SessionRequestError> {
            Ok(SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
    }

    let host = Arc::new(RecordingHost(AtomicUsize::new(0)));
    let shell = LiveShell::new_with_session_host(host.clone()).expect("live shell");
    let startup_commands = host.0.load(Ordering::Relaxed);

    assert!(shell.dispatch_session_command(
        "test-direct-session-host",
        crate::platform::ShellCommand::CreateWorkspace,
    ));
    assert_eq!(host.0.load(Ordering::Relaxed), startup_commands + 1);
}

#[cfg(target_os = "linux")]
#[test]
fn active_window_screenshot_uses_in_process_capture_and_crops_before_copy() {
    use std::sync::Mutex;

    use crate::session_host::DesktopCapturePoll;
    use nickel_session_protocol::{
        Geometry, OutputSnapshot, OutputTransform, Snapshot, WindowId, WindowSnapshot, WorkspaceId,
    };

    #[derive(Default)]
    struct CaptureHost {
        outputs: Mutex<Vec<Option<String>>>,
        copied_sizes: Mutex<Vec<(u32, u32)>>,
    }

    impl crate::session_host::SessionHost for CaptureHost {
        fn dispatch(
            &self,
            _: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }

        fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
            self.outputs.lock().unwrap().push(output.map(str::to_owned));
            DesktopCapturePoll::Ready(Ok(crate::platform::DesktopCapture {
                image: RgbaImage::new(200, 100),
            }))
        }

        fn copy_image(&self, image: RgbaImage) -> Result<(), String> {
            self.copied_sizes.lock().unwrap().push(image.dimensions());
            Ok(())
        }
    }

    let host = Arc::new(CaptureHost::default());
    let mut shell = LiveShell::new_with_session_host(host.clone()).expect("live shell");
    shell.apply_internal_session_snapshot(Snapshot {
        outputs: vec![OutputSnapshot {
            name: "DP-1".into(),
            model: "Test".into(),
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            work_area: Geometry {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_120: 120,
            transform: OutputTransform::Normal,
            physical_width_mm: 1,
            physical_height_mm: 1,
            primary: true,
            enabled: true,
            modes: Vec::new(),
            current_mode: None,
        }],
        windows: vec![WindowSnapshot {
            id: WindowId(7),
            application_id: "test.app".into(),
            title: "Test".into(),
            active: true,
            minimized: false,
            maximized: false,
            fullscreen: false,
            geometry: Some(Geometry {
                x: 10,
                y: 5,
                width: 30,
                height: 20,
            }),
            workspace: WorkspaceId(1),
        }],
        focused: Some(WindowId(7)),
        ..Snapshot::default()
    });

    assert!(shell.global_shortcut(GlobalShortcut::Screenshot(
        crate::platform::ScreenshotAction::ActiveWindow
    )));
    assert!(!shell.capture_screenshot());
    assert_eq!(
        host.outputs.lock().unwrap().as_slice(),
        &[Some("DP-1".into())]
    );
    assert_eq!(host.copied_sizes.lock().unwrap().as_slice(), &[(60, 40)]);
    assert!(!shell.screenshot.visible());
}

#[test]
fn compositor_owned_shell_scenario_routes_focus_switching_and_files_without_transport() {
    use std::{path::PathBuf, sync::Mutex};

    use nickel_core::{
        hotkeys::HotkeyAction,
        task_switcher::{SwitchWindow, TaskSwitcher},
    };
    use nickel_file::{FileLaunch, FileWindowRequest};

    #[derive(Default)]
    struct RecordingHost(Mutex<Vec<crate::platform::ShellCommand>>);

    impl crate::session_host::SessionHost for RecordingHost {
        fn dispatch(
            &self,
            command: crate::platform::ShellCommand,
        ) -> Result<(), crate::platform::SessionRequestError> {
            self.0.lock().unwrap().push(command);
            Ok(())
        }

        fn secure_storage_state(
            &self,
        ) -> Result<crate::platform::SecureStorageState, crate::platform::SessionRequestError>
        {
            Ok(crate::platform::SecureStorageState::Ready)
        }

        fn request_secure_storage_retry(&self) -> Result<(), crate::platform::SessionRequestError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingFiles(Mutex<Vec<FileWindowRequest>>);

    impl crate::file_window_host::FileWindowHost for RecordingFiles {
        fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
            self.0.lock().unwrap().push(request);
            Ok(())
        }
    }

    let session = Arc::new(RecordingHost::default());
    let files = Arc::new(RecordingFiles::default());
    let mut shell = LiveShell::new_with_hosts(session.clone(), files.clone()).expect("live shell");

    shell.apply_session_launcher_visibility(true);
    assert!(
        shell
            .plugin_launcher_host
            .as_ref()
            .unwrap()
            .inspect()
            .keyboard_focus
            .is_some()
    );

    shell.windows = vec![
        OpenWindow {
            id: WindowId(10),
            application_id: Some(ApplicationId::new("org.nickel.One")),
            active: true,
            title: "One".into(),
            state: crate::model::WindowState::default(),
        },
        OpenWindow {
            id: WindowId(20),
            application_id: Some(ApplicationId::new("org.nickel.Two")),
            active: false,
            title: "Two".into(),
            state: crate::model::WindowState::default(),
        },
    ];
    let switch_windows = shell
        .windows
        .iter()
        .map(|window| SwitchWindow {
            id: window.id,
            application_id: window.application_id.as_ref().unwrap().as_str().to_owned(),
            active: window.active,
        })
        .collect::<Vec<_>>();
    shell.task_switcher = TaskSwitcher::default();
    assert!(
        !shell
            .task_switcher
            .apply(HotkeyAction::SwitchNext, &switch_windows)
            .is_empty()
    );
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::SwitchNext));
    let preview_role = crate::winit_shell::SurfaceRole::WindowPreview;
    let _ = shell.scene(preview_role, 640, 240);
    let first_preview_token = shell
        .scene_change_token(preview_role)
        .expect("task switcher preview token");
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::SwitchNext));
    assert!(
        shell.plugin_preview_host.is_some(),
        "consecutive switch steps must retain the JSX preview host so its presentation token advances"
    );
    let _ = shell.scene(preview_role, 640, 240);
    assert_ne!(
        shell.scene_change_token(preview_role),
        Some(first_preview_token),
        "each visible task-switch selection must receive a distinct presentation token"
    );
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::CommitSwitch));
    assert!(session.0.lock().unwrap().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));

    // The Windows shortcut adapter forwards cancellation through this same
    // production shell action without committing the highlighted candidate.
    shell.task_switcher = TaskSwitcher::default();
    shell
        .task_switcher
        .apply(HotkeyAction::SwitchNext, &switch_windows);
    session.0.lock().unwrap().clear();
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::CancelSwitch));
    assert!(shell.task_switcher.session().is_none());
    assert!(!session.0.lock().unwrap().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));

    // Session actions share this platform-neutral shell path on Windows and
    // Linux. They must retire a pending switch (and its delayed peek) before
    // the platform begins locking, logging out, suspending, or restarting.
    shell.task_switcher = TaskSwitcher::default();
    shell
        .task_switcher
        .apply(HotkeyAction::SwitchNext, &switch_windows);
    session.0.lock().unwrap().clear();
    shell.apply_control_action(ControlAction::SessionAction(
        crate::platform::SessionAction::Lock,
    ));
    assert!(shell.task_switcher.session().is_none());
    let commands = session.0.lock().unwrap();
    assert!(commands.iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::SessionAction(crate::platform::SessionAction::Lock)
    )));
    assert!(!commands.iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));
    drop(commands);

    let path = PathBuf::from("/tmp/internal-file-scenario");
    shell.launch_application(crate::model::Application::new(
        "place:test".into(),
        "Test location".into(),
        None,
        None,
        Some(vec!["nickel-file".into(), path.display().to_string()]),
    ));
    assert_eq!(
        files.0.lock().unwrap().as_slice(),
        [FileWindowRequest::OpenOrFocus(FileLaunch::Browse(path))]
    );
}
