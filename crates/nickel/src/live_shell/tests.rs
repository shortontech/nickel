use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use image::{Rgba, RgbaImage};
use nickel_input::KeyCode;
use nickel_session_protocol::{
    AnchorSide, PointerInteraction, PreviewTargetAction, ScreenshotTargetAction, ShellRole,
    ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::{
    ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
    HostTelemetry, InputModality, OverlayAnchor, Point, Rect, SemanticAction, SemanticRole,
    SemanticSelector, SemanticValueInput, SemanticValueSnapshot, Shortcut, UiEvent, UiHost,
    ViewContext,
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
    semantic_theme_from_palette, session_feed_status_label, shortcut_capability_status,
    visible_tray_item, window_belongs_to_panel,
};

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");
include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");

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
            .volume_osd_host
            .application()
            .label
            .starts_with("Volume 31%")
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
fn native_audio_feedback_ignores_startup_metadata_and_reconnect_but_shows_value_changes() {
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
            .volume_osd_host
            .application()
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

    assert!(shell.dispatch_session_command(
        "test-direct-session-host",
        crate::platform::ShellCommand::CreateWorkspace,
    ));
    assert_eq!(host.0.load(Ordering::Relaxed), 1);
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
    shell.launcher_host.step(HostBatch {
        surface_size: Some((920, 680)),
        ..HostBatch::default()
    });
    assert!(shell.launcher_host.inspect().keyboard_focus.is_some());

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
    assert!(shell.global_shortcut(crate::platform::GlobalShortcut::CommitSwitch));
    assert!(session.0.lock().unwrap().iter().any(|command| matches!(
        command,
        crate::platform::ShellCommand::WindowAction {
            action: crate::platform::WindowAction::Activate,
            ..
        }
    )));

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
