fn ordinary_package_runtime(
    shell: &LiveShell,
    id: &str,
) -> std::rc::Rc<std::cell::RefCell<nickel_plugin_runtime::JsxRuntime>> {
    match &shell.package_runtimes[id] {
        crate::live_shell::RetainedPackageRuntime::Ordinary(runtime) => runtime.clone(),
        crate::live_shell::RetainedPackageRuntime::Composed(_) => {
            panic!("ordinary package unexpectedly uses composition")
        }
    }
}

// Boa package evaluation needs the same stack reserve as the shipped host.
fn with_package_runtime_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn native_wallpaper_chooser_completion_handles_cancel_failure_and_retired_authority() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        let plugin_id = "org.example.wallpaper-dialog".to_owned();
        let manifest: nickel_core::plugins::PluginManifest = serde_json::from_str(r#"{"api_version":1,"id":"org.example.wallpaper-dialog","name":"Wallpaper dialog","entry":"main.js","capabilities":["wallpaper-read","wallpaper-control"],"surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#).unwrap();
        shell.plugin_registry.register(manifest).unwrap();
        shell.plugin_registry.set_enabled(&plugin_id, true).unwrap();
        let identity = shell.wallpaper_chooser_identity(&plugin_id).unwrap();
        let effect = crate::appearance_capabilities::AppearanceEffect::ChooseImage {
            generation: 1,
            prior: crate::wallpaper_service::Preferences {
                custom_image_configured: false,
                position: crate::wallpaper_service::Position::Fill,
            },
        };
        for (outcome, locked, retire, expected) in [
            (
                nickel_platform::FileDialogOutcome::Cancelled,
                false,
                false,
                "cancelled",
            ),
            (
                nickel_platform::FileDialogOutcome::Failed("/private/dialog/path".into()),
                false,
                false,
                "failed",
            ),
            (
                nickel_platform::FileDialogOutcome::Selected("/private/image.png".into()),
                true,
                false,
                "rejected",
            ),
            (
                nickel_platform::FileDialogOutcome::Selected("/private/image.png".into()),
                false,
                true,
                "rejected",
            ),
        ] {
            let (sender, receiver) = std::sync::mpsc::channel();
            shell.wallpaper_chooser = Some(super::WallpaperChooserRequest {
                plugin_id: plugin_id.clone(),
                identity: identity.clone(),
                activation: shell.plugin_activation_generation,
                effect: effect.clone(),
                receiver,
            });
            assert!(!shell.poll_wallpaper_chooser());
            shell.locked = locked;
            if retire {
                shell.plugin_activation_generation += 1;
            }
            sender.send(outcome).unwrap();
            assert!(shell.poll_wallpaper_chooser());
            assert!(shell.wallpaper_chooser.is_none());
            let result = &shell.wallpaper_chooser_results[&plugin_id];
            assert_eq!(result["status"], expected);
            assert!(!result.to_string().contains("/private"));
            shell.locked = false;
        }
    });
}

#[test]
fn embedded_package_visibility_preserves_shared_runtime_until_disable() {
    with_package_runtime_stack(|| {
        use nickel_core::plugins::{
            PluginHealth, PluginPackage, PluginPackageSource, PluginSurfaceKey,
        };
        let manifest = br#"{"api_version":1,"id":"org.example.embedded-lifecycle","name":"Embedded lifecycle","entry":"src/main.js","capabilities":["settings-write"],"surfaces":[{"id":"first","kind":"window","width":400,"height":240},{"id":"second","kind":"window","width":400,"height":240,"initially_open":false}]}"#;
        let source = b"import { shared } from './state.js';\nregisterSetting({id:'enabled',group:'Example',label:'Enabled',type:'switch',defaultValue:false,onChange:()=>{shared.count+=10;}});\nexport default function App() { const id=nickel.data.surface.id; return h(Window,{id,width:400,height:240},h(Text,{},String(shared.count)),h(Button,{onClick:()=>{shared.count++;}},'Increment')); }";
        let package = PluginPackage::from_embedded(&[
            ("plugin.json", manifest),
            ("src/main.js", source),
            ("src/state.js", b"export const shared = {count:0};"),
        ])
        .unwrap();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), PluginPackageSource::embedded(package));
        shell.set_plugin_enabled(&id, true).unwrap();
        let runtime = ordinary_package_runtime(&shell, &id);
        let first = PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: "first".into(),
        };
        let second = PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: "second".into(),
        };
        assert!(shell.plugin_surface_hosts.contains_key(&first));
        assert!(!shell.plugin_surface_hosts.contains_key(&second));
        shell.plugin_panel_scene(&first, 400, 240).unwrap();
        let button = shell
            .plugin_panel_host_for(&first)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Increment".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &first,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            400,
            240
        ));
        assert!(shell.close_plugin_window(&first).unwrap());
        assert!(
            shell
                .plugin_surface_hosts
                .keys()
                .all(|key| key.plugin_id != id)
        );
        assert_eq!(
            shell.plugin_registry.get(&id).unwrap().health,
            PluginHealth::Running
        );
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &ordinary_package_runtime(&shell, &id)
        ));
        assert!(
            shell
                .package_settings_registry
                .settings_snapshot()
                .settings
                .iter()
                .any(|setting| setting.provider_package == id)
        );
        shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::InvokeRegisteredSetting {
                caller: id.clone(),
                provider: id.clone(),
                id: "enabled".into(),
                value: serde_json::Value::Bool(true),
            },
        ]);
        assert!(shell.show_plugin_window(&id, "second").unwrap());
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &shell
                .plugin_panel_host_for(&second)
                .unwrap()
                .application()
                .shared_runtime()
        ));
        let count: u32 = runtime
            .borrow_mut()
            .eval_json("JSON.stringify(__nickelRequireModule('src/state.js').shared.count)")
            .unwrap();
        assert_eq!(count, 11);
        shell.show_plugin_window(&id, "first").unwrap();
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &shell
                .plugin_panel_host_for(&first)
                .unwrap()
                .application()
                .shared_runtime()
        ));
        shell.set_plugin_enabled(&id, false).unwrap();
        assert!(!shell.package_runtimes.contains_key(&id));
        assert!(
            shell
                .package_settings_registry
                .settings_snapshot()
                .settings
                .iter()
                .all(|setting| setting.provider_package != id)
        );
        assert!(shell.show_plugin_window(&id, "second").is_err());
        shell.set_plugin_enabled(&id, true).unwrap();
        assert!(!std::rc::Rc::ptr_eq(
            &runtime,
            &ordinary_package_runtime(&shell, &id)
        ));
        let count: u32 = ordinary_package_runtime(&shell, &id)
            .borrow_mut()
            .eval_json("JSON.stringify(__nickelRequireModule('src/state.js').shared.count)")
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn embedded_package_can_enable_with_all_windows_initially_closed() {
    with_package_runtime_stack(|| {
        let manifest=br#"{"api_version":1,"id":"org.example.hidden-package","name":"Hidden package","entry":"main.js","surfaces":[{"id":"first","kind":"window","width":400,"height":240,"initially_open":false},{"id":"second","kind":"window","width":400,"height":240,"initially_open":false}]}"#;
        let source=b"registerSetting({id:'flag',group:'Hidden',label:'Flag',type:'switch',defaultValue:false}); function App() { return h(Window,{id:nickel.data.surface.id,width:400,height:240},h(Text,{},'Hidden package')); }";
        let package = nickel_core::plugins::PluginPackage::from_embedded(&[
            ("plugin.json", manifest),
            ("main.js", source),
        ])
        .unwrap();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell.external_plugin_packages.insert(
            id.clone(),
            nickel_core::plugins::PluginPackageSource::embedded(package),
        );
        shell.set_plugin_enabled(&id, true).unwrap();
        let runtime = ordinary_package_runtime(&shell, &id);
        assert!(
            shell
                .plugin_surface_hosts
                .keys()
                .all(|key| key.plugin_id != id)
        );
        assert!(
            shell
                .package_settings_registry
                .settings_snapshot()
                .settings
                .iter()
                .any(|setting| setting.provider_package == id)
        );
        shell.show_plugin_window(&id, "second").unwrap();
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: "second".into(),
        };
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &shell
                .plugin_panel_host_for(&key)
                .unwrap()
                .application()
                .shared_runtime()
        ));
        shell.set_plugin_enabled(&id, false).unwrap();
        assert!(!shell.package_runtimes.contains_key(&id));
    });
}

#[test]
fn stock_shell_package_handles_semantic_taskbar_input_and_global_surface_intents() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        let taskbar = LiveShell::default_shell_surface_key("taskbar");
        assert_eq!(
            shell.plugin_registry.get("nickel-default").unwrap().health,
            nickel_core::plugins::PluginHealth::Running
        );
        assert_eq!(shell.taskbar_reservation_height(), 56);
        let panels = shell.shell_panel_surfaces();
        assert_eq!(panels.iter().filter(|(key, _)| key == &taskbar).count(), 1);
        let surface = &panels.iter().find(|(key, _)| key == &taskbar).unwrap().1;
        assert!(surface.reserve_work_area);
        assert_eq!(surface.output, nickel_core::plugins::PluginOutputScope::All);
        assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Taskbar));
        let runtime = shell
            .plugin_panel_host_for(&taskbar)
            .unwrap()
            .application()
            .shared_composition_runtime()
            .unwrap();
        shell.plugin_panel_scene(&taskbar, 1280, 56).unwrap();
        let button = shell
            .plugin_panel_host_for(&taskbar)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open Nickel Start".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &taskbar,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            1280,
            56
        ));
        assert!(shell.launcher_intent_visible());
        assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::Launcher));
        let launcher = LiveShell::default_shell_surface_key("launcher");
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &shell
                .plugin_panel_host_for(&launcher)
                .unwrap()
                .application()
                .shared_composition_runtime()
                .unwrap()
        ));
        shell.global_shortcut(crate::platform::GlobalShortcut::HideLauncher);
        assert!(!shell.launcher_intent_visible());
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowLauncher);
        assert!(shell.launcher_intent_visible());
        shell.global_shortcut(crate::platform::GlobalShortcut::ToggleLauncher);
        assert!(!shell.launcher_intent_visible());
        shell.global_shortcut(crate::platform::GlobalShortcut::OpenSettings);
        assert!(shell.default_shell_surface_visible("settings"));
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowControlCenter);
        assert!(shell.default_shell_surface_visible("quick-settings"));
        assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::ControlCenter));
        for surface in ["settings", "quick-settings"] {
            let key = LiveShell::default_shell_surface_key(surface);
            assert!(std::rc::Rc::ptr_eq(
                &runtime,
                &shell
                    .plugin_panel_host_for(&key)
                    .unwrap()
                    .application()
                    .shared_composition_runtime()
                    .unwrap()
            ));
        }
        shell.set_plugin_enabled("nickel-default", false).unwrap();
        assert_eq!(shell.taskbar_reservation_height(), 0);
        assert!(!shell.can_show_launcher());
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowLauncher);
        assert!(!shell.launcher_intent_visible());
    });
}

#[test]
fn zero_surface_composition_provider_runs_once_and_rejoins_the_active_shell() {
    with_package_runtime_stack(|| {
        let root = tempfile::tempdir().unwrap();
        let id = "org.example.independent-provider";
        let directory = root.path().join(id);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("plugin.json"),r#"{"api_version":1,"id":"org.example.independent-provider","version":"0.2.0","name":"Independent provider","entry":"main.js","capabilities":["settings-write","windows-focus"],"composition":{"api_version":1,"id":"org.example.independent-provider","version":"0.2.0","contributions":[{"collection":"taskbar.items","id":"item","implementation":"./main.js#Item"},{"collection":"settings.pages","id":"page","implementation":"./main.js#Page"}]}}"#).unwrap();
        std::fs::write(directory.join("main.js"),"globalThis.starts=(globalThis.starts||0)+1; registerSetting({id:'enabled',group:'Example',label:'Enabled',type:'switch',defaultValue:false,value:()=>false});\nexport function Item(){return h(Button,{onClick:()=>nickel.windows.activate('native-window')},'Independent item');}\nexport function Page(){const [count,setCount]=useState(0);return h(Column,null,h(Text,null,'Provider activations '+count),h(Button,{onClick:()=>{setCount(count+1);nickel.windows.activate('71');}},'Activate provider window'),h(Button,{onClick:()=>{setCount(count+100);nickel.windows.close('71');}},'Denied provider close'));} registerSettingsPage({id:'page',group:'Example',label:'Independent',component:Page});").unwrap();
        let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
        let descriptor = catalog.packages.remove(id).unwrap();
        let session = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(session.clone()).unwrap();
        shell.windows = vec![crate::model::OpenWindow {
            id: crate::model::WindowId(71),
            application_id: None,
            active: true,
            title: "Provider target".into(),
            state: crate::model::WindowState::default(),
        }];
        shell
            .plugin_registry
            .register(descriptor.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.into(), descriptor.into());
        shell.set_plugin_enabled(id, true).unwrap();
        assert_eq!(
            shell.plugin_registry.get(id).unwrap().health,
            nickel_core::plugins::PluginHealth::Running
        );
        assert!(
            !shell
                .plugin_surface_hosts
                .keys()
                .any(|key| key.plugin_id == id)
        );
        let crate::live_shell::RetainedPackageRuntime::Composed(provider) =
            shell.package_runtimes[id].clone()
        else {
            panic!("provider context was not retained")
        };
        let owner = provider.borrow().resolution().active.clone();
        let runtime = provider.borrow().shared_owner_runtime(&owner).unwrap();
        let crate::live_shell::RetainedPackageRuntime::Composed(active) =
            shell.package_runtimes["nickel-default"].clone()
        else {
            panic!("active shell is not composed")
        };
        assert!(std::rc::Rc::ptr_eq(
            &runtime,
            &active.borrow().shared_owner_runtime(&owner).unwrap()
        ));
        let active_owner = active.borrow().resolution().active.clone();
        assert!(std::rc::Rc::ptr_eq(
            &active.borrow().shared_owner_runtime(&active_owner).unwrap(),
            &provider
                .borrow()
                .shared_owner_runtime(&active_owner)
                .unwrap()
        ));
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u32>("JSON.stringify(globalThis.starts)")
                .unwrap(),
            1
        );
        assert!(
            shell
                .package_settings_registry
                .settings_pages_snapshot()
                .pages
                .iter()
                .any(|page| page.provider_package == id)
        );
        // Select the registry page through the shipped Settings navigation, not its contribution alias.
        shell.launch_settings(None);
        let settings = shell.active_shell_surface_key("settings");
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        let selector = |role, name: &str| nickel_ui::SemanticSelector::RoleAndName {
            role,
            name: name.into(),
        };
        let destination = shell
            .plugin_panel_host_for(&settings)
            .unwrap()
            .query_unique(&selector(nickel_ui::SemanticRole::Button, "Independent"))
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &settings,
            nickel_ui::UiEvent::AccessibilityActivate(destination.id),
            1100,
            800
        ));
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        let button = shell
            .plugin_panel_host_for(&settings)
            .unwrap()
            .query_unique(&selector(
                nickel_ui::SemanticRole::Button,
                "Activate provider window",
            ))
            .unwrap();
        session.take_commands();
        assert!(shell.plugin_panel_host_ui_for(
            &settings,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            1100,
            800
        ));
        assert!(matches!(
            session.take_commands().as_slice(),
            [crate::platform::ShellCommand::WindowAction {
                window: crate::model::WindowId(71),
                action: crate::platform::WindowAction::Activate
            }]
        ));
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        // The shell has windows-context, but the source provider has only windows-focus.
        let context = nickel_core::plugins::PluginCapability::WindowsContext;
        assert!(
            shell
                .plugin_registry
                .get("nickel-default")
                .unwrap()
                .manifest
                .capabilities
                .contains(&context)
        );
        assert!(
            !shell
                .plugin_registry
                .get(id)
                .unwrap()
                .manifest
                .capabilities
                .contains(&context)
        );
        let denied = shell
            .plugin_panel_host_for(&settings)
            .unwrap()
            .query_unique(&selector(
                nickel_ui::SemanticRole::Button,
                "Denied provider close",
            ))
            .unwrap();
        shell.plugin_panel_host_ui_for(
            &settings,
            nickel_ui::UiEvent::AccessibilityActivate(denied.id),
            1100,
            800,
        );
        assert!(session.take_commands().is_empty());
        assert!(
            shell
                .plugin_panel_host_for(&settings)
                .unwrap()
                .application()
                .last_error()
                .is_some()
        );
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        assert!(
            shell
                .plugin_panel_host_for(&settings)
                .unwrap()
                .query_unique(&selector(
                    nickel_ui::SemanticRole::Text,
                    "Provider activations 1"
                ))
                .is_ok()
        );
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u32>("JSON.stringify(globalThis.starts)")
                .unwrap(),
            1
        );
        let stale = active
            .borrow()
            .contributions("taskbar.items")
            .into_iter()
            .find(|reference| reference.owner().id == id)
            .unwrap();
        shell.set_plugin_enabled(id, false).unwrap();
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        assert!(
            shell
                .package_settings_registry
                .settings_pages_snapshot()
                .pages
                .iter()
                .all(|page| page.provider_package != id)
        );
        let settings_host = shell.plugin_panel_host_for(&settings).unwrap();
        assert!(
            settings_host
                .query_unique(&selector(nickel_ui::SemanticRole::Button, "Independent"))
                .is_err()
        );
        assert!(
            settings_host
                .query_unique(&selector(
                    nickel_ui::SemanticRole::Button,
                    "Activate provider window"
                ))
                .is_err()
        );
        assert!(active.borrow_mut().mount(&stale).is_err());
        assert!(
            active
                .borrow()
                .contributions("taskbar.items")
                .iter()
                .all(|reference| reference.owner().id != id)
        );
        shell.set_plugin_enabled(id, true).unwrap();
        assert!(active.borrow_mut().mount(&stale).is_err());
        assert!(
            active
                .borrow()
                .contributions("settings.pages")
                .iter()
                .any(|reference| reference.owner().id == id)
        );
        let crate::live_shell::RetainedPackageRuntime::Composed(restarted) =
            shell.package_runtimes[id].clone()
        else {
            panic!("provider context was not restarted")
        };
        assert!(!std::rc::Rc::ptr_eq(
            &runtime,
            &restarted.borrow().shared_owner_runtime(&owner).unwrap()
        ));
    });
}

#[test]
fn installed_package_settings_follow_activation_and_retirement() {
    let root = tempfile::tempdir().unwrap();
    let id = "org.example.registered-settings";
    let directory = root.path().join(id);
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("plugin.json"), r#"{"api_version":1,"id":"org.example.registered-settings","name":"Settings provider","entry":"main.js","capabilities":["settings-write","launcher-show"],"surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#).unwrap();
    std::fs::write(directory.join("main.js"), "registerSetting({id:'enabled',group:'Example',label:'Enabled',type:'switch',defaultValue:false,onChange:value=>nickel.request('show-launcher')}); function App() { return h(Window,{id:'main',width:400,height:240},h(Text,{},'Provider')); }").unwrap();
    let mut catalog = nickel_core::plugins::PluginCatalog::discover(root.path()).unwrap();
    let descriptor = catalog.packages.remove(id).unwrap();
    let mut shell = LiveShell::new().unwrap();
    shell
        .plugin_registry
        .register(descriptor.manifest.clone())
        .unwrap();
    shell
        .external_plugin_packages
        .insert(id.into(), descriptor.into());
    shell.set_plugin_enabled(id, true).unwrap();
    let snapshot = shell.package_settings_registry.settings_snapshot();
    assert!(
        snapshot
            .settings
            .iter()
            .any(|setting| setting.provider_package == id && setting.registration.id == "enabled")
    );
    shell.refresh_package_settings();
    assert_eq!(
        shell
            .package_settings_registry
            .settings_snapshot()
            .generation,
        snapshot.generation
    );
    let invoke = |value| crate::plugin_panel::PluginEffect::InvokeRegisteredSetting {
        caller: id.into(),
        provider: id.into(),
        id: "enabled".into(),
        value,
    };
    shell.apply_plugin_effects(vec![invoke(serde_json::json!("bad"))]);
    assert!(!shell.launcher_visible);
    shell.apply_plugin_effects(vec![invoke(serde_json::json!(true))]);
    assert!(shell.launcher_visible);
    shell.set_plugin_enabled(id, false).unwrap();
    assert!(
        !shell
            .package_settings_registry
            .settings_snapshot()
            .settings
            .iter()
            .any(|setting| setting.provider_package == id)
    );
    shell.set_plugin_enabled(id, true).unwrap();
    assert_eq!(
        shell
            .package_settings_registry
            .settings_snapshot()
            .settings
            .iter()
            .filter(|setting| setting.provider_package == id)
            .count(),
        1
    );
}

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::platform::NotificationSource;
use image::{Rgba, RgbaImage};
use nickel_input::KeyCode;
use nickel_ui::{
    ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
    HostTelemetry, InputModality, Point, Rect, SemanticAction, SemanticRole, SemanticSelector,
    SemanticValueInput, SemanticValueSnapshot, UiEvent, UiHost, ViewContext,
};
use nickel_ui_testkit::{Scenario, Selector};

use super::{
    HostRuntimeSamples, LiveShell, desktop_label_foreground, initial_wallpaper, panel_tray_icons,
    platform::{
        AudioStatus, BluetoothStatus, GlobalShortcut, NetworkStatus, SecureStorageState,
        SystemStatusUpdate,
    },
    preview_refresh_due, retain_unchanged_desktop_icons, shortcut_capability_status,
    window_belongs_to_panel,
};

#[cfg(target_os = "linux")]
#[test]
fn plugin_display_preview_rejects_stale_topology_and_rolls_back_on_deadline() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            use crate::session_host::SessionHost;
            use std::sync::Mutex;

            struct DisplayHost {
                outputs: Mutex<Vec<nickel_session_protocol::OutputSnapshot>>,
                applied: Mutex<Vec<nickel_session_protocol::OutputLayout>>,
            }
            impl crate::session_host::SessionHost for DisplayHost {
                fn dispatch(
                    &self,
                    command: crate::platform::ShellCommand,
                ) -> Result<(), crate::platform::SessionRequestError> {
                    if let crate::platform::ShellCommand::ApplyOutputs(layout) = command {
                        let mut outputs = self.outputs.lock().unwrap();
                        for placement in &layout.placements {
                            let output = outputs
                                .iter_mut()
                                .find(|output| output.name == placement.name)
                                .unwrap();
                            output.geometry.x = placement.x;
                            output.geometry.y = placement.y;
                            output.enabled = placement.enabled;
                            output.primary = placement.name == layout.primary;
                            output.scale_120 = placement.scale_120;
                            if let Some(mode) = placement.mode {
                                output.current_mode = Some(mode);
                            }
                            if let Some(transform) = placement.transform {
                                output.transform = transform;
                            }
                        }
                        self.applied.lock().unwrap().push(layout);
                    }
                    Ok(())
                }
                fn projection_outputs(
                    &self,
                ) -> Result<Vec<nickel_session_protocol::OutputSnapshot>, String> {
                    Ok(self.outputs.lock().unwrap().clone())
                }
            }
            let mode = nickel_session_protocol::OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihz: 60_000,
            };
            let outputs = ["DP-1", "HDMI-1"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| nickel_session_protocol::OutputSnapshot {
                    name: name.into(),
                    model: name.into(),
                    geometry: nickel_session_protocol::Geometry {
                        x: index as i32 * 1920,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                    work_area: nickel_session_protocol::Geometry {
                        x: index as i32 * 1920,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                    scale_120: 120,
                    transform: nickel_session_protocol::OutputTransform::Normal,
                    physical_width_mm: 500,
                    physical_height_mm: 300,
                    primary: index == 0,
                    enabled: true,
                    modes: vec![mode],
                    current_mode: Some(mode),
                })
                .collect();
            let host = Arc::new(DisplayHost {
                outputs: Mutex::new(outputs),
                applied: Mutex::new(Vec::new()),
            });
            let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
            let mut requested =
                super::output_layout_from_snapshot(&host.projection_outputs().unwrap());
            requested.placements[1].x = 0;
            requested.placements[1].y = 1080;
            requested.placements[1].transform =
                Some(nickel_session_protocol::OutputTransform::Rotate90);
            let revision =
                crate::display_capabilities::revision(&host.projection_outputs().unwrap());
            let mut stale = requested.clone();
            stale.placements[1].name = "disconnected".into();
            assert!(!shell.preview_plugin_display_layout("plugin-a".into(), stale, &revision));
            assert!(host.applied.lock().unwrap().is_empty());
            host.outputs.lock().unwrap()[1].transform =
                nickel_session_protocol::OutputTransform::Rotate180;
            assert!(!shell.preview_plugin_display_layout(
                "plugin-a".into(),
                requested.clone(),
                &revision
            ));
            host.outputs.lock().unwrap()[1].transform =
                nickel_session_protocol::OutputTransform::Normal;
            assert!(shell.preview_plugin_display_layout("plugin-a".into(), requested, &revision));
            assert!(!shell.confirm_plugin_display_layout("plugin-b"));
            host.outputs.lock().unwrap()[1].transform =
                nickel_session_protocol::OutputTransform::Rotate270;
            assert!(!shell.confirm_plugin_display_layout("plugin-a"));
            host.outputs.lock().unwrap()[1].transform =
                nickel_session_protocol::OutputTransform::Rotate90;
            assert_eq!(host.applied.lock().unwrap().len(), 1);
            shell.display_preview.as_mut().unwrap().deadline =
                Instant::now() - Duration::from_millis(1);
            shell.poll_deadlines(Instant::now());
            assert!(shell.display_preview.is_none());
            assert_eq!(host.applied.lock().unwrap().len(), 2);
            assert_eq!(host.outputs.lock().unwrap()[1].geometry.x, 1920);
            assert_eq!(
                host.outputs.lock().unwrap()[1].transform,
                nickel_session_protocol::OutputTransform::Normal
            );
            let mut requested =
                super::output_layout_from_snapshot(&host.projection_outputs().unwrap());
            requested.placements[1].x = 0;
            requested.placements[1].y = 1080;
            requested.placements[1].transform =
                Some(nickel_session_protocol::OutputTransform::Rotate90);
            let revision =
                crate::display_capabilities::revision(&host.projection_outputs().unwrap());
            assert!(shell.preview_plugin_display_layout("plugin-a".into(), requested, &revision));
            assert!(shell.confirm_plugin_display_layout("plugin-a"));
            assert!(shell.display_preview.is_none());
            shell.poll_deadlines(Instant::now() + Duration::from_secs(16));
            assert_eq!(host.applied.lock().unwrap().len(), 3);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn installed_autostart_excludes_safe_mode_and_the_already_started_default_shell() {
    assert!(super::should_auto_start_installed_plugin(
        "org.example.panel",
        true,
        false
    ));
    assert!(!super::should_auto_start_installed_plugin(
        "org.example.panel",
        true,
        true
    ));
    assert!(!super::should_auto_start_installed_plugin(
        "org.example.panel",
        false,
        false
    ));
    assert!(!super::should_auto_start_installed_plugin(
        "nickel-default",
        true,
        false
    ));
}

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");

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
        .insert(descriptor.manifest.id.clone(), descriptor.into());
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
        .insert(descriptor.manifest.id.clone(), descriptor.into());
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
        .insert(id.clone(), descriptor.into());
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
    assert!(!shell.notification_action_granted("nickel-default", notification_id));
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
        .insert(id.clone(), descriptor.into());
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
        .insert(id.clone(), descriptor.into());
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
            .insert(id.clone(), descriptor.into());
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
        .insert(descriptor.manifest.id.clone(), descriptor.into());

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

include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");

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
        ..Default::default()
    };
    let bluetooth = BluetoothStatus {
        available: true,
        powered: true,
        discovering: false,
        devices: Vec::new(),
        ..Default::default()
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
fn control_center_focus_loss_dismisses_the_ephemeral_surface() {
    let mut shell = LiveShell::new().expect("live shell");
    shell.apply_control_visibility(true);

    assert!(shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.surface_visible(crate::winit_shell::SurfaceRole::ControlCenter));
    assert!(!shell.dismiss_ephemeral_on_focus_loss(crate::winit_shell::SurfaceRole::ControlCenter));
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
    assert!(shell.default_shell_surface_visible("volume-osd"));
    shell.plugin_panel_scene(&shell.active_shell_surface_key("volume-osd"), 420, 96);
    assert_eq!(
        crate::audio_capabilities::snapshot(&shell.audio, shell.locked)["percent"],
        31
    );
    assert!(
        shell
            .plugin_panel_host_ref(&shell.active_shell_surface_key("volume-osd"))
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
    assert!(!shell.default_shell_surface_visible("volume-osd"));
    for snapshot in [status(true, 50, true), status(true, 50, false)] {
        sender
            .send(Arc::new(SystemStatusUpdate::Audio(snapshot)))
            .unwrap();
    }
    for update in receiver.drain() {
        assert!(shell.apply_system_status_update(update));
    }
    assert!(shell.default_shell_surface_visible("volume-osd"));
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
    assert!(!shell.default_shell_surface_visible("volume-osd"));
    status.devices.push(crate::platform::AudioDeviceStatus {
        id: "sink".into(),
        name: "Speaker".into(),
        is_default: true,
    });
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(!shell.default_shell_surface_visible("volume-osd"));
    status.volume_percent = 36;
    assert!(shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone())));
    assert!(shell.default_shell_surface_visible("volume-osd"));
    shell.plugin_panel_scene(&shell.active_shell_surface_key("volume-osd"), 420, 96);
    assert_eq!(
        crate::audio_capabilities::snapshot(&shell.audio, shell.locked)["percent"],
        36
    );
    let first = shell.volume_osd_until.unwrap();
    status.muted = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    assert!(shell.volume_osd_until.unwrap() >= first);
    let outcome = shell.poll_deadlines(Instant::now() + Duration::from_secs(2));
    assert!(outcome.visibility_changed);
    assert!(!shell.default_shell_surface_visible("volume-osd"));
    status.available = false;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status.clone()));
    status.available = true;
    shell.apply_system_status_update(SystemStatusUpdate::Audio(status));
    assert!(!shell.default_shell_surface_visible("volume-osd"));
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
        ThemePalette::from_appearance(
            settings.resolve_appearance(crate::appearance_capabilities::system_appearance())
        )
    );

    settings.theme = ThemePreference::Dark;
    assert!(shell.apply_shell_settings(settings.clone()));
    assert_ne!(shell.palette, light);
    assert_eq!(
        shell.palette,
        ThemePalette::from_appearance(
            settings.resolve_appearance(crate::appearance_capabilities::system_appearance())
        )
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
fn plugin_management_rechecks_grants_and_retires_disabled_package_resources() {
    with_package_runtime_stack(|| {
        use nickel_core::plugins::{PluginCapability, PluginPackage, PluginPackageSource};
        let package = PluginPackage::from_embedded(&[
            ("plugin.json", br#"{"api_version":1,"id":"org.example.management","name":"Management","entry":"main.js","capabilities":["plugins-read","plugins-control"],"surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#),
            ("main.js", b"export default function App() {return h(Window,{id:'main',width:400,height:240},h(Text,{},'Management'));}"),
        ]).unwrap();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), PluginPackageSource::embedded(package));
        shell.set_plugin_enabled(&id, true).unwrap();
        assert!(shell.package_runtimes.contains_key(&id));
        let request = crate::plugins_capabilities::PluginsEffect {
            id: id.clone(),
            enabled: false,
            revision: shell.plugin_activation_generation,
            prior_enabled: true,
        };
        shell.locked = true;
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Plugins {
            plugin_id: id.clone(),
            effect: request.clone(),
        }]);
        assert!(shell.plugin_registry.get(&id).unwrap().desired_enabled);
        shell.locked = false;
        let mut readonly = shell.plugin_registry.get(&id).unwrap().manifest.clone();
        readonly.id = "org.example.readonly-management".into();
        readonly
            .capabilities
            .retain(|capability| *capability != PluginCapability::PluginsControl);
        shell.plugin_registry.register(readonly.clone()).unwrap();
        shell
            .plugin_registry
            .set_enabled(&readonly.id, true)
            .unwrap();
        shell.plugin_registry.mark_running(&readonly.id).unwrap();
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Plugins {
            plugin_id: readonly.id.clone(),
            effect: request.clone(),
        }]);
        assert!(shell.plugin_registry.get(&id).unwrap().desired_enabled);
        let mut stale = request.clone();
        stale.revision = stale.revision.wrapping_sub(1);
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Plugins {
            plugin_id: id.clone(),
            effect: stale,
        }]);
        assert!(shell.plugin_registry.get(&id).unwrap().desired_enabled);
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Plugins {
            plugin_id: id.clone(),
            effect: request,
        }]);
        assert!(!shell.plugin_registry.get(&id).unwrap().desired_enabled);
        assert!(!shell.package_runtimes.contains_key(&id));
        assert!(
            !shell
                .plugin_surface_hosts
                .keys()
                .any(|key| key.plugin_id == id)
        );
        assert_eq!(shell.plugins_results[&id]["status"], "applied");
        let restore = crate::plugins_capabilities::PluginsEffect {
            id: id.clone(),
            enabled: true,
            revision: shell.plugin_activation_generation,
            prior_enabled: false,
        };
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Plugins {
            plugin_id: id.clone(),
            effect: restore,
        }]);
        assert!(!shell.plugin_registry.get(&id).unwrap().desired_enabled);
    });
}

#[test]
fn desktop_settings_requests_open_shared_surface_and_preserve_navigation_identity() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.desktop_host.application_mut().set_active_output(
            "DP-2".into(),
            nickel_file::desktop::Point::default(),
            1.0,
        );
        shell
            .desktop_host
            .application_mut()
            .open_background_context(None);
        nickel_ui::Application::update(
            shell.desktop_host.application_mut(),
            super::desktop::DesktopMessage::Command(
                super::desktop::DesktopCommand::DisplaySettings,
            ),
        );
        let outcome = shell.poll_deadlines(Instant::now());
        assert!(outcome.visibility_changed);
        assert!(shell.default_shell_surface_visible("settings"));
        assert_eq!(
            shell.settings_navigation.as_ref().unwrap()["destination"],
            "displays"
        );
        assert_eq!(
            shell.settings_navigation.as_ref().unwrap()["output"],
            "DP-2"
        );
        assert!(shell.settings_navigation.as_ref().unwrap()["revision"].is_string());
        assert!(
            !shell
                .plugin_registry
                .entries()
                .any(|entry| entry.manifest.id == "org.nickel.settings")
        );
    });
}

#[test]
fn public_plugin_metadata_setting_updates_existing_runtime_with_checked_prior_state() {
    with_package_runtime_stack(|| {
        use nickel_core::plugins::{PluginPackage, PluginPackageSource};
        let package=PluginPackage::from_embedded(&[
            ("plugin.json",br#"{"api_version":1,"id":"org.example.metadata-edit","name":"Metadata edit","entry":"main.js","capabilities":["plugins-read","plugins-control"],"settings":[{"id":"count","label":"Count","kind":"integer","default":2,"min":1,"max":4}],"surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#),
            ("main.js",b"export default function App(){return h(Window,{id:'main',width:400,height:240},h(Text,{},String(nickel.data.settings.count)));}"),
        ]).unwrap();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), PluginPackageSource::embedded(package));
        shell.set_plugin_enabled(&id, true).unwrap();
        let mut effect = crate::plugins_capabilities::PluginsSettingEffect {
            id: id.clone(),
            key: "count".into(),
            revision: shell.plugin_activation_generation,
            prior_value: serde_json::json!(2),
            value: serde_json::json!(3),
        };
        shell.locked = true;
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::PluginsSetting {
            plugin_id: id.clone(),
            effect: effect.clone(),
        }]);
        assert_eq!(
            shell.plugin_management(&id).unwrap()["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .find(|plugin| plugin["id"] == id)
                .unwrap()["settings"][0]["value"],
            2
        );
        shell.locked = false;
        effect.prior_value = serde_json::json!(1);
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::PluginsSetting {
            plugin_id: id.clone(),
            effect: effect.clone(),
        }]);
        assert_eq!(shell.plugins_results[&id]["status"], "rejected");
        effect.prior_value = serde_json::json!(2);
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::PluginsSetting {
            plugin_id: id.clone(),
            effect,
        }]);
        assert_eq!(shell.plugin_settings[&id]["count"], 3);
        assert_eq!(shell.plugins_results[&id]["status"], "applied");
        assert!(shell.package_runtimes.contains_key(&id));
    });
}

#[test]
fn inherited_shell_selection_hides_base_roots_and_honors_omitted_settings() {
    with_package_runtime_stack(|| {
        use nickel_core::plugins::{PluginPackage, PluginPackageSource, PluginSurfaceKey};
        let package = PluginPackage::from_embedded(&[
            ("plugin.json", br#"{"api_version":1,"id":"example-theme","name":"Theme","version":"1.0.0","entry":"main.js","composition":{"api_version":1,"id":"example-theme","version":"1.0.0","extends":"nickel-default","requires":{"nickel-default":"^0.2.0"},"replaces":{"shell":"./main.js#Shell"}},"surfaces":[{"id":"taskbar","kind":"panel","width":800,"height":42,"reserve_work_area":true},{"id":"launcher","kind":"window","width":400,"height":240,"initially_open":false}]}"#),
            ("main.js", b"export function Shell(){const s=nickel.data.surface;return h(s.id==='taskbar'?FixedWindow:Window,{id:s.id,width:s.id==='taskbar'?800:400,height:s.id==='taskbar'?42:240,edge:s.id==='taskbar'?'bottom':undefined},h(Text,{},'Theme'));}\nexport default Shell;")
        ]).unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell.external_plugin_packages.insert(
            "example-theme".into(),
            PluginPackageSource::embedded(package),
        );
        shell.set_plugin_enabled("example-theme", true).unwrap();
        assert!(
            !shell
                .plugin_surface_hosts
                .keys()
                .any(|key| key.plugin_id == "example-theme")
        );
        let inventory = shell.plugin_management("nickel-default").unwrap();
        let selection = crate::plugins_capabilities::ShellSelectionEffect::parse(
            &serde_json::json!({"id":"example-theme","revision":inventory["revision"]}),
        )
        .unwrap();
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ShellSelection {
                plugin_id: "nickel-default".into(),
                effect: selection
            }
        ]));
        assert_eq!(shell.plugins_results["nickel-default"]["status"], "preview");
        let token = shell.shell_selection_preview.as_ref().unwrap().token;
        assert!(
            shell
                .confirm_shell_preview(Some("nickel-default"), token)
                .unwrap()
        );
        assert!(shell.package_runtimes.contains_key("nickel-default"));
        assert!(
            !shell
                .plugin_surface_hosts
                .keys()
                .any(|key| key.plugin_id == "nickel-default")
        );
        assert_eq!(
            shell.taskbar_surface_key(),
            Some(PluginSurfaceKey {
                plugin_id: "example-theme".into(),
                surface_id: "taskbar".into()
            })
        );
        assert_eq!(shell.taskbar_reservation_height(), 42);
        assert!(!shell.global_shortcut(crate::platform::GlobalShortcut::ShowRun));
        assert!(!shell.plugin_surface_matches(&shell.active_shell_surface_key("run")));
        assert_eq!(
            shell.shell_surface_effect_owner("nickel-default"),
            "example-theme"
        );
        shell
            .show_plugin_window("example-theme", "launcher")
            .unwrap();
        assert!(shell.default_shell_surface_visible("launcher"));
        shell.global_shortcut(crate::platform::GlobalShortcut::OpenSettings);
        assert!(!shell.default_shell_surface_visible("settings"));
        assert!(
            shell
                .show_plugin_window("nickel-default", "settings")
                .is_err()
        );
        shell.set_plugin_enabled("example-theme", false).unwrap();
        assert!(!shell.can_show_launcher());
        assert!(shell.select_shell_package("example-theme").unwrap());
        assert_eq!(shell.taskbar_reservation_height(), 42);
        assert!(shell.select_shell_package("nickel-default").unwrap());
        assert_eq!(shell.taskbar_reservation_height(), 56);
        let inventory = shell.plugin_management("nickel-default").unwrap();
        let effect = crate::plugins_capabilities::ShellSelectionEffect::parse(
            &serde_json::json!({"id":"example-theme","revision":inventory["revision"]}),
        )
        .unwrap();
        assert!(effect.validate(&inventory).is_ok());
        let mut stale = inventory.clone();
        stale["revision"] = serde_json::json!("0");
        assert!(effect.validate(&stale).is_err());
    });
}

#[test]
fn public_notification_actions_recheck_feed_identity_grants_lock_and_provider_lifecycle() {
    let mut shell = LiveShell::new().unwrap();
    let id = shell
        .notification_feed
        .notify_internal(crate::notification::NotificationRequest {
            app_name: "Mail".into(),
            summary: "New mail".into(),
            body: "Body".into(),
            actions: vec![crate::notification::NotificationAction {
                key: "open".into(),
                label: "Open".into(),
            }],
            expire_timeout_ms: 0,
        });
    let plugin_id = "nickel-default";
    assert!(shell.notification_action_granted(plugin_id, id));
    assert!(!shell.apply_plugin_effects(vec![
        crate::plugin_panel::PluginEffect::InvokeNotification {
            plugin_id: plugin_id.into(),
            id,
            key: "stale".into()
        }
    ]));
    shell.locked = true;
    assert!(!shell.notification_action_granted(plugin_id, id));
    shell.locked = false;
    shell.plugin_registry.set_enabled(plugin_id, false).unwrap();
    assert!(!shell.notification_action_granted(plugin_id, id));
    shell.plugin_registry.set_enabled(plugin_id, true).unwrap();
    assert!(!shell.notification_action_granted(plugin_id, id));
    shell.plugin_registry.mark_running(plugin_id).unwrap();
    assert!(shell.apply_plugin_effects(vec![
        crate::plugin_panel::PluginEffect::DismissNotification {
            plugin_id: plugin_id.into(),
            id
        }
    ]));
    assert!(!shell.notification_action_granted(plugin_id, id));
}

#[test]
fn composed_shell_keeps_each_host_identity_when_launcher_opens_settings() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowLauncher);
        let launcher = LiveShell::default_shell_surface_key("launcher");
        let (width, height) = {
            let surface = &shell.plugin_surface_hosts[&launcher].0;
            (surface.width, surface.height)
        };
        shell.plugin_panel_scene(&launcher, width, height).unwrap();
        let button = shell
            .plugin_panel_host_for(&launcher)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Settings".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &launcher,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            width,
            height
        ));
        assert!(shell.default_shell_surface_visible("settings"));
        for id in ["taskbar", "settings", "launcher", "settings", "taskbar"] {
            let key = LiveShell::default_shell_surface_key(id);
            let (width, height) = {
                let surface = &shell.plugin_surface_hosts[&key].0;
                (surface.width, surface.height)
            };
            assert!(
                shell.plugin_panel_scene(&key, width, height).is_some(),
                "{id} scene disappeared"
            );
            assert!(
                shell
                    .plugin_panel_host_for(&key)
                    .unwrap()
                    .application()
                    .last_error()
                    .is_none()
            );
        }
        shell.close_plugin_window(&launcher).unwrap();
        let settings = LiveShell::default_shell_surface_key("settings");
        assert!(shell.plugin_panel_scene(&settings, 1100, 800).is_some());
        assert!(
            shell
                .plugin_panel_scene(&LiveShell::default_shell_surface_key("taskbar"), 1920, 56)
                .is_some()
        );
        assert_eq!(
            shell.plugin_registry.get("nickel-default").unwrap().health,
            nickel_core::plugins::PluginHealth::Running
        );
    });
}

#[test]
fn window_menu_hotkey_uses_selected_package_and_native_window_policy() {
    with_package_runtime_stack(|| {
        let session = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(session.clone()).unwrap();
        shell.windows = vec![crate::model::OpenWindow {
            id: crate::model::WindowId(71),
            application_id: None,
            active: true,
            title: "Editor".into(),
            state: crate::model::WindowState::default(),
        }];
        session.take_commands();
        assert!(shell.global_shortcut(crate::platform::GlobalShortcut::ShowWindowMenu));
        let key = shell.active_shell_surface_key("window-menu");
        assert!(shell.plugin_panel_scene(&key, 320, 400).is_some());
        let button = shell
            .plugin_panel_host_for(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Minimize".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &key,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            320,
            400
        ));
        assert!(!shell.default_shell_surface_visible("window-menu"));
        assert!(session.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction {
                window: crate::model::WindowId(71),
                action: crate::platform::WindowAction::Minimize
            }
        )));
        assert!(shell.global_shortcut(crate::platform::GlobalShortcut::ShowWindowMenu));
        shell.plugin_panel_scene(&key, 320, 400).unwrap();
        let stale = shell
            .plugin_panel_host_for(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Minimize".into(),
            })
            .unwrap();
        session.take_commands();
        shell.windows[0].state.capabilities.minimize = false;
        assert!(shell.plugin_panel_host_ui_for(
            &key,
            nickel_ui::UiEvent::AccessibilityActivate(stale.id),
            320,
            400
        ));
        assert!(
            !session.take_commands().iter().any(|command| matches!(
                command,
                crate::platform::ShellCommand::WindowAction { .. }
            ))
        );
        assert!(!shell.apply_public_window_operation(
            "windows.minimize",
            Some(crate::model::WindowId(71)),
            None
        ));
        assert!(session.take_commands().is_empty());
        shell.windows[0].state.maximized = true;
        assert!(!shell.apply_public_window_operation(
            "windows.maximize",
            Some(crate::model::WindowId(71)),
            None
        ));
        assert!(session.take_commands().is_empty());
        assert!(shell.apply_public_window_operation(
            "windows.restore",
            Some(crate::model::WindowId(71)),
            None
        ));
        assert!(matches!(
            session.take_commands().as_slice(),
            [crate::platform::ShellCommand::WindowAction {
                window: crate::model::WindowId(71),
                action: crate::platform::WindowAction::Maximize
            }]
        ));
        // A selected theme may omit this frontend; native hotkeys cannot summon a base fallback.
        use nickel_core::plugins::{PluginPackage, PluginPackageSource};
        let package = PluginPackage::from_embedded(&[
            ("plugin.json", br#"{"api_version":1,"id":"window-menu-omitted-theme","name":"Theme","version":"1.0.0","entry":"main.js","composition":{"api_version":1,"id":"window-menu-omitted-theme","version":"1.0.0","extends":"nickel-default","requires":{"nickel-default":"^0.2.0"},"replaces":{"shell":"./main.js#Shell"}},"surfaces":[{"id":"taskbar","kind":"panel","width":800,"height":42}]}"#),
            ("main.js", b"export function Shell(){return h(FixedWindow,{id:'taskbar',width:800,height:42},h(Text,{},'Theme'));}\nexport default Shell;")
        ]).unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell.external_plugin_packages.insert(
            "window-menu-omitted-theme".into(),
            PluginPackageSource::embedded(package),
        );
        shell
            .select_shell_package("window-menu-omitted-theme")
            .unwrap();
        session.take_commands();
        assert!(!shell.global_shortcut(crate::platform::GlobalShortcut::ShowWindowMenu));
        assert!(!shell.default_shell_surface_visible("window-menu"));
        assert!(session.take_commands().is_empty());
    });
}

#[test]
fn selected_volume_surface_is_optional_and_hidden_by_lock() {
    let mut shell = LiveShell::new().unwrap();
    shell.show_volume_osd();
    assert!(shell.default_shell_surface_visible("volume-osd"));
    assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
    shell.global_shortcut(crate::platform::GlobalShortcut::LockState { locked: true });
    assert!(!shell.default_shell_surface_visible("volume-osd"));
    assert!(shell.volume_osd_until.is_none());
    shell.show_volume_osd();
    assert!(shell.volume_osd_until.is_none());
    shell.locked = false;
    shell.active_shell_package_id = "theme-without-volume".into();
    shell.show_volume_osd();
    assert!(shell.volume_osd_until.is_none());
}

#[test]
fn run_effect_rechecks_current_authority_before_native_execution() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        let owner = "nickel-default";
        let snapshot = shell.plugin_run_snapshot(owner).unwrap();
        let execute = crate::run_capabilities::Execute::parse(&serde_json::json!({"command":"must-not-execute-stale-command", "revision":snapshot["revision"]})).unwrap();
        shell.plugin_activation_generation += 1;
        assert!(
            !shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::RunExecute {
                plugin_id: owner.into(),
                execute: execute.clone()
            }])
        );
        assert!(shell.run_status.is_empty());
        shell.locked = true;
        assert!(shell.plugin_run_snapshot(owner).is_none());
        let execute = crate::run_capabilities::Execute {
            revision: crate::run_capabilities::revision(owner, shell.plugin_activation_generation),
            ..execute
        };
        assert!(
            !shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::RunExecute {
                plugin_id: owner.into(),
                execute
            }])
        );
        assert!(shell.run_status.is_empty());
    });
}

#[test]
fn stock_window_menu_dismisses_after_native_window_focus_loss() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        assert!(shell.set_default_shell_surface_visible("window-menu", true));
        let key = shell.active_shell_surface_key("window-menu");
        assert!(shell.plugin_panel_host_window_focus_for(&key, false, 320, 400));
        assert!(shell.default_shell_surface_visible("window-menu"));
        shell.plugin_panel_host_window_focus_for(&key, true, 320, 400);
        assert!(shell.default_shell_surface_visible("window-menu"));
        assert!(shell.plugin_panel_host_window_focus_for(&key, false, 320, 400));
        assert!(!shell.default_shell_surface_visible("window-menu"));
    });
}

#[test]
fn child_window_surface_observation_tracks_geometry_then_native_focus() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        assert!(shell.set_default_shell_surface_visible("quick-settings", true));
        let key = shell.active_shell_surface_key("quick-settings");
        shell
            .plugin_panel_scene_for_output(&key, Some("primary"), 720, 540)
            .unwrap();
        let observation = shell
            .plugin_panel_host_ref(&key)
            .unwrap()
            .application()
            .surface_observation();
        assert_eq!(observation["output"], "primary");
        assert_eq!(observation["visible"], true);
        assert_eq!(observation["focused"], true);

        assert!(shell.plugin_panel_host_window_focus_for(&key, false, 720, 540));
        let observation = shell
            .plugin_panel_host_ref(&key)
            .unwrap()
            .application()
            .surface_observation();
        assert_eq!(observation["output"], "primary");
        assert_eq!(observation["focused"], false);
    });
}

#[test]
fn selected_launcher_and_run_never_use_reserved_launcher_presentation() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().expect("live shell");
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowLauncher);
        assert!(shell.launcher_intent_visible());
        let launcher = shell.active_launcher_surface_key().unwrap();
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowRun);
        let run = shell.active_shell_surface_key("run");
        assert_ne!(run, launcher);
        assert!(shell.plugin_surface_matches(&run));
        assert!(!shell.shell_fixed_surface_keys().contains(&run));
        assert!(!shell.plugin_panel_scene(&run, 620, 180).unwrap().is_empty());
        assert!(shell.plugin_surface_change_token(&run).is_some());
        assert!(shell.plugin_surface_change_token(&launcher).is_some());
        let role = SurfaceRole::Launcher;
        assert!(!shell.surface_visible(role));
        assert!(shell.scene(role, 800, 600).is_empty());
        assert!(shell.scene_change_token(role).is_none());
        assert!(shell.layout_snapshot(role, None, None).is_none());
        assert!(shell.surface_remote_access_protected(role));
        assert!(shell.bounded_shell_semantics(role, None).is_err());
        assert!(!shell.dismiss_ephemeral_on_focus_loss(role));
        assert!(!shell.hide_overlay(role));
        assert!(shell.plugin_surface_matches(&launcher));
        assert!(shell.plugin_surface_matches(&run));
        shell.global_shortcut(crate::platform::GlobalShortcut::HideLauncher);
        assert!(!shell.launcher_intent_visible());
        assert!(shell.plugin_surface_matches(&run));
    });
}

#[test]
fn shipped_example_shell_composes_owned_taskbar_default_controls_and_registered_setting() {
    with_package_runtime_stack(|| {
        use nickel_core::plugins::{PluginPackage, PluginPackageSource, PluginSurfaceKey};
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-shell"
        ))
        .unwrap();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.clone(), PluginPackageSource::embedded(package));
        shell.set_plugin_enabled(&id, true).unwrap();
        shell.select_shell_package(&id).unwrap();
        let key = |surface: &str| PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: surface.into(),
        };
        let taskbar = key("taskbar");
        shell.plugin_panel_scene(&taskbar, 1280, 56).unwrap();
        let shared = shell
            .plugin_panel_host_for(&taskbar)
            .unwrap()
            .application()
            .shared_composition_runtime()
            .unwrap();
        let resolution = shared.borrow().resolution().clone();
        assert_eq!(resolution.exports["shell.taskbar"].implemented_by.id, id);
        assert_eq!(
            resolution.exports["shell.quickSettings"].implemented_by.id,
            "nickel-default"
        );
        assert!(
            !shell
                .plugin_registry
                .get(&id)
                .unwrap()
                .manifest
                .capabilities
                .contains(&nickel_core::plugins::PluginCapability::AudioControl)
        );
        let button = shell
            .plugin_panel_host_for(&taskbar)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Quick Settings".into(),
            })
            .unwrap();
        shell.plugin_panel_host_ui_for(
            &taskbar,
            nickel_ui::UiEvent::AccessibilityActivate(button.id),
            1280,
            56,
        );
        let quick = key("quick-settings");
        shell.plugin_panel_scene(&quick, 420, 600).unwrap();
        for _ in 0..3 {
            shell.plugin_panel_scene(&taskbar, 1280, 56).unwrap();
            shell.plugin_panel_scene(&quick, 420, 600).unwrap();
        }
        let stable_taskbar_token = shell.plugin_surface_change_token(&taskbar).unwrap();
        let stable_quick_settings_token = shell.plugin_surface_change_token(&quick).unwrap();
        let stable_application_catalog_builds = shell.application_catalog_builds;
        for _ in 0..8 {
            shell.plugin_panel_scene(&taskbar, 1280, 56).unwrap();
            shell.plugin_panel_scene(&quick, 420, 600).unwrap();
            assert_eq!(
                shell.plugin_surface_change_token(&taskbar),
                Some(stable_taskbar_token),
                "alternating composed surfaces must not rebuild the taskbar"
            );
            assert_eq!(
                shell.plugin_surface_change_token(&quick),
                Some(stable_quick_settings_token),
                "alternating composed surfaces must not rebuild quick settings"
            );
        }
        assert_eq!(
            shell.application_catalog_builds, stable_application_catalog_builds,
            "stable composed surfaces must reuse their application projection"
        );
        assert!(std::rc::Rc::ptr_eq(
            &shared,
            &shell
                .plugin_panel_host_for(&quick)
                .unwrap()
                .application()
                .shared_composition_runtime()
                .unwrap()
        ));
        assert!(
            shell
                .plugin_panel_host_for(&quick)
                .unwrap()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Lock".into()
                })
                .is_ok()
        );
        assert!(
            shell
                .package_settings_registry
                .settings_snapshot()
                .settings
                .iter()
                .any(|setting| setting.provider_package == id
                    && setting.registration.id == "show-title")
        );
        shell.launch_settings(Some(&format!("{id}/show-title")));
        let settings = key("settings");
        shell.plugin_panel_scene(&settings, 1100, 800).unwrap();
        let toggle = shell
            .plugin_panel_host_for(&settings)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Switch,
                name: "Show shell title".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &settings,
            nickel_ui::UiEvent::AccessibilityActivate(toggle.id),
            1100,
            800
        ));
        let owner = shared.borrow().resolution().active.clone();
        let values = shared
            .borrow()
            .shared_owner_runtime(&owner)
            .unwrap()
            .borrow_mut()
            .read_settings_values(&shell.package_settings_registry)
            .unwrap();
        assert_eq!(values["show-title"], serde_json::json!(false));
    });
}

#[test]
fn admitted_native_application_windows_reach_granted_public_resources() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        let application =
            crate::codex_project_application_id(Some("public-proof"), std::path::Path::new(""));
        shell.windows = vec![crate::model::OpenWindow {
            id: crate::model::WindowId(71),
            application_id: Some(crate::model::ApplicationId::new(application.clone())),
            active: true,
            title: "Native project chat".into(),
            state: crate::model::WindowState::default(),
        }];
        let windows = shell.external_plugin_windows("nickel-default").unwrap();
        assert_eq!(windows[0]["id"], "71");
        assert_eq!(windows[0]["applicationId"], application);
        assert_eq!(windows[0]["canActivate"], true);
        assert_eq!(windows[0]["canClose"], true);
        let apps = shell
            .external_plugin_applications("nickel-default")
            .unwrap();
        let app = apps
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == application)
            .unwrap();
        assert_eq!(app["name"], "Native project chat");
        assert_eq!(app["canLaunch"], false); // running identity is not an invented launch command
        assert_eq!(app["canPin"], false);
        let images = shell.plugin_application_images(Some(&apps), None);
        assert!(images.contains_key(&crate::application_capabilities::icon_asset(&application)));
        let key = shell.active_shell_surface_key("taskbar");
        shell.plugin_panel_scene(&key, 1280, 40).unwrap();
        let button = shell
            .plugin_panel_host_for(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Native project chat".into(),
            })
            .unwrap();
        assert!(shell.plugin_panel_host_ui_for(
            &key,
            nickel_ui::UiEvent::AccessibilityContextMenu(button.id),
            1280,
            40
        ));
        assert!(
            shell
                .plugin_panel_host_for(&key)
                .unwrap()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::MenuItem,
                    name: "Pin".into(),
                })
                .is_err()
        );
        assert!(
            shell
                .plugin_panel_host_for(&key)
                .unwrap()
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::MenuItem,
                    name: "Close Native project chat".into(),
                })
                .is_ok()
        );
        shell
            .external_plugin_packages
            .get_mut("nickel-default")
            .unwrap()
            .manifest
            .capabilities
            .retain(|capability| {
                *capability != nickel_core::plugins::PluginCapability::WindowsRead
            });
        assert!(shell.external_plugin_windows("nickel-default").is_none());
        let read_only_catalog = shell
            .external_plugin_applications("nickel-default")
            .unwrap();
        assert!(
            !read_only_catalog
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == application)
        );
        let manifest = &mut shell
            .external_plugin_packages
            .get_mut("nickel-default")
            .unwrap()
            .manifest;
        manifest.capabilities.retain(|capability| {
            !matches!(
                capability,
                nickel_core::plugins::PluginCapability::WindowsRead
                    | nickel_core::plugins::PluginCapability::ApplicationsRead
            )
        });
        assert!(shell.external_plugin_windows("nickel-default").is_none());
        assert!(
            shell
                .external_plugin_applications("nickel-default")
                .is_none()
        );
    });
}

#[test]
fn launcher_sizes_to_its_named_output_and_expands_again_on_a_larger_output() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.set_desktop_outputs(vec![
            nickel_file::desktop::DesktopOutput {
                id: "large".into(),
                primary: true,
                work_area: nickel_file::desktop::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1024.0,
                },
                scale: 1.0,
            },
            nickel_file::desktop::DesktopOutput {
                id: "small".into(),
                primary: false,
                work_area: nickel_file::desktop::Rect {
                    x: -376.0,
                    y: 0.0,
                    width: 376.0,
                    height: 436.0,
                },
                scale: 2.0,
            },
        ]);
        shell.global_shortcut(crate::platform::GlobalShortcut::ShowLauncher);
        let key = LiveShell::default_shell_surface_key("launcher");
        shell
            .plugin_panel_scene_for_output(&key, Some("small"), 360, 420)
            .unwrap();
        assert_eq!(
            (
                shell.plugin_surface_hosts[&key].0.width,
                shell.plugin_surface_hosts[&key].0.height
            ),
            (360, 420)
        );
        shell
            .plugin_panel_scene_for_output(&key, Some("large"), 608, 628)
            .unwrap();
        assert_eq!(
            (
                shell.plugin_surface_hosts[&key].0.width,
                shell.plugin_surface_hosts[&key].0.height
            ),
            (608, 628)
        );
    });
}

#[test]
fn replicated_taskbar_keeps_one_output_agnostic_viewport_across_monitors() {
    with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.set_desktop_outputs(vec![
            nickel_file::desktop::DesktopOutput {
                id: "DP-3".into(),
                primary: true,
                work_area: nickel_file::desktop::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1024.0,
                },
                scale: 1.0,
            },
            nickel_file::desktop::DesktopOutput {
                id: "DVI-I-2".into(),
                primary: false,
                work_area: nickel_file::desktop::Rect {
                    x: 1920.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1024.0,
                },
                scale: 1.0,
            },
        ]);
        let key = LiveShell::default_shell_surface_key("taskbar");
        shell
            .plugin_panel_scene_for_output(&key, Some("DP-3"), 1920, 56)
            .unwrap();
        shell
            .plugin_panel_scene_for_output(&key, Some("DVI-I-2"), 1920, 56)
            .unwrap();
        let viewport = serde_json::json!({
            "width": 1920, "height": 56, "output": null,
            "availableWidth": null, "availableHeight": null,
        });
        assert!(
            !shell
                .plugin_surface_hosts
                .get_mut(&key)
                .unwrap()
                .1
                .application_mut()
                .sync_host_data_field("viewport", &viewport)
                .unwrap()
        );
    });
}
