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
    platform::{AudioStatus, FeedState, FeedStatus, GlobalShortcut, SecureStorageState},
    preview_refresh_due, retain_unchanged_desktop_icons, secure_storage_status_label,
    semantic_theme_from_palette, session_feed_status_label, shortcut_capability_status,
    visible_tray_item, window_belongs_to_panel,
};

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");
include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");

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
