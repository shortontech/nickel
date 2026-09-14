use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nickel-core is nested under the workspace crates directory")
        .to_path_buf()
}

fn rust_sources(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

#[test]
fn reuse_audit_source_inventory_covers_every_workspace_rust_source() {
    let root = workspace_root();
    let inventory = fs::read_to_string(root.join("assets/code-reuse-source-inventory.tsv"))
        .expect("code-reuse source inventory must be readable");
    let rows = inventory
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, count) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("invalid source inventory row: {line}"));
            (
                name.to_owned(),
                count
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("invalid source count in row: {line}")),
            )
        })
        .collect::<Vec<_>>();
    let expected = rows.iter().cloned().collect::<BTreeMap<_, _>>();
    assert_eq!(
        expected.len(),
        rows.len(),
        "code-reuse source inventory crate names must be unique"
    );
    let crates = root.join("crates");
    let mut observed = BTreeMap::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory must be readable") {
        let path = entry.expect("crate entry must be readable").path();
        if !path.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        rust_sources(&path, &mut sources);
        if !sources.is_empty() {
            observed.insert(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                sources.len(),
            );
        }
    }
    assert_eq!(
        observed, expected,
        "Rust sources changed; audit the new or removed production behavior and refresh the checked-in inventory"
    );
}

#[test]
fn logical_rectangle_and_intersection_have_one_shared_authority() {
    let root = workspace_root();
    let authority = fs::read_to_string(root.join("crates/nickel-core/src/geometry.rs")).unwrap();
    assert_eq!(authority.matches("pub struct LogicalRect").count(), 1);
    assert_eq!(authority.matches("pub fn intersection_area").count(), 1);

    for consumer in [
        "crates/nickel-core/src/dpi.rs",
        "crates/nickel/src/session/shell_layout.rs",
        "crates/nickel/src/platform/linux.rs",
    ] {
        let source = fs::read_to_string(root.join(consumer)).unwrap();
        assert!(
            !source.contains("fn intersection_area"),
            "{consumer} restored parallel logical-rectangle intersection arithmetic"
        );
    }
    let shell_layout =
        fs::read_to_string(root.join("crates/nickel/src/session/shell_layout.rs")).unwrap();
    assert!(shell_layout.contains("nickel_core::geometry::LogicalRect"));
}

#[test]
fn nickel_core_delegates_configuration_storage_mechanics() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&source, &mut files);
    let forbidden = [
        "XDG_CONFIG_HOME",
        "LOCALAPPDATA",
        "Library/Application Support/Nickel",
        "tmp-{}",
    ];
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for token in forbidden {
            if text.contains(token) {
                violations.push(format!("{} contains {token:?}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "platform storage mechanics leaked into nickel-core:\n{}",
        violations.join("\n")
    );

    for (module, expected_calls, writer) in [
        ("launcher_preferences.rs", 1, "atomic_write"),
        ("wallpaper_settings.rs", 1, "atomic_write"),
        ("shell_settings.rs", 1, "stage_write"),
        ("optional_features.rs", 2, "atomic_write"),
        ("dpi.rs", 2, "atomic_write"),
    ] {
        let text = fs::read_to_string(source.join(module)).unwrap();
        let production = text.split("#[cfg(test)]").next().unwrap();
        assert!(
            production
                .lines()
                .any(|line| line.starts_with("use nickel_storage::{")
                    && line.contains(writer)
                    && line.contains("config_path")),
            "{module} must delegate paths and replacement to nickel-storage"
        );
        assert_eq!(
            production.matches(&format!("{writer}(")).count(),
            expected_calls,
            "{module} must route each complete settings write through the shared authority"
        );
        assert!(
            !production.contains("fs::write(") && !production.contains("fs::rename("),
            "{module} restored a non-atomic settings writer"
        );
    }
}

#[test]
fn geometry_effect_entrypoints_publish_desired_state_before_writing() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let section = |start: &str, end: &str| {
        let start = state
            .find(start)
            .unwrap_or_else(|| panic!("missing {start}"));
        let end = state[start..]
            .find(end)
            .map(|offset| start + offset)
            .unwrap_or(state.len());
        &state[start..end]
    };

    let mapped = section(
        "pub(crate) fn map_compositor_moved_window",
        "fn apply_compositor_moved_window_effect",
    );
    assert!(
        mapped.find("try_authorize_desired_geometry").unwrap()
            < mapped.find("apply_compositor_moved_window_effect").unwrap(),
        "interactive placement must be authorized before its adapter effect"
    );

    let internal = section(
        "pub(crate) fn apply_internal_move",
        "pub(crate) fn finish_internal_move",
    );
    assert!(
        internal.find("authorize_placement").unwrap()
            < internal.find("internal_ui.relocate").unwrap(),
        "internal movement must be authorized before relocation"
    );
    assert!(
        !state.contains("(placement.geometry.0, placement.geometry.1) !="),
        "rollback must not infer ownership from equal geometry values"
    );
}

#[test]
fn xdg_configures_and_repeated_x11_client_requests_use_typed_tracking() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let resize =
        fs::read_to_string(root.join("crates/nickel/src/session/grabs/resize_grab.rs")).unwrap();
    let xdg =
        fs::read_to_string(root.join("crates/nickel/src/session/handlers/xdg_shell.rs")).unwrap();
    let x11 =
        fs::read_to_string(root.join("crates/nickel/src/session/handlers/xwayland.rs")).unwrap();

    assert_eq!(
        state.matches("send_pending_configure()").count(),
        1,
        "state.rs must emit pending XDG configures only inside the tracked wrapper"
    );
    assert!(!resize.contains("xdg.send_pending_configure()"));
    assert!(!xdg.contains("toplevel.send_pending_configure()"));
    assert!(state.contains("record_xdg_desired_geometry(&window, desired, serial)"));
    assert!(state.contains("Option<crate::session::grabs::resize_grab::ResizeEdge>"));
    assert!(!x11.contains("settlement_request = registry_id.map"));
    let client_fact = state
        .split("pub(crate) fn record_x11_client_desired_geometry")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn record_x11_interactive_final")
        .next()
        .unwrap();
    assert!(client_fact.contains("observe_x11_geometry"));
    assert!(!client_fact.contains("record_x11_desired_geometry"));
}

#[test]
fn xdg_tracking_extends_equal_revision_requests_and_uses_effective_placement() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let record = state
        .split("pub(crate) fn record_xdg_desired_geometry")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn observe_xdg_geometry_commit")
        .next()
        .unwrap();

    assert!(record.contains("xdg_configure_extends_existing_request"));
    assert!(
        record.find("record_xdg_configure_incorporation").unwrap()
            < record
                .find("NativeRequestId(self.x11_next_native_request)")
                .unwrap(),
        "an unchanged desired revision must extend serial incorporation before allocating a request"
    );
    assert!(
        state
            .matches(".map(|authority| authority.constrained_proposal)")
            .count()
            >= 2
    );
}

#[test]
fn independent_x11_configure_revokes_an_active_operation_before_native_apply() {
    let root = workspace_root();
    let x11 =
        fs::read_to_string(root.join("crates/nickel/src/session/handlers/xwayland.rs")).unwrap();
    let accepted = x11
        .split("diagnostic: X11 configure accepted")
        .nth(1)
        .unwrap()
        .split("fn configure_notify")
        .next()
        .unwrap();
    let cancel = accepted
        .find("CancellationReason::Superseded")
        .expect("independent geometry must supersede an overlapping operation");
    let observe = accepted
        .find("record_x11_client_desired_geometry")
        .expect("independent geometry must revoke Nickel authority");
    let apply = accepted
        .find("window.configure(geometry)")
        .expect("accepted geometry must reach XWayland");
    assert!(cancel < observe && observe < apply);
}

#[test]
fn unknown_x11_notify_defers_issued_request_failure_to_bounded_reconciliation() {
    let root = workspace_root();
    let x11 =
        fs::read_to_string(root.join("crates/nickel/src/session/handlers/xwayland.rs")).unwrap();
    let notify = x11
        .split("fn configure_notify(")
        .nth(1)
        .unwrap()
        .split("fn property_notify")
        .next()
        .unwrap();
    let pending = notify.find("x11_has_pending_issued_request").unwrap();
    let cancel = notify.find("cancel_geometry_window_operation").unwrap();
    let observe = notify.find("observe_x11_untrusted_notification").unwrap();
    assert!(pending < cancel && cancel < observe);
    assert!(notify.contains("observe_x11_untrusted_notification"));

    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let binding = state
        .split("fn bind_x11_geometry_request")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn record_xdg_desired_geometry")
        .next()
        .unwrap();
    assert!(binding.contains("SettlementStatus::Unconfirmed"));
    assert!(binding.contains("cancel_geometry_window_operation"));
}

#[test]
fn relayout_and_hidden_rescue_require_live_geometry_authority() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let relayout = state
        .split("pub(crate) fn relayout_maximized_windows")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn apply_maximized_x11_geometry")
        .next()
        .unwrap();
    assert!(relayout.contains("apply_authorized_complete_window_geometry"));
    assert!(!relayout.contains("self.configure_window(&window, geometry)"));
    let removal = state
        .split("for (id, (window, location)) in &mut self.minimized_windows")
        .nth(1)
        .unwrap()
        .split("self.displaced_output_windows")
        .next()
        .unwrap();
    assert!(removal.matches("try_authorize_placement").count() >= 2);
    assert!(!removal.contains("authority.set_placement"));
}

#[test]
fn interactive_x11_effects_bind_bounded_requests_before_native_writes() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    for (start, end) in [
        (
            "pub(crate) fn apply_authorized_interactive_resize",
            "pub(crate) fn compensate_interactive_resize",
        ),
        (
            "fn apply_compositor_moved_window_effect",
            "fn apply_authorized_complete_window_geometry",
        ),
    ] {
        let section = state
            .split(start)
            .nth(1)
            .unwrap()
            .split(end)
            .next()
            .unwrap();
        assert!(
            section.find("bind_x11_geometry_request").unwrap()
                < section.find(".configure(Rectangle::new").unwrap()
        );
    }
    let binding = state
        .split("fn bind_x11_geometry_request")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn x11_has_pending_issued_request")
        .next()
        .unwrap();
    assert!(binding.contains("x11_issued_geometry_requests.entry(id)"));
}

#[test]
fn mapped_output_rescue_records_displacement_only_after_success() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let mapped = state
        .split("for (id, window, location, size) in mapped")
        .nth(1)
        .unwrap()
        .split("for (id, (window, location)) in &mut self.minimized_windows")
        .next()
        .unwrap();
    let apply = mapped.find("if !self.map_compositor_moved_window").unwrap();
    let record = mapped.find("displaced.push").unwrap();
    assert!(apply < record);
    assert!(mapped[apply..record].contains("continue"));
}

#[test]
fn x11_notify_stays_unknown_and_drag_deadline_is_bounded() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let causality = state
        .split("pub(crate) fn observe_x11_untrusted_notification")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn cancel_geometry_window_operation")
        .next()
        .unwrap();
    assert!(causality.contains("ObservationCausality::Unknown"));
    assert!(!causality.contains("settlement.request.placement == observed"));
    assert!(!causality.contains("ObservationCausality::Correlated"));
    assert!(causality.contains("authority.observe("));
    assert!(causality.contains("ledger.back_mut()"));

    let binding = state
        .split("fn bind_x11_geometry_request")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn x11_has_pending_issued_request")
        .next()
        .unwrap();
    assert!(binding.contains("now.saturating_add(10_000)"));
    assert!(binding.contains("ledger.push_back(settlement)"));
    assert!(!binding.contains("settlement.request.placement = desired"));
}

#[test]
fn remote_set_bounds_authorizes_exact_geometry_before_effects() {
    let root = workspace_root();
    let state = fs::read_to_string(root.join("crates/nickel/src/session/state.rs")).unwrap();
    let action = state
        .split("WindowAction::SetBounds {")
        .nth(2)
        .unwrap()
        .split("WindowAction::Activate")
        .next()
        .unwrap();
    let authorize = action.find("try_authorize_desired_geometry").unwrap();
    let xdg = action.find("self.configure_window").unwrap();
    let x11 = action.find(".configure(Rectangle::new").unwrap();
    assert!(authorize < xdg && authorize < x11);
    assert!(action.contains("apply_authorized_complete_window_geometry"));
    assert!(!action.contains("record_desired_geometry(id, geometry)"));
}
