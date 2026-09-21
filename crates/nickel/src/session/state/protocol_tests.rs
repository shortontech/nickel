use super::{
    ControllerConnectionGeneration, ControllerHostId, DisplacedWindow,
    ExternalControllerLeaseBinding, PREVIEW_BYTE_CAPACITY, PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER,
    PREVIEW_ENTRY_CAPACITY, PREVIEW_FRAME_BYTES, PendingLaunchObservation,
    PendingLaunchWindowDisposition, RegisteredShellRole, ShellRegistrationRejection,
    XdgConfigureSettlement, admitted_preview_ids, advance_preview_content_generation,
    apply_shell_behavior_value, bounded_preview_ids, clamp_decorated_content_to_work_area,
    clamp_window_location, clamped_restore_geometry, command_requires_shell_identity,
    drag_icon_location, external_controller_surface_changed,
    fail_x11_settlement_on_timer_registration, identification_expiry_is_current,
    internal_restore_is_current, mapped_surface_origin, maximized_content_geometry,
    output_contains_logical_point, output_index_for_shell_surface,
    output_rescue_revision_is_current, pending_launch_window_disposition,
    placement_restore_is_current, prepare_shell_behavior_update, preview_mapping_has_exact_size,
    protocol_preview_from_cached, record_preview_capture_attempt, restored_drag_content_geometry,
    retain_live_idle_inhibitors, retain_superseded_xdg_settlement, retire_displaced_window,
    retire_pointer_surface, retire_shell_surface, reuse_preview_pixels, shell_behavior_value,
    shell_registration_is_active, shell_registration_rejection, shell_registration_role_changed,
    shell_role_accepts_ordinary_focus, test_control_may_invoke, x11_fullscreen_restore_geometry,
    xdg_configure_extends_existing_request, xdg_configure_matches_existing_desired,
    xdg_settlement_requires_resize_cleanup,
};

#[test]
fn mapped_client_surface_origin_accounts_for_nonzero_window_geometry() {
    let mapped = smithay::utils::Point::from((120, 80));
    let client_geometry = smithay::utils::Point::from((7, 40));

    assert_eq!(
        mapped_surface_origin(mapped, client_geometry),
        (113, 40).into()
    );
}

#[test]
fn x11_fullscreen_restore_uses_global_mapping_and_client_size() {
    let client_geometry = smithay::utils::Rectangle::new((0, 0).into(), (484, 316).into());

    assert_eq!(
        x11_fullscreen_restore_geometry((390, 230).into(), client_geometry),
        smithay::utils::Rectangle::new((390, 230).into(), (484, 316).into())
    );
}

#[test]
fn x11_timer_registration_failure_is_terminal_and_preserves_observed_fact() {
    use nickel_core::geometry_authority::{
        CoordinateUnits, GeometryAuthority, GeometryMeaning, NativeRequest, NativeRequestId,
        ObservationCausality, Presentation, Settlement, SettlementLimits, SettlementStatus,
        TaggedGeometry,
    };
    let placement = nickel_core::geometry::LogicalRect {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    let authority = GeometryAuthority::new(placement, Presentation::Normal);
    let request_id = NativeRequestId(41);
    let mut settlement = Settlement::new(
        NativeRequest {
            id: request_id,
            mapping_generation: 7,
            desired: authority.revisions(),
            placement,
        },
        SettlementLimits {
            deadline_tick: 750,
            max_corrections: 1,
        },
    );
    let observed = TaggedGeometry {
        rect: nickel_core::geometry::LogicalRect { x: 11, ..placement },
        meaning: GeometryMeaning::CanonicalManagedBounds,
        units: CoordinateUnits::CanonicalLogical,
        topology_version: 1,
    };
    settlement.observe(observed, ObservationCausality::Unknown);

    assert_eq!(
        fail_x11_settlement_on_timer_registration(&mut settlement, request_id),
        Some(Some(observed))
    );
    assert_eq!(settlement.status, SettlementStatus::Failed);
    assert_eq!(
        fail_x11_settlement_on_timer_registration(&mut settlement, request_id),
        None
    );
}

#[test]
fn repeated_xdg_configure_extends_only_the_same_desired_request() {
    use nickel_core::geometry_authority::{
        GeometryAuthority, NativeRequest, NativeRequestId, Presentation, Settlement,
        SettlementLimits,
    };
    let first = nickel_core::geometry::LogicalRect {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    let mut authority = GeometryAuthority::new(first, Presentation::Normal);
    let revisions = authority.revisions();
    let mut record = XdgConfigureSettlement {
        configures: std::collections::VecDeque::new(),
        outcomes: std::collections::VecDeque::new(),
        settlement: Settlement::new(
            NativeRequest {
                id: NativeRequestId(1),
                mapping_generation: 7,
                desired: revisions,
                placement: first,
            },
            SettlementLimits {
                deadline_tick: 750,
                max_corrections: 1,
            },
        ),
    };
    assert!(xdg_configure_extends_existing_request(
        &record, revisions, first
    ));
    assert!(xdg_configure_matches_existing_desired(
        &record, revisions, first
    ));
    assert!(!xdg_settlement_requires_resize_cleanup(&record));
    record.configures.push_back((
        12_u32.into(),
        revisions,
        Some(crate::session::grabs::resize_grab::ResizeEdge::LEFT),
    ));
    assert!(xdg_settlement_requires_resize_cleanup(&record));
    record.settlement.fail();
    assert!(!xdg_configure_extends_existing_request(
        &record, revisions, first
    ));
    assert!(
        xdg_configure_matches_existing_desired(&record, revisions, first),
        "a terminal request still identifies later configures carrying its desired fields"
    );
    record.settlement.status = nickel_core::geometry_authority::SettlementStatus::Pending;
    record.settlement.expire(750);
    assert!(!xdg_configure_extends_existing_request(
        &record, revisions, first
    ));
    record.settlement.status = nickel_core::geometry_authority::SettlementStatus::Pending;

    let newer = nickel_core::geometry::LogicalRect { x: 11, ..first };
    authority.authorize_placement(
        newer,
        nickel_core::geometry_authority::GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    assert!(!xdg_configure_extends_existing_request(
        &record,
        authority.revisions(),
        newer
    ));

    let outcomes = retain_superseded_xdg_settlement(Some(record));
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        outcomes[0].status,
        nickel_core::geometry_authority::SettlementStatus::Superseded
    );
}

#[test]
fn external_controller_lease_is_bound_to_exact_surface_generation() {
    let binding = ExternalControllerLeaseBinding {
        host: ControllerHostId(44),
        connection: ControllerConnectionGeneration(7),
        surface: super::WindowId(10),
    };
    let active = Some((binding.host, binding.connection));
    assert!(!external_controller_surface_changed(
        binding,
        active,
        Some(super::WindowId(10))
    ));
    assert!(external_controller_surface_changed(
        binding,
        active,
        Some(super::WindowId(11))
    ));
    assert!(external_controller_surface_changed(binding, active, None));
}

#[test]
fn orderly_external_relinquish_selects_only_the_internal_successor() {
    let external = (ControllerHostId(44), ControllerConnectionGeneration(7));
    let internal = ControllerConnectionGeneration(2);
    assert_eq!(
        super::orderly_external_controller_successor(Some(external), external, internal),
        Some((ControllerHostId(0), internal))
    );
    assert_eq!(
        super::orderly_external_controller_successor(None, external, internal),
        None
    );
    assert_eq!(
        super::orderly_external_controller_successor(
            Some((ControllerHostId(0), internal)),
            (ControllerHostId(0), internal),
            internal,
        ),
        None
    );
}

#[test]
fn relinquish_then_overflow_preserves_internal_recovery_intent() {
    let external = (ControllerHostId(44), ControllerConnectionGeneration(7));
    let internal = ControllerConnectionGeneration(2);
    let mut intent = super::ControllerRecoveryIntent {
        eligible_owner: Some(external),
        ..Default::default()
    };
    assert!(intent.merge_orderly_relinquish(external, internal));
    intent.merge_overflow(None);
    assert_eq!(
        intent.observe_fresh_batch(true),
        Some((ControllerHostId(0), internal))
    );
}

#[test]
fn overflow_then_relinquish_preserves_generation_fence_and_internal_successor() {
    let external = (ControllerHostId(44), ControllerConnectionGeneration(7));
    let internal = ControllerConnectionGeneration(2);
    let mut intent = super::ControllerRecoveryIntent::default();
    intent.merge_overflow(Some(external));
    assert!(intent.merge_orderly_relinquish(external, internal));
    assert!(intent.require_fresh_generation);
    assert_eq!(
        intent.observe_fresh_batch(true),
        Some((ControllerHostId(0), internal))
    );
}
use crate::session::output_retirement::{
    BIND_SETTLE_GRACE as OUTPUT_GLOBAL_BIND_SETTLE_GRACE,
    DISABLED_GRACE as OUTPUT_GLOBAL_DISABLED_GRACE,
    MAX_PENDING as MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS,
};
use crate::session::shell_layout::Geometry;
use crate::{
    platform::{SessionRequestError, ShellCommand},
    session_host::SessionHost,
};
use nickel_session_protocol::{
    Command, OutputTransform, Query, ServerEnvelope, ServerMessage, SessionAction,
    ShellBehaviorSetting, ShellBehaviorTransaction, ShellBehaviorValue, ShellRole, TestOutput,
};
use smithay::{
    output::{Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        wayland_server::{Display, backend::ObjectId},
    },
    utils::Point,
};
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct InternalWindowTestApp;
struct InternalHitTestApp;

mod diagnostic_action_scenarios {
    include!("diagnostic_action_scenarios.rs");
}

fn internal_shell_test_session() -> (
    EventLoop<'static, super::NickelSession>,
    super::NickelSession,
) {
    let (event_loop, mut session) = preview_test_session();
    let output = Output::new(
        "file-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Test".into(),
            serial_number: "file-test".into(),
        },
    );
    output.change_current_state(
        Some(smithay::output::Mode {
            size: (1280, 720).into(),
            refresh: 60_000,
        }),
        None,
        None,
        None,
    );
    session.space.map_output(&output, (0, 0));
    let (_sender, receiver) = crate::platform::status_mailbox::channel();
    session
        .enable_internal_shell_with_system_updates(Arc::new(IdleInternalHost), receiver)
        .unwrap();
    (event_loop, session)
}

#[test]
fn remote_codex_owner_creates_hidden_menu_and_tears_down_every_owned_surface() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let mut settings = nickel_core::optional_features::OptionalFeatureSettings {
        codex_enabled: false,
        codex_generation: 41,
        ..Default::default()
    };
    session.apply_remote_codex_preference(&settings).unwrap();
    assert!(session.internal_codex.is_none());
    assert_eq!(session.remote_codex_runtime_generation, 41);

    settings.codex_enabled = true;
    settings.codex_generation = 42;
    session.apply_remote_codex_preference(&settings).unwrap();
    let owned = session
        .internal_codex
        .as_ref()
        .unwrap()
        .surface_ids()
        .collect::<Vec<_>>();
    assert_eq!(owned.len(), 1);
    assert!(!session.internal_ui.is_visible(owned[0]));
    assert_eq!(session.remote_codex_runtime_generation, 42);

    settings.codex_enabled = false;
    settings.codex_generation = 43;
    session.apply_remote_codex_preference(&settings).unwrap();
    assert!(session.internal_codex.is_none());
    assert!(
        owned
            .into_iter()
            .all(|id| !session.internal_ui.is_visible(id))
    );
    assert_eq!(session.remote_codex_runtime_generation, 43);
}

#[test]
fn newly_inserted_window_context_menu_receives_keyboard_focus() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session
        .internal_shell
        .as_mut()
        .unwrap()
        .apply_session_snapshot(nickel_session_protocol::Snapshot {
            windows: vec![nickel_session_protocol::WindowSnapshot {
                id: nickel_session_protocol::WindowId(41),
                application_id: "owned-test".into(),
                title: "Owned test".into(),
                active: true,
                minimized: false,
                maximized: false,
                fullscreen: false,
                geometry: None,
                workspace: nickel_session_protocol::WorkspaceId(1),
            }],
            ..Default::default()
        });
    assert!(
        session
            .internal_shell
            .as_mut()
            .unwrap()
            .open_window_menu_at(41, 120, 80)
    );
    session.sync_internal_shell();
    let menu = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surface(crate::winit_shell::SurfaceRole::WindowContextMenu, None)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&menu];
    assert_eq!(session.internal_ui.focused(), Some(runtime));

    assert!(
        !session
            .internal_ui
            .pointer_button_with_client((900.0, 200.0), true, true)
    );
    session.flush_internal_shell_input();
    assert!(!session.internal_shell.as_ref().unwrap().visible(menu));
    assert!(!session.internal_ui.is_visible(runtime));
}

#[test]
fn removed_shell_output_retires_every_old_surface_presentation() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let old = session
        .internal_shell_surfaces
        .values()
        .copied()
        .collect::<Vec<_>>();
    assert!(!old.is_empty());

    session.internal_shell.as_mut().unwrap().set_outputs(&[]);
    session.sync_internal_shell();

    assert!(session.internal_shell_surfaces.is_empty());
    assert!(
        old.into_iter()
            .all(|id| !session.internal_ui.is_visible(id))
    );
}

#[test]
fn workspace_notifications_record_coarse_owner_state_once() {
    use nickel_remote_control::desktop_events::DesktopEventKind;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let initial_count = session.workspaces.ordered().len();

    session.notify_workspace_state();
    session.notify_workspace_state();
    assert_eq!(session.remote_desktop_events.snapshot().events.len(), 1);

    session.workspaces.create().unwrap();
    session.notify_workspace_state();
    let snapshot = session.remote_desktop_events.snapshot();
    assert_eq!(snapshot.events.len(), 2);
    assert_eq!(
        snapshot.events.last().unwrap().event,
        DesktopEventKind::WorkspaceStateChanged {
            active_workspace: 1,
            workspaces: initial_count + 1,
        }
    );
}

#[test]
fn protocol_publication_records_ordinary_window_state_changes() {
    use nickel_remote_control::desktop_events::DesktopEventKind;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let window = session.register_internal_application(surface).unwrap();
    session.notify_protocol_snapshot();

    session.maximize_window(window);
    let event = session
        .remote_desktop_events
        .snapshot()
        .events
        .into_iter()
        .rev()
        .find(|event| {
            matches!(
                event.event,
                DesktopEventKind::WindowStateChanged { window_id, .. }
                    if window_id == window.0
            )
        })
        .expect("window state event");
    assert!(matches!(
        event.event,
        DesktopEventKind::WindowStateChanged {
            maximized: true,
            minimized: false,
            fullscreen: false,
            ..
        }
    ));
}

#[test]
fn codex_diagnostic_projects_health_without_source_or_failure_details() {
    use nickel_core::optional_features::{
        CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
    };
    use nickel_remote_control::diagnostics::{
        FeatureHealthDiagnostic, FeatureInstallationDiagnostic,
    };
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session
        .internal_shell
        .as_mut()
        .unwrap()
        .apply_codex_projection(CodexAvailabilityProjection::new(
            FeatureSupport::Supported,
            FeatureInstallation::Incompatible,
            true,
            FeatureHealth::Failed,
            47,
            Some("private path and provider failure".into()),
        ));
    let diagnostic = session
        .remote_codex_feature_diagnostic(8, 19)
        .expect("Codex projection");
    assert_eq!(diagnostic.observation_generation, 8);
    assert_eq!(diagnostic.observed_at_us, 19);
    assert!(diagnostic.supported && diagnostic.enabled);
    assert_eq!(
        diagnostic.installation,
        FeatureInstallationDiagnostic::Incompatible
    );
    assert_eq!(diagnostic.health, FeatureHealthDiagnostic::Failed);
    assert_eq!(diagnostic.configuration_generation, 47);
    let json = serde_json::to_string(&diagnostic).unwrap();
    assert!(!json.contains("private path"));
    assert!(!json.contains("provider failure"));
}

#[test]
fn pending_effect_diagnostic_counts_work_without_payloads_or_targets() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .first()
        .unwrap()
        .id;
    session.pending_desktop_scenes.insert(surface);
    session.pending_shell_focus_role = Some(ShellRole::Launcher);

    let diagnostic = session.remote_pending_effects_diagnostic(12, 34);
    assert_eq!(diagnostic.observation_generation, 12);
    assert_eq!(diagnostic.observed_at_us, 34);
    assert_eq!(diagnostic.desktop_scene_updates, 1);
    assert_eq!(diagnostic.image_copy_frames, 0);
    assert_eq!(diagnostic.launch_observations, 0);
    assert_eq!(diagnostic.output_retirements, 0);
    assert!(diagnostic.shell_focus_pending);

    let json = serde_json::to_string(&diagnostic).unwrap();
    assert!(!json.contains("launcher"));
    assert!(!json.contains("internal:"));
}

#[test]
#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
fn shell_capture_evidence_excludes_hidden_retired_and_trusted_surfaces() {
    use nickel_remote_control::leases::ResourceId;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.refresh_remote_output_identities();
    let desktop = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|entry| entry.role == crate::winit_shell::SurfaceRole::Desktop)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&desktop];
    let identity = ResourceId {
        id: format!("internal:{}", runtime.snapshot_token()),
        generation: runtime.snapshot_token(),
    };
    let (_, output) = session.surface_capture_evidence(&identity).unwrap();
    assert_eq!(output.unwrap().id, "file-test");
    session.internal_ui.set_visible(runtime, false);
    assert!(session.surface_capture_evidence(&identity).is_err());
    session.internal_ui.set_visible(runtime, true);
    session.locked = true;
    assert!(session.surface_capture_evidence(&identity).is_err());
    session.locked = false;
    let mut placement = session.internal_ui.placement(runtime).unwrap().clone();
    placement.geometry.0 -= 1;
    session.internal_ui.relocate(runtime, placement);
    assert!(
        session
            .surface_capture_evidence(&identity)
            .unwrap()
            .1
            .is_none(),
        "straddling content cannot claim output membership"
    );
    // Same label with a new native output object cannot reuse old output evidence
    // even before the identity refresh handles topology retirement.
    let native = session.space.outputs().next().unwrap().clone();
    session.space.unmap_output(&native);
    assert!(
        session
            .surface_capture_evidence(&identity)
            .unwrap()
            .1
            .is_none()
    );
    let trusted = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::TrustedControl,
            geometry: (0, 0, 20, 20),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let trusted_identity = ResourceId {
        id: format!("internal:{}", trusted.snapshot_token()),
        generation: trusted.snapshot_token(),
    };
    assert!(session.surface_capture_evidence(&trusted_identity).is_err());
    assert!(session.internal_ui.remove(runtime));
    assert!(session.surface_capture_evidence(&identity).is_err());
}

#[test]
#[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
fn pointer_targets_resolve_exact_generations_coordinates_and_protected_hits() {
    use nickel_remote_control::pointer::PointerTarget;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "fractional-rotated".into(),
            logical_width: 800,
            logical_height: 600,
            scale_120: 180,
            transform: OutputTransform::Rotate90,
        })
        .unwrap();
    session.refresh_remote_output_identities();
    let desktop = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Desktop)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&desktop];
    let placement = session.internal_ui.placement(runtime).unwrap().clone();
    let surface = PointerTarget::Surface {
        surface_id: format!("internal:{}", runtime.snapshot_token()),
        generation: runtime.snapshot_token(),
    };
    let local = (
        placement.geometry.2 as i32 / 2,
        placement.geometry.3 as i32 / 2,
    );
    let resolved = session
        .resolve_remote_pointer_target(&surface, local.0, local.1)
        .unwrap();
    assert_eq!(
        (resolved.global_x, resolved.global_y),
        (
            placement.geometry.0 + local.0,
            placement.geometry.1 + local.1
        )
    );
    assert!(resolved.surface.is_some());
    assert!(
        session
            .resolve_remote_pointer_target(&surface, -1, local.1)
            .is_err()
    );

    let generation = session.remote_output_generations["file-test"].1;
    let output = PointerTarget::Output {
        output_id: "file-test".into(),
        generation,
    };
    assert!(
        session
            .resolve_remote_pointer_target(&output, resolved.global_x, resolved.global_y)
            .unwrap()
            .output
            .is_some()
    );
    assert!(
        session
            .resolve_remote_pointer_target(&output, -1, resolved.global_y)
            .is_err()
    );
    assert!(
        session
            .resolve_remote_pointer_target(
                &PointerTarget::Output {
                    output_id: "file-test".into(),
                    generation: generation + 1,
                },
                resolved.global_x,
                resolved.global_y,
            )
            .is_err()
    );
    let desktop_target = session
        .resolve_remote_pointer_target(
            &PointerTarget::Desktop,
            resolved.global_x,
            resolved.global_y,
        )
        .unwrap();
    assert!(desktop_target.output.is_none() && desktop_target.surface.is_none());
    assert!(
        session
            .resolve_remote_pointer_target(&PointerTarget::Desktop, -1, -1)
            .is_err()
    );

    let transformed = session
        .space
        .outputs()
        .find(|output| output.name() == "fractional-rotated")
        .unwrap()
        .clone();
    let transformed_geometry = session.space.output_geometry(&transformed).unwrap();
    assert_eq!(transformed.current_scale().fractional_scale(), 1.5);
    assert_eq!(
        transformed.current_transform(),
        smithay::utils::Transform::_90
    );
    assert_eq!(transformed_geometry.size, (800, 600).into());
    let transformed_generation = session.remote_output_generations["fractional-rotated"].1;
    let transformed_target = PointerTarget::Output {
        output_id: "fractional-rotated".into(),
        generation: transformed_generation,
    };
    let transformed_point = (
        transformed_geometry.loc.x + transformed_geometry.size.w - 1,
        transformed_geometry.loc.y + transformed_geometry.size.h - 1,
    );
    let transformed_resolved = session
        .resolve_remote_pointer_target(
            &transformed_target,
            transformed_point.0,
            transformed_point.1,
        )
        .unwrap();
    assert_eq!(
        (transformed_resolved.global_x, transformed_resolved.global_y),
        transformed_point
    );
    assert!(
        session
            .resolve_remote_pointer_target(
                &transformed_target,
                transformed_geometry.loc.x - 1,
                transformed_geometry.loc.y,
            )
            .is_err(),
        "the adjacent output must stay outside a fractional transformed target"
    );

    let scaled_surface = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| {
            surface.role == crate::winit_shell::SurfaceRole::Desktop
                && surface.output.as_deref() == Some("fractional-rotated")
        })
        .map(|surface| session.internal_shell_surfaces[&surface.id])
        .expect("fractional transformed output has a production desktop surface");
    let scaled_placement = session
        .internal_ui
        .placement(scaled_surface)
        .unwrap()
        .clone();
    let scaled_surface_target = PointerTarget::Surface {
        surface_id: format!("internal:{}", scaled_surface.snapshot_token()),
        generation: scaled_surface.snapshot_token(),
    };
    let scaled_local_edge = (
        scaled_placement.geometry.2 as i32 - 1,
        scaled_placement.geometry.3 as i32 / 2,
    );
    let scaled_edge = session
        .resolve_remote_pointer_target(
            &scaled_surface_target,
            scaled_local_edge.0,
            scaled_local_edge.1,
        )
        .unwrap();
    assert_eq!(
        (scaled_edge.global_x, scaled_edge.global_y),
        (
            scaled_placement.geometry.0 + scaled_local_edge.0,
            scaled_placement.geometry.1 + scaled_local_edge.1,
        )
    );
    assert!(
        session
            .resolve_remote_pointer_target(
                &scaled_surface_target,
                scaled_placement.geometry.2 as i32,
                scaled_local_edge.1,
            )
            .is_err(),
        "surface-local coordinates cannot enter scale or decoration overflow"
    );

    session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::TrustedControl,
            geometry: (resolved.global_x - 5, resolved.global_y - 5, 10, 10),
            output: Some("file-test".into()),
        },
        1.0,
    );
    assert!(
        session
            .resolve_remote_pointer_target(&output, resolved.global_x, resolved.global_y)
            .is_err()
    );
}

#[test]
fn desktop_motion_burst_rebuilds_once_at_frame_boundary_and_focus_cancels_immediately() {
    use crate::session::internal_ui::DesktopPointerAction;
    use nickel_input::{InputEvent, KeyEdge, PointerButton};
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let desktop = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Desktop)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&desktop];
    let placement = session.internal_ui.placement(runtime).unwrap().clone();
    let point = (
        f64::from(placement.geometry.0) + f64::from(placement.geometry.2) / 2.0,
        f64::from(placement.geometry.1) + f64::from(placement.geometry.3) / 2.0,
    );
    assert!(session.internal_ui.desktop_pointer_input(
        "test",
        point,
        DesktopPointerAction::Button {
            button: PointerButton::Primary,
            edge: KeyEdge::Pressed
        },
        Default::default(),
        false
    ));
    session.flush_internal_shell_input();
    let generation = |session: &super::NickelSession| {
        session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .find(|surface| surface.id == desktop)
            .unwrap()
            .scene_generation
    };
    let before = generation(&session);
    let armed = session.internal_shell_timer_counters().armed;
    for index in 0..1000 {
        assert!(session.internal_ui.desktop_pointer_input(
            "test",
            (point.0 + f64::from(index % 40), point.1),
            DesktopPointerAction::Motion,
            Default::default(),
            false
        ));
        session.flush_internal_shell_input();
    }
    assert_eq!(
        generation(&session),
        before,
        "input dispatch must not rebuild scenes"
    );
    assert_eq!(session.pending_desktop_scenes.len(), 1);
    assert_eq!(
        session.internal_shell_timer_counters().armed,
        armed,
        "motion must not accumulate wake timers"
    );
    session.flush_desktop_scenes_for_frame();
    assert_eq!(generation(&session), before + 1);
    assert!(session.pending_desktop_scenes.is_empty());
    session.flush_desktop_scenes_for_frame();
    assert_eq!(
        generation(&session),
        before + 1,
        "idle frames must not repeat layout"
    );
    // Cancel through the production lifecycle boundary; never persist a
    // test drag into the user's desktop layout or activate a real file.
    session.internal_ui.step(
        runtime,
        nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Normalized {
                input: InputEvent::FocusLost {
                    order: nickel_input::EventOrder(2000),
                },
                clipboard_text: None,
            }],
            ..Default::default()
        },
    );
    session.flush_internal_shell_input();
    assert!(generation(&session) > before + 1);
    assert!(session.pending_desktop_scenes.is_empty());
}

#[test]
fn surface_lease_retires_with_runtime_slot_and_cannot_follow_reopened_launcher() {
    use nickel_remote_control::leases::{ResourceId, ResourceScope};
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.set_launcher_visible(true);
    let record = session
        .remote_shell_surface_diagnostics()
        .0
        .into_iter()
        .find(|record| {
            matches!(
                record.role,
                nickel_remote_control::diagnostics::ShellDiagnosticRole::Launcher
            )
        })
        .unwrap();
    let resource = ResourceId {
        id: record.id,
        generation: record.generation,
    };
    assert!(
        session
            .internal_ui
            .has_surface_identity(&resource.id, resource.generation)
    );
    assert!(!session.internal_ui.has_surface_identity(
        &format!("internal:0{}", resource.generation),
        resource.generation
    ));
    assert!(
        !session
            .internal_ui
            .has_surface_identity(&resource.id, resource.generation + 1)
    );
    let control = session.remote_control.control();
    let lease = control
        .lock()
        .unwrap()
        .leases_mut()
        .approve_local(
            "test-agent".into(),
            ResourceScope::Surface(resource.clone()),
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    session.schedule_remote_resource_retirement();
    session.flush_remote_resource_retirement();
    assert!(
        control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .any(|candidate| candidate.id == lease)
    );
    session.set_launcher_visible(false);
    assert!(
        !session
            .internal_ui
            .has_surface_identity(&resource.id, resource.generation)
    );
    session.poll_internal_shell(Instant::now());
    session.flush_remote_resource_retirement();
    assert!(
        !control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .any(|candidate| candidate.id == lease)
    );
    session.set_launcher_visible(true);
    assert!(
        !session
            .internal_ui
            .has_surface_identity(&resource.id, resource.generation)
    );
    assert!(
        !control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .any(|candidate| candidate.id == lease)
    );
}

#[test]
fn repeated_native_identity_verification_does_not_duplicate_observation_events() {
    use crate::session::{remote_identity::IdentitySource, window_registry::WindowAdmission};
    use nickel_remote_control::desktop_events::DesktopEventKind;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    let window = session.windows.insert(WindowAdmission::Ordinary).unwrap();
    for _ in 0..2 {
        session.schedule_remote_window_identity(
            window,
            IdentitySource::WaylandPeer {
                pid: std::process::id(),
                app_id: None,
            },
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.remote_window_is_protected(window) && Instant::now() < deadline {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
        }
        assert!(!session.remote_window_is_protected(window));
    }
    let verified = session
        .remote_desktop_events
        .snapshot()
        .events
        .into_iter()
        .filter(|event| {
            event.event
                == DesktopEventKind::WindowIdentityVerified {
                    window_id: window.0,
                }
        })
        .count();
    assert_eq!(verified, 1);
    session.retire_surface_window_references(None, Some(window));
    session.retire_surface_window_references(None, Some(window));
    let retired = session
        .remote_desktop_events
        .snapshot()
        .events
        .into_iter()
        .filter(|event| {
            event.event
                == DesktopEventKind::WindowRetired {
                    window_id: window.0,
                }
        })
        .count();
    assert_eq!(retired, 1);
    assert!(!session.remote_event_windows.contains(&window));
}

#[test]
fn launch_output_rejects_replaced_native_output_before_inventory_refresh() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.refresh_remote_output_identities();
    let identity = session.remote_output_identity("file-test".into()).unwrap();
    let (original, _) = session.remote_output_generations["file-test"].clone();
    assert!(session.validate_launch_output(Some(&identity)).is_ok());
    session.space.map_output(&original, (-1280, 0));
    assert!(session.validate_launch_output(Some(&identity)).is_ok());
    session.space.unmap_output(&original);
    let replacement = Output::new(
        "file-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Replacement".into(),
            serial_number: "replacement".into(),
        },
    );
    session.space.map_output(&replacement, (0, 0));
    // The published generation has not caught up yet. Comparing the name
    // alone would incorrectly authorize this replacement native object.
    assert_eq!(
        session.remote_output_identity("file-test".into()),
        Some(identity.clone())
    );
    assert!(session.validate_launch_output(Some(&identity)).is_err());
}

#[test]
fn output_lease_generation_survives_repositioning_but_not_output_retirement() {
    use nickel_remote_control::leases::{ResourceId, ResourceScope};
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.refresh_remote_output_identities();
    let (output, generation) = session.remote_output_generations["file-test"].clone();
    let capture_identity = ResourceId {
        id: "file-test".into(),
        generation,
    };
    assert_eq!(
        session.output_capture_evidence(&capture_identity).unwrap(),
        output
    );
    assert!(
        session
            .output_capture_evidence(&ResourceId {
                id: "file-test".into(),
                generation: generation.saturating_add(1),
            })
            .is_err()
    );
    let initial_history = session.remote_desktop_events.snapshot();
    let control = session.remote_control.control();
    let lease = control
        .lock()
        .unwrap()
        .leases_mut()
        .approve_local(
            "test-agent".into(),
            ResourceScope::Output(ResourceId {
                id: "file-test".into(),
                generation,
            }),
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    session.space.map_output(&output, (-1280, 0));
    session.refresh_remote_output_identities();
    session.flush_remote_resource_retirement();
    assert_eq!(session.remote_output_generations["file-test"].1, generation);
    assert!(session.output_capture_evidence(&capture_identity).is_ok());
    assert_eq!(
        session.remote_desktop_events.snapshot().generation,
        initial_history.generation,
        "repositioning must not invent an output membership transition"
    );
    assert!(
        control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .any(|candidate| candidate.id == lease)
    );
    session.space.unmap_output(&output);
    assert!(session.output_capture_evidence(&capture_identity).is_err());
    session.refresh_remote_output_identities();
    session.flush_remote_resource_retirement();
    assert!(control.lock().unwrap().leases().iter().next().is_none());
    let removed_history = session
        .remote_desktop_events
        .since(initial_history.generation)
        .unwrap();
    assert_eq!(removed_history.events.len(), 1);
    assert_eq!(
        removed_history.events[0].event,
        nickel_remote_control::desktop_events::DesktopEventKind::OutputMembershipChanged {
            latest_output_identity_generation: generation,
            outputs: 0,
        }
    );
    session.refresh_remote_output_identities();
    assert_eq!(
        session.remote_desktop_events.snapshot().generation,
        removed_history.generation,
        "rechecking unchanged membership must not duplicate events"
    );
    session.space.map_output(&output, (0, 0));
    session.refresh_remote_output_identities();
    let replacement_generation = session.remote_output_generations["file-test"].1;
    assert_ne!(replacement_generation, generation);
    assert!(session.output_capture_evidence(&capture_identity).is_err());
    assert!(
        session
            .output_capture_evidence(&ResourceId {
                id: "file-test".into(),
                generation: replacement_generation,
            })
            .is_ok()
    );
    let restored_history = session
        .remote_desktop_events
        .since(removed_history.generation)
        .unwrap();
    assert_eq!(restored_history.events.len(), 1);
    assert_eq!(
        restored_history.events[0].event,
        nickel_remote_control::desktop_events::DesktopEventKind::OutputMembershipChanged {
            latest_output_identity_generation: replacement_generation,
            outputs: 1,
        }
    );
    assert!(restored_history.events[0].observed_at_us >= removed_history.events[0].observed_at_us);
}

#[test]
fn output_capture_rejects_excessive_physical_pixel_bounds() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let output = Output::new(
        "capture-too-large".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Capture bounds".into(),
            serial_number: "capture-too-large".into(),
        },
    );
    output.change_current_state(
        Some(smithay::output::Mode {
            size: (8192, 8192).into(),
            refresh: 60_000,
        }),
        None,
        None,
        None,
    );
    session.space.map_output(&output, (1280, 0));
    session.refresh_remote_output_identities();
    let identity = session
        .remote_output_identity("capture-too-large".into())
        .unwrap();
    assert_eq!(
        session.output_capture_evidence(&identity).unwrap_err(),
        "output capture dimensions exceed limit"
    );
}

#[test]
fn removing_internal_surface_cancels_its_active_move_authority() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();

        fn update(&mut self, _: ()) {}

        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("move target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (10, 20, 300, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let subject = crate::session::grabs::move_internal_grab::operation_window(surface);
    let operation = crate::session::grabs::move_grab::WindowPointerOperation::begin(
        &mut session.window_operations,
        nickel_core::window_operation::BeginRequest {
            seat: nickel_core::window_operation::SeatId::new(1),
            subject: nickel_core::window_operation::WindowMapping {
                window: subject,
                native_lifetime: nickel_core::window_operation::NativeLifetimeId::new(
                    surface.snapshot_token(),
                ),
                generation: nickel_core::window_operation::MappingGeneration::new(
                    surface.snapshot_token(),
                ),
            },
            kind: nickel_core::window_operation::OperationKind::Move,
            control: nickel_core::geometry_authority::ControlMode::Enforced,
            origin: nickel_core::window_operation::CompletionBinding {
                source: nickel_core::window_operation::Source {
                    id: nickel_core::window_operation::SourceId::new(1),
                    generation: nickel_core::window_operation::SourceGeneration::new(1),
                },
                gesture: nickel_core::window_operation::CompletionGesture::Button(0x110),
                press_epoch: nickel_core::window_operation::PressEpoch::new(1),
            },
            optional_update_sources: Vec::new(),
        },
    )
    .expect("internal move admitted");
    let id = session
        .window_operations
        .operation_for_window(subject)
        .expect("active internal move");

    assert!(session.remove_internal_surface(surface));
    assert_eq!(
        session.window_operations.terminal_outcome(id),
        Some(nickel_core::window_operation::TerminalOutcome::Cancelled(
            nickel_core::window_operation::CancellationReason::TargetDestroyed,
        ))
    );
    assert!(operation.complete(&mut session.window_operations));
}

#[test]
fn x11_unmaximize_fence_cancels_active_resize_baseline_and_late_motion() {
    use nickel_core::{
        geometry::LogicalRect,
        geometry_authority::{ControlMode, GeometryAuthority, GeometryConstraints, Presentation},
        window_operation::{
            BeginRequest, CancellationReason, CompletionBinding, CompletionGesture, GeometrySeed,
            MappingGeneration, NativeLifetimeId, OperationKind, PressEpoch, SeatId, Source,
            SourceGeneration, SourceId, TerminalOutcome, WindowMapping,
        },
    };

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let id = crate::session::window_registry::WindowId(9_903);
    let geometry = Geometry {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    let mut authority = GeometryAuthority::new(geometry, Presentation::Normal);
    authority.set_presentation(Presentation::Maximized);
    session
        .interactive_resize_baselines
        .insert(id, authority.baseline());
    session.geometry_authorities.insert(id, authority);

    let source = Source {
        id: SourceId::new(1),
        generation: SourceGeneration::new(1),
    };
    let operation = crate::session::grabs::move_grab::WindowPointerOperation::begin_with_geometry(
        &mut session.window_operations,
        BeginRequest {
            seat: SeatId::new(1),
            subject: WindowMapping {
                window: nickel_core::window_operation::WindowId::new(id.0),
                native_lifetime: NativeLifetimeId::new(1),
                generation: MappingGeneration::new(id.0),
            },
            kind: OperationKind::Resize(
                nickel_core::window_operation::ResizeEdges::new(
                    Some(nickel_core::window_operation::HorizontalEdge::Left),
                    None,
                )
                .unwrap(),
            ),
            control: ControlMode::Cooperative,
            origin: CompletionBinding {
                source,
                gesture: CompletionGesture::Button(0x110),
                press_epoch: PressEpoch::new(1),
            },
            optional_update_sources: Vec::new(),
        },
        GeometrySeed {
            anchor: LogicalRect {
                x: geometry.x,
                y: geometry.y,
                width: geometry.width,
                height: geometry.height,
            },
            constraints: GeometryConstraints {
                min_width: 1,
                min_height: 1,
                max_width: None,
                max_height: None,
            },
        },
    )
    .expect("active X11 resize admitted");
    let operation_id = operation.id();

    // This is the shared fence called by both directions of the X11
    // maximize toggle before set_maximized/configure publishes geometry.
    session.supersede_interactive_resize_for_presentation(id);

    assert!(!session.interactive_resize_baselines.contains_key(&id));
    assert_eq!(
        session.window_operations.terminal_outcome(operation_id),
        Some(TerminalOutcome::Cancelled(CancellationReason::Superseded))
    );
    assert!(
        operation
            .propose(&mut session.window_operations, 40, 0)
            .is_none(),
        "late X11 resize motion cannot overwrite maximized geometry"
    );
}

#[test]
fn internal_move_compensation_restores_only_the_last_owned_placement() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("compensation target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (10, 20, 300, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    session
        .register_internal_application(surface)
        .expect("internal test application admitted");
    let mut moved = session.internal_ui.placement(surface).unwrap().clone();
    moved.geometry.0 = 80;
    moved.geometry.1 = 90;
    assert!(session.apply_internal_move(surface, moved));
    assert!(session.finish_internal_move(surface, true));
    assert_eq!(
        session.internal_ui.placement(surface).unwrap().geometry.0,
        10
    );

    let mut moved = session.internal_ui.placement(surface).unwrap().clone();
    moved.geometry.0 = 80;
    assert!(session.apply_internal_move(surface, moved));
    let id = session.internal_surface_windows[&surface];
    let equal_value = session.geometry_authorities[&id].base_placement.value;
    session.record_desired_geometry(id, equal_value);
    assert!(!session.finish_internal_move(surface, true));
    assert_eq!(
        session.internal_ui.placement(surface).unwrap().geometry.0,
        80
    );
}

#[test]
fn internal_application_move_crosses_output_without_losing_presentation() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("cross-output target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let second = Output::new(
        "second-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Test".into(),
            serial_number: "second-test".into(),
        },
    );
    second.change_current_state(
        Some(smithay::output::Mode {
            size: (1280, 720).into(),
            refresh: 60_000,
        }),
        None,
        Some(smithay::output::Scale::Fractional(1.5)),
        Some((1280, 0).into()),
    );
    session.space.map_output(&second, (1280, 0));
    let negative = Output::new(
        "negative-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Test".into(),
            serial_number: "negative-test".into(),
        },
    );
    negative.change_current_state(
        Some(smithay::output::Mode {
            size: (800, 720).into(),
            refresh: 60_000,
        }),
        None,
        Some(smithay::output::Scale::Fractional(0.75)),
        Some((-800, 0).into()),
    );
    session.space.map_output(&negative, (-800, 0));

    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (1060, 20, 300, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    session
        .register_internal_application(surface)
        .expect("internal application admitted");
    assert!(
        session
            .internal_ui
            .ids_for_output("file-test")
            .any(|id| id == surface)
    );
    assert!(
        session
            .internal_ui
            .ids_for_output("second-test")
            .any(|id| id == surface)
    );

    let mut moved = session.internal_ui.placement(surface).unwrap().clone();
    moved.geometry.0 = 1250;
    assert!(session.apply_internal_move(surface, moved));
    assert_eq!(
        session
            .internal_ui
            .placement(surface)
            .unwrap()
            .output
            .as_deref(),
        Some("second-test")
    );
    assert_eq!(session.internal_ui.scale_factor(surface), Some(1.5));
    assert!(
        session
            .internal_ui
            .ids_for_output("file-test")
            .any(|id| id == surface)
    );
    assert!(
        session
            .internal_ui
            .ids_for_output("second-test")
            .any(|id| id == surface)
    );
    assert!(!session.finish_internal_move(surface, false));
    let settled = session.internal_ui.placement(surface).unwrap();
    assert_eq!(settled.geometry.0, 1250);
    assert_eq!(settled.output.as_deref(), Some("second-test"));

    let mut returned = settled.clone();
    returned.geometry.0 = 1060;
    assert!(session.apply_internal_move(surface, returned));
    assert!(!session.finish_internal_move(surface, false));
    let returned = session.internal_ui.placement(surface).unwrap();
    assert_eq!(returned.output.as_deref(), Some("file-test"));
    assert_eq!(session.internal_ui.scale_factor(surface), Some(1.0));

    let mut cancelled = returned.clone();
    cancelled.geometry.0 = 1250;
    assert!(session.apply_internal_move(surface, cancelled));
    assert!(session.finish_internal_move(surface, true));
    let restored = session.internal_ui.placement(surface).unwrap();
    assert_eq!(restored.geometry.0, 1060);
    assert_eq!(restored.output.as_deref(), Some("file-test"));

    let mut negative_move = restored.clone();
    negative_move.geometry.0 = -700;
    assert!(session.apply_internal_move(surface, negative_move));
    assert!(!session.finish_internal_move(surface, false));
    let negative_settled = session.internal_ui.placement(surface).unwrap();
    assert_eq!(negative_settled.output.as_deref(), Some("negative-test"));
    assert_eq!(session.internal_ui.scale_factor(surface), Some(0.75));

    let mut final_return = negative_settled.clone();
    final_return.geometry.0 = 100;
    assert!(session.apply_internal_move(surface, final_return));
    assert!(!session.finish_internal_move(surface, false));
    let final_return = session.internal_ui.placement(surface).unwrap();
    assert_eq!(final_return.output.as_deref(), Some("file-test"));
    assert_eq!(session.internal_ui.scale_factor(surface), Some(1.0));
}

#[test]
fn removed_output_rescues_internal_window_without_replacing_its_surface() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("output removal target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let second = Output::new(
        "removed-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Test".into(),
            serial_number: "removed-test".into(),
        },
    );
    second.change_current_state(
        Some(smithay::output::Mode {
            size: (640, 480).into(),
            refresh: 60_000,
        }),
        None,
        Some(smithay::output::Scale::Fractional(1.25)),
        Some((1280, 0).into()),
    );
    session.space.map_output(&second, (1280, 0));
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (1500, 80, 500, 300),
            output: Some("removed-test".into()),
        },
        1.25,
    );
    let window = session
        .register_internal_application(surface)
        .expect("internal application admitted");

    let subject = crate::session::grabs::move_internal_grab::operation_window(surface);
    let operation = crate::session::grabs::move_grab::WindowPointerOperation::begin(
        &mut session.window_operations,
        nickel_core::window_operation::BeginRequest {
            seat: nickel_core::window_operation::SeatId::new(1),
            subject: nickel_core::window_operation::WindowMapping {
                window: subject,
                native_lifetime: nickel_core::window_operation::NativeLifetimeId::new(
                    surface.snapshot_token(),
                ),
                generation: nickel_core::window_operation::MappingGeneration::new(
                    surface.snapshot_token(),
                ),
            },
            kind: nickel_core::window_operation::OperationKind::Move,
            control: nickel_core::geometry_authority::ControlMode::Enforced,
            origin: nickel_core::window_operation::CompletionBinding {
                source: nickel_core::window_operation::Source {
                    id: nickel_core::window_operation::SourceId::new(1),
                    generation: nickel_core::window_operation::SourceGeneration::new(1),
                },
                gesture: nickel_core::window_operation::CompletionGesture::Button(0x110),
                press_epoch: nickel_core::window_operation::PressEpoch::new(1),
            },
            optional_update_sources: Vec::new(),
        },
    )
    .expect("internal move admitted");
    let operation_id = operation.id();
    let mut dragged = session.internal_ui.placement(surface).unwrap().clone();
    dragged.geometry.0 += 20;
    assert!(session.apply_internal_move(surface, dragged));
    assert!(session.internal_move_baselines.contains_key(&surface));

    session.stage_output_removal(&second);
    session.space.unmap_output(&second);
    session.reconcile_output_removal("removed-test");

    let placement = session.internal_ui.placement(surface).unwrap();
    assert_eq!(
        session.window_operations.terminal_outcome(operation_id),
        Some(nickel_core::window_operation::TerminalOutcome::Cancelled(
            nickel_core::window_operation::CancellationReason::Superseded,
        ))
    );
    assert!(!session.internal_move_baselines.contains_key(&surface));
    assert!(
        operation
            .propose(&mut session.window_operations, 60, 0)
            .is_none(),
        "late drag motion cannot overwrite output-removal recovery"
    );
    assert_eq!(session.internal_window_for_surface(surface), Some(window));
    assert_eq!(placement.output.as_deref(), Some("file-test"));
    assert_eq!(session.internal_ui.scale_factor(surface), Some(1.0));
    let frame =
        crate::session::window_frame::outer_geometry(super::internal_placement_geometry(placement));
    let work_area =
        session.work_area_for_output(session.output_geometry_named("file-test").unwrap());
    assert!(frame.x >= work_area.x);
    assert!(frame.y >= work_area.y);
    assert!(frame.x + frame.width <= work_area.x + work_area.width);
    assert!(
        session
            .internal_ui
            .ids_for_output("file-test")
            .any(|id| id == surface)
    );
}

#[test]
fn internal_resize_updates_placement_and_compensates_owned_geometry() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("resizable")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (10, 20, 300, 200),
            output: Some("resize-test".into()),
        },
        1.0,
    );
    session.register_internal_application(surface).unwrap();
    let mut resized = session.internal_ui.placement(surface).unwrap().clone();
    resized.geometry = (10, 20, 420, 280);
    assert!(session.apply_internal_resize(surface, resized));
    assert_eq!(
        session.internal_ui.placement(surface).unwrap().geometry,
        (10, 20, 420, 280)
    );
    assert!(session.finish_internal_resize(surface, true));
    assert_eq!(
        session.internal_ui.placement(surface).unwrap().geometry,
        (10, 20, 300, 200)
    );
}

#[test]
fn exposed_internal_frame_corners_use_effective_scene_hit_authority() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("corner target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (100, 140, 300, 200),
            output: Some("resize-test".into()),
        },
        1.0,
    );
    let lower = session.register_internal_application(surface).unwrap();

    assert_eq!(
        session.effective_frame_part_at((105.0, 105.0).into()),
        Some(crate::session::window_frame::FramePart::ResizeNorthWest)
    );
    assert_eq!(
        session.effective_frame_part_at((395.0, 105.0).into()),
        Some(crate::session::window_frame::FramePart::ResizeNorthEast)
    );

    let covering = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (90, 90, 220, 160),
            output: Some("resize-test".into()),
        },
        1.0,
    );
    session.register_internal_application(covering).unwrap();
    assert_eq!(
        session.effective_frame_part_at((105.0, 105.0).into()),
        None,
        "an upper window's content must occlude a lower resize corner"
    );
    session.activate_window(lower);
    assert_eq!(
        session.effective_frame_part_at((105.0, 105.0).into()),
        Some(crate::session::window_frame::FramePart::ResizeNorthWest),
        "raising the lower owner must expose its corner through production scene order"
    );
}

#[test]
fn internal_codex_chat_resize_uses_supported_logical_minimum() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let backend =
        nickel_codex::ReplayBackend::from_json(r#"{"name":"minimum","events":[]}"#).unwrap();
    let chat = nickel_codex_ui::ChatApplication::new(nickel_codex_ui::BackendMode::Replay {
        backend,
        cwd: "/projects/nickel".into(),
    });
    let surface = session.insert_internal_surface(
        chat,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (10, 20, 1120, 760),
            output: Some("resize-test".into()),
        },
        1.0,
    );
    let constraints = session.internal_resize_constraints(surface);
    assert_eq!((constraints.min_width, constraints.min_height), (640, 480));
}

#[test]
fn desired_geometry_record_does_not_reclaim_external_or_unknown_owner() {
    use crate::session::window_registry::WindowId;
    use nickel_core::geometry_authority::FieldOwner;

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let original = Geometry {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    for (raw_id, owner) in [(9_901, FieldOwner::External), (9_902, FieldOwner::Unknown)] {
        let id = WindowId(raw_id);
        session.record_desired_geometry(id, original);
        let authority = session.geometry_authorities.get_mut(&id).unwrap();
        authority.base_placement.owner = owner;
        let revision = authority.revisions().placement;

        let observed = session.record_desired_geometry(id, Geometry { x: 40, ..original });
        let authority = &session.geometry_authorities[&id];
        assert_eq!(observed, revision);
        assert_eq!(authority.base_placement.revision, revision);
        assert_eq!(authority.base_placement.value, original);
        assert_eq!(authority.base_placement.owner, owner);
    }
}

#[test]
fn lock_cancellation_releases_window_admission_without_waiting_for_settlement() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let request = nickel_core::window_operation::BeginRequest {
        seat: nickel_core::window_operation::SeatId::new(1),
        subject: nickel_core::window_operation::WindowMapping {
            window: nickel_core::window_operation::WindowId::new(900),
            native_lifetime: nickel_core::window_operation::NativeLifetimeId::new(1),
            generation: nickel_core::window_operation::MappingGeneration::new(1),
        },
        kind: nickel_core::window_operation::OperationKind::Move,
        control: nickel_core::geometry_authority::ControlMode::Enforced,
        origin: nickel_core::window_operation::CompletionBinding {
            source: nickel_core::window_operation::Source {
                id: nickel_core::window_operation::SourceId::new(1),
                generation: nickel_core::window_operation::SourceGeneration::new(1),
            },
            gesture: nickel_core::window_operation::CompletionGesture::Button(1),
            press_epoch: nickel_core::window_operation::PressEpoch::new(1),
        },
        optional_update_sources: Vec::new(),
    };
    let (operation, _) = session.window_operations.begin(request.clone());
    let operation = operation.unwrap();

    assert!(
        session.cancel_window_interactions(nickel_core::window_operation::CancellationReason::Lock)
    );
    assert!(session.window_operations.operation(operation).is_none());
    assert!(session.window_operations.begin(request).0.is_some());
}

#[test]
fn secure_lifecycle_cancels_normalized_and_internal_touch_domains_before_new_routing() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let desktop = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Desktop)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&desktop];
    let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
    let point = (f64::from(geometry.0 + 10), f64::from(geometry.1 + 10));

    assert!(session.internal_ui.normalized_touch_input(
        "physical-a",
        7,
        point,
        crate::session::TouchPhase::Started,
        false,
    ));
    assert!(session.internal_ui.touch_from_source(
        "physical-b",
        7,
        point,
        crate::session::TouchPhase::Started,
        false,
    ));
    session.cancel_all_touch_authority();

    assert!(!session.internal_ui.normalized_touch_input(
        "physical-a",
        7,
        point,
        crate::session::TouchPhase::Moved,
        false,
    ));
    assert!(!session.internal_ui.touch_from_source(
        "physical-b",
        7,
        point,
        crate::session::TouchPhase::Ended,
        false,
    ));
    session.cancel_all_touch_authority();
}

#[test]
fn native_client_touch_cancel_preserves_internal_touch_domains() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let desktop = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Desktop)
        .unwrap()
        .id;
    let runtime = session.internal_shell_surfaces[&desktop];
    let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
    let point = (f64::from(geometry.0 + 10), f64::from(geometry.1 + 10));

    assert!(session.internal_ui.normalized_touch_input(
        "surviving-output",
        9,
        point,
        crate::session::TouchPhase::Started,
        false,
    ));
    assert!(session.internal_ui.touch_from_source(
        "surviving-host",
        10,
        point,
        crate::session::TouchPhase::Started,
        false,
    ));

    session.cancel_native_client_touch_authority();

    assert!(session.internal_ui.normalized_touch_input(
        "surviving-output",
        9,
        point,
        crate::session::TouchPhase::Moved,
        false,
    ));
    assert!(session.internal_ui.touch_from_source(
        "surviving-host",
        10,
        point,
        crate::session::TouchPhase::Ended,
        false,
    ));
}

#[test]
fn internal_maximize_supersedes_active_titlebar_move() {
    #[derive(Default)]
    struct App;
    impl nickel_ui::Application for App {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("internal move target")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.insert_internal_surface(
        App,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (10, 20, 300, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    session
        .register_internal_application(surface)
        .expect("internal application admitted");
    let id = session.internal_surface_windows[&surface];
    let subject = crate::session::grabs::move_internal_grab::operation_window(surface);
    let operation = crate::session::grabs::move_grab::WindowPointerOperation::begin(
        &mut session.window_operations,
        nickel_core::window_operation::BeginRequest {
            seat: nickel_core::window_operation::SeatId::new(1),
            subject: nickel_core::window_operation::WindowMapping {
                window: subject,
                native_lifetime: nickel_core::window_operation::NativeLifetimeId::new(
                    surface.snapshot_token(),
                ),
                generation: nickel_core::window_operation::MappingGeneration::new(
                    surface.snapshot_token(),
                ),
            },
            kind: nickel_core::window_operation::OperationKind::Move,
            control: nickel_core::geometry_authority::ControlMode::Enforced,
            origin: nickel_core::window_operation::CompletionBinding {
                source: nickel_core::window_operation::Source {
                    id: nickel_core::window_operation::SourceId::new(1),
                    generation: nickel_core::window_operation::SourceGeneration::new(1),
                },
                gesture: nickel_core::window_operation::CompletionGesture::Button(0x110),
                press_epoch: nickel_core::window_operation::PressEpoch::new(1),
            },
            optional_update_sources: Vec::new(),
        },
    )
    .expect("internal move admitted");
    let operation_id = operation.id();
    let mut moved = session.internal_ui.placement(surface).unwrap().clone();
    moved.geometry.0 += 20;
    assert!(session.apply_internal_move(surface, moved));

    session.maximize_window(id);

    assert_eq!(
        session.window_operations.terminal_outcome(operation_id),
        Some(nickel_core::window_operation::TerminalOutcome::Cancelled(
            nickel_core::window_operation::CancellationReason::Superseded,
        ))
    );
    assert!(!session.internal_move_baselines.contains_key(&surface));
    assert!(
        operation
            .propose(&mut session.window_operations, 40, 20)
            .is_none(),
        "late titlebar motion cannot overwrite maximized placement"
    );
    assert!(session.internal_maximized_restore.contains_key(&id));
    let placement = session.internal_ui.placement(surface).unwrap();
    let corner = smithay::utils::Point::from((
        f64::from(placement.geometry.0 + 5),
        f64::from(placement.geometry.1 - crate::session::window_frame::TITLEBAR_HEIGHT + 5),
    ));
    assert_eq!(
        session.internal_frame_target_at(corner),
        None,
        "maximized internal windows must not advertise a resize cursor"
    );
}

#[test]
fn output_capture_rejects_visible_protected_application_content() {
    struct CaptureApp {
        protected: bool,
    }
    impl nickel_ui::Application for CaptureApp {
        type Message = ();

        fn update(&mut self, _: ()) {}

        fn remote_access_protected(&self) -> bool {
            self.protected
        }

        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("private capture fixture")
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.refresh_remote_output_identities();
    let identity = session.remote_output_identity("file-test".into()).unwrap();

    let elsewhere = session.internal_ui.insert(
        CaptureApp { protected: true },
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (1500, 0, 100, 100),
            output: Some("another-output".into()),
        },
        1.0,
    );
    assert!(session.output_capture_evidence(&identity).is_ok());
    let mut straddling = session.internal_ui.placement(elsewhere).unwrap().clone();
    straddling.geometry.0 = 1250;
    assert!(session.internal_ui.relocate(elsewhere, straddling.clone()));
    assert_eq!(
        session.output_capture_evidence(&identity).unwrap_err(),
        "output capture contains protected content"
    );
    straddling.geometry.0 = 1500;
    assert!(session.internal_ui.relocate(elsewhere, straddling));
    assert!(session.output_capture_evidence(&identity).is_ok());
    assert!(session.internal_ui.set_window_decoration(
        elsewhere,
        crate::session::internal_ui::InternalWindowDecoration {
            owner: 1,
            title: "Protected fixture".into(),
            active: false,
            maximized: false,
            background: 0xff20_2020,
            foreground: 0xffff_ffff,
        },
    ));
    let mut titlebar_only = session.internal_ui.placement(elsewhere).unwrap().clone();
    titlebar_only.geometry.0 = 100;
    titlebar_only.geometry.1 = 740;
    assert!(session.internal_ui.relocate(elsewhere, titlebar_only));
    assert_eq!(
        session.output_capture_evidence(&identity).unwrap_err(),
        "output capture contains protected content"
    );
    let mut clear = session.internal_ui.placement(elsewhere).unwrap().clone();
    clear.geometry.0 = 1500;
    clear.geometry.1 = 0;
    assert!(session.internal_ui.relocate(elsewhere, clear));
    assert!(session.output_capture_evidence(&identity).is_ok());

    let application = session.internal_ui.insert(
        CaptureApp { protected: false },
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );
    assert!(session.output_capture_evidence(&identity).is_ok());
    session
        .internal_ui
        .application_mut::<CaptureApp>(application)
        .unwrap()
        .protected = true;
    assert_eq!(
        session.output_capture_evidence(&identity).unwrap_err(),
        "output capture contains protected content"
    );

    session
        .internal_ui
        .application_mut::<CaptureApp>(application)
        .unwrap()
        .protected = false;
    let trusted = session.internal_ui.insert(
        CaptureApp { protected: true },
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::TrustedControl,
            geometry: (0, 0, 100, 40),
            output: Some("file-test".into()),
        },
        1.0,
    );
    assert!(session.internal_ui.remote_access_protected(trusted));
    assert!(session.output_capture_evidence(&identity).is_ok());

    assert!(session.internal_ui.remove(application));
    assert!(session.internal_ui.remove(elsewhere));
    assert!(session.internal_ui.remove(trusted));
}

#[test]
fn window_retirement_revokes_its_lease_after_the_current_authorized_dispatch() {
    use crate::session::window_registry::{WindowAdmission, WindowId};
    use nickel_remote_control::leases::{ResourceId, ResourceScope};
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let removed = session.windows.insert(WindowAdmission::Ordinary).unwrap();
    let retained = session.windows.insert(WindowAdmission::Ordinary).unwrap();
    let control = session.remote_control.control();
    let mut authority = control.lock().unwrap();
    authority.set_enabled(true);
    let identity = authority.connect_identity("Retirement test").unwrap();
    let scope = |id: WindowId| {
        ResourceScope::Window(ResourceId {
            id: id.0.to_string(),
            generation: id.0,
        })
    };
    let retired_lease = authority
        .leases_mut()
        .approve_local(
            identity.client_id.clone(),
            scope(removed),
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    let retained_lease = authority
        .leases_mut()
        .approve_local(
            identity.client_id,
            scope(retained),
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    // Production lifecycle runs while the outer operation still holds authorization.
    session.remote_window_identities.insert(
        removed,
        crate::session::remote_identity::WindowIdentity::Pending,
    );
    session.retire_surface_window_references(None, Some(removed));
    assert!(
        !session.remote_window_identities.contains_key(&removed),
        "retirement must bound identity storage before deferred lease cleanup"
    );
    assert!(session.remote_resource_recheck_pending);
    drop(authority);
    session.flush_remote_resource_retirement();
    let mut authority = control.lock().unwrap();
    assert_eq!(
        authority
            .leases()
            .iter()
            .map(|lease| lease.id)
            .collect::<Vec<_>>(),
        vec![retained_lease]
    );
    assert!(
        authority
            .leases_mut()
            .take_cancellations()
            .contains(&retired_lease)
    );
}

#[test]
fn remote_lease_approval_rejects_retired_and_nonexistent_generation_targets() {
    use nickel_session_protocol::{
        Command, RemoteLeaseRequest, RemoteResourceId, RemoteResourceScope, ServerMessage,
    };
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let control = session.remote_control.control();
    control.lock().unwrap().set_enabled(true);
    let surface = session.internal_ui.insert_scene(
        Vec::new(),
        super::super::internal_ui::InternalSurfacePlacement {
            role: super::super::internal_ui::InternalSurfaceRole::Panel,
            geometry: (0, 0, 100, 40),
            output: None,
        },
        1.0,
    );
    let identity = control
        .lock()
        .unwrap()
        .connect_identity("Pending retirement")
        .unwrap();
    let request = RemoteLeaseRequest {
        renewal: None,
        scope: RemoteResourceScope::Surface(RemoteResourceId {
            id: format!("internal:{}", surface.snapshot_token()),
            generation: surface.snapshot_token(),
        }),
        duration_seconds: Some(1200),
        allow_resumption: false,
        full_debug: false,
    };
    assert!(session.remote_lease_target_live(&request.scope));
    {
        let mut owner = control.lock().unwrap();
        let now = Instant::now();
        let watch = owner
            .reserve_connection_watch(&identity.client_id, &identity.token, now)
            .unwrap();
        owner
            .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
            .unwrap();
    }
    control
        .lock()
        .unwrap()
        .request_lease(
            &identity.client_id,
            &identity.token,
            request.clone().into(),
            Instant::now(),
        )
        .unwrap();
    session.internal_ui.remove(surface);
    let pending_generation = control
        .lock()
        .unwrap()
        .lease_requests()
        .pending_generation(&identity.client_id)
        .unwrap();
    let result = session.handle_protocol_command(
        Command::DecideRemoteLease {
            pending_generation,
            client_id: identity.client_id,
            request,
            allow: true,
        },
        None,
        0,
    );
    assert!(matches!(result, ServerMessage::Error { .. }));
    assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
    for scope in [
        RemoteResourceScope::Window(RemoteResourceId {
            id: u64::MAX.to_string(),
            generation: u64::MAX,
        }),
        RemoteResourceScope::Output(RemoteResourceId {
            id: "missing-output".into(),
            generation: u64::MAX,
        }),
    ] {
        let identity = control
            .lock()
            .unwrap()
            .connect_identity("Missing resource")
            .unwrap();
        let request = RemoteLeaseRequest {
            renewal: None,
            scope,
            duration_seconds: Some(1200),
            allow_resumption: false,
            full_debug: false,
        };
        {
            let mut owner = control.lock().unwrap();
            let now = Instant::now();
            let watch = owner
                .reserve_connection_watch(&identity.client_id, &identity.token, now)
                .unwrap();
            owner
                .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
                .unwrap();
        }
        control
            .lock()
            .unwrap()
            .request_lease(
                &identity.client_id,
                &identity.token,
                request.clone().into(),
                Instant::now(),
            )
            .unwrap();
        let pending_generation = control
            .lock()
            .unwrap()
            .lease_requests()
            .pending_generation(&identity.client_id)
            .unwrap();
        let result = session.handle_protocol_command(
            Command::DecideRemoteLease {
                pending_generation,
                client_id: identity.client_id,
                request,
                allow: true,
            },
            None,
            0,
        );
        assert!(matches!(result, ServerMessage::Error { .. }));
    }
    assert_eq!(control.lock().unwrap().leases().iter().count(), 0);
}

#[test]
fn remote_lease_resume_checks_live_protection_and_pending_retirement() {
    use crate::session::internal_ui::{InternalSurfacePlacement, InternalSurfaceRole};
    use nickel_session_protocol::{
        Command, RemoteLeaseAction, RemoteResourceId, RemoteResourceScope, ServerMessage,
    };
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let placement = |role| InternalSurfacePlacement {
        role,
        geometry: (0, 0, 100, 40),
        output: None,
    };
    let surface =
        session
            .internal_ui
            .insert_scene(Vec::new(), placement(InternalSurfaceRole::Panel), 1.0);
    let control = session.remote_control.control();
    let mut authority = control.lock().unwrap();
    authority.set_enabled(true);
    let identity = authority.connect_identity("Resume lifecycle").unwrap();
    let lease = authority
        .leases_mut()
        .approve_local(
            identity.client_id,
            RemoteResourceScope::Surface(RemoteResourceId {
                id: format!("internal:{}", surface.snapshot_token()),
                generation: surface.snapshot_token(),
            }),
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    drop(authority);
    let manage = |session: &mut super::NickelSession, action| {
        session.handle_protocol_command(
            Command::ManageRemoteLease {
                lease_id: lease,
                action,
            },
            None,
            0,
        )
    };
    assert!(matches!(
        manage(&mut session, RemoteLeaseAction::Pause),
        ServerMessage::RemoteControl(_)
    ));
    session
        .internal_ui
        .relocate(surface, placement(InternalSurfaceRole::TrustedControl));
    assert!(matches!(
        manage(&mut session, RemoteLeaseAction::Resume),
        ServerMessage::Error { .. }
    ));
    assert!(
        control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .find(|item| item.id == lease)
            .unwrap()
            .suspended
    );
    session
        .internal_ui
        .relocate(surface, placement(InternalSurfaceRole::Panel));
    assert!(matches!(
        manage(&mut session, RemoteLeaseAction::Resume),
        ServerMessage::RemoteControl(_)
    ));
    assert!(
        !control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .find(|item| item.id == lease)
            .unwrap()
            .suspended
    );
    manage(&mut session, RemoteLeaseAction::Pause);
    session.internal_ui.remove(surface);
    // Retirement is deferred; resume must check the owner before that queue runs.
    assert!(matches!(
        manage(&mut session, RemoteLeaseAction::Resume),
        ServerMessage::Error { .. }
    ));
    assert!(
        control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .find(|item| item.id == lease)
            .unwrap()
            .suspended
    );
}

#[test]
fn remote_lease_local_commands_approve_pause_resume_and_revoke() {
    use nickel_session_protocol::{
        Command, RemoteLeaseAction, RemoteLeaseRequest, RemoteResourceScope, ServerMessage,
    };
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let control = session.remote_control.control();
    control.lock().unwrap().set_enabled(true);
    let identity = control
        .lock()
        .unwrap()
        .connect_identity("Lease test")
        .unwrap();
    let request = RemoteLeaseRequest {
        renewal: None,
        scope: RemoteResourceScope::FullSession,
        duration_seconds: Some(1200),
        allow_resumption: false,
        full_debug: false,
    };
    {
        let mut owner = control.lock().unwrap();
        let now = Instant::now();
        let watch = owner
            .reserve_connection_watch(&identity.client_id, &identity.token, now)
            .unwrap();
        owner
            .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
            .unwrap();
    }
    control
        .lock()
        .unwrap()
        .request_lease(
            &identity.client_id,
            &identity.token,
            request.clone().into(),
            Instant::now(),
        )
        .unwrap();
    let pending_generation = control
        .lock()
        .unwrap()
        .lease_requests()
        .pending_generation(&identity.client_id)
        .unwrap();
    let result = session.handle_protocol_command(
        Command::DecideRemoteLease {
            pending_generation,
            client_id: identity.client_id,
            request,
            allow: true,
        },
        None,
        0,
    );
    let ServerMessage::RemoteControl(snapshot) = result else {
        panic!("approval failed");
    };
    assert!(snapshot.pending_leases.is_empty());
    assert_eq!(snapshot.active_leases.len(), 1);
    let lease_id = snapshot.active_leases[0].lease_id;
    for (action, suspended) in [
        (RemoteLeaseAction::Pause, true),
        (RemoteLeaseAction::Resume, false),
    ] {
        let result = session.handle_protocol_command(
            Command::ManageRemoteLease { lease_id, action },
            None,
            0,
        );
        let ServerMessage::RemoteControl(snapshot) = result else {
            panic!("management failed");
        };
        assert_eq!(snapshot.active_leases[0].suspended, suspended);
        assert_eq!(session.remote_indicator_surfaces.len(), 1);
        let indicator = *session.remote_indicator_surfaces.values().next().unwrap();
        let app = session
            .internal_ui
            .application_mut::<crate::session::remote_indicator::RemoteIndicator>(indicator)
            .unwrap();
        assert_eq!(app.grants.len(), 1);
        assert_eq!(app.grants[0].suspended, suspended);
        assert!(app.grants[0].connected);
    }
    let result = session.handle_protocol_command(
        Command::ManageRemoteLease {
            lease_id,
            action: RemoteLeaseAction::Revoke,
        },
        None,
        0,
    );
    let ServerMessage::RemoteControl(snapshot) = result else {
        panic!("revoke failed");
    };
    assert!(snapshot.active_leases.is_empty());
    assert!(session.remote_indicator_surfaces.is_empty());
}

#[test]
fn spec_0231_delayed_collectors_do_not_stall_or_commit_after_cancellation() {
    use nickel_remote_control::{
        DesktopPermit,
        diagnostics::{
            DiagnosticAction, PlatformRefreshDomain, PlatformRefreshOutcome,
            UnavailableDiagnosticDomain,
        },
        leases::ResourceScope,
    };

    fn permit(
        control: &std::sync::Arc<std::sync::Mutex<nickel_remote_control::ControlPlane>>,
        identity: &nickel_remote_control::IssuedCapability,
        lease: u64,
    ) -> DesktopPermit {
        DesktopPermit::from_active_lease(
            control.clone(),
            identity.client_id.clone(),
            identity.token.clone(),
            lease,
        )
        .unwrap()
    }

    fn owner_snapshot(
        event_loop: &mut EventLoop<'static, super::NickelSession>,
        session: &mut super::NickelSession,
        permit: DesktopPermit,
    ) -> nickel_remote_control::diagnostics::DiagnosticSnapshot {
        let authority = session.remote_desktop_authority.clone();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            sent.send(authority.diagnostic_snapshot(permit)).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        let snapshot = loop {
            event_loop
                .dispatch(Duration::from_millis(20), session)
                .unwrap();
            match received.try_recv() {
                Ok(snapshot) => break snapshot,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    assert!(Instant::now() < deadline, "owner snapshot did not complete");
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("owner snapshot worker stopped")
                }
            }
        };
        worker.join().unwrap();
        snapshot.unwrap()
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    // Give the retained platform result a deterministic age beyond the
    // protocol's five-second freshness bound without sleeping.
    session.start_time = Instant::now() - Duration::from_secs(6);
    session.remote_platform_refresh_generation = 41;
    session
        .remote_platform_refreshes
        .push(PlatformRefreshOutcome {
            domain: PlatformRefreshDomain::Audio,
            generation: 41,
            observation_started_at_us: 10,
            observed_at_us: 20,
            preparation_duration_us: 10,
            stale: false,
            network_available: false,
            bluetooth_available: false,
            audio_available: true,
            printers_available: false,
            volumes_available: false,
            filesystems_available: false,
            printer_count: 0,
            volume_count: 0,
            filesystem_count: 0,
            maintenance_available: false,
            updates_available: None,
            restart_required: None,
            firewall_healthy: None,
            malware_protection_healthy: None,
            known_permission_states: 0,
            secure_storage_status_available: false,
            associations_available: false,
            association_targets_queried: 0,
            effective_associations: 0,
            directly_writable_associations: 0,
            partial: false,
            reconciliation_confirmed: true,
        });

    let control = session.remote_control.control();
    let now = Instant::now();
    let (identity, lease) = {
        let mut owner = control.lock().unwrap();
        owner.set_enabled(true);
        let identity = owner
            .connect_identity("delayed collector scenario")
            .unwrap();
        let watch = owner
            .reserve_connection_watch(&identity.client_id, &identity.token, now)
            .unwrap();
        owner
            .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
            .unwrap();
        let lease = owner
            .leases_mut()
            .approve_local(
                identity.client_id.clone(),
                ResourceScope::FullSession,
                now,
                Some(now + Duration::from_secs(1200)),
                false,
                true,
            )
            .unwrap();
        (identity, lease)
    };

    // Hold the exact single-flight admission owned by production platform
    // preparation. Owner input, renderer preparation, and snapshots must
    // all complete before this delayed collector is released.
    let delayed_staging = session.remote_diagnostic_staging.clone();
    let delayed = delayed_staging.acquire().unwrap();
    let pointer_before = session.seat.get_pointer().unwrap().current_location();
    session
        .inject_test_input(nickel_session_protocol::TestInput::PointerMove { x: 320, y: 240 })
        .unwrap();
    assert_ne!(
        session.seat.get_pointer().unwrap().current_location(),
        pointer_before
    );

    let renderer = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );
    session.register_internal_application(renderer).unwrap();
    let frames_before = session
        .internal_ui
        .renderer_diagnostics(renderer)
        .unwrap()
        .gpu_frames;
    session.internal_ui.mark_dirty(renderer);
    let _ = session.internal_ui.render_buffer(renderer);
    let frames_after = session
        .internal_ui
        .renderer_diagnostics(renderer)
        .unwrap()
        .gpu_frames;
    assert!(frames_after > frames_before);

    let pending = owner_snapshot(
        &mut event_loop,
        &mut session,
        permit(&control, &identity, lease),
    );
    let worker = pending.diagnostic_worker.as_ref().unwrap();
    assert!(worker.busy);
    assert_eq!(worker.generation, 1);
    assert!(worker.last_changed_uptime_us <= worker.collector_uptime_us);
    let retained = pending
        .platform_refreshes
        .iter()
        .find(|refresh| refresh.domain == PlatformRefreshDomain::Audio)
        .unwrap();
    assert_eq!(retained.generation, 41);
    assert_eq!(retained.observation_started_at_us, 10);
    assert_eq!(retained.observed_at_us, 20);
    assert!(retained.stale);
    let rendered = pending
        .internal_renderers
        .iter()
        .find(|entry| entry.surface_generation == renderer.snapshot_token())
        .unwrap();
    assert_eq!(rendered.observed_at_us, pending.observed_at_us);
    assert!(rendered.gpu_frames >= frames_after);
    assert_eq!(
        pending.input.observation_generation,
        pending.observation_generation
    );
    assert_eq!(pending.input.observed_at_us, pending.observed_at_us);
    let application = pending
        .internal_applications
        .iter()
        .find(|entry| entry.generation == renderer.snapshot_token())
        .unwrap();
    assert_eq!(
        pending
            .input
            .pointer_hit_test
            .as_ref()
            .and_then(|hit| hit.window.as_deref()),
        Some(application.window.as_str())
    );
    let shared_cache = pending.shared_presenter_cache.as_ref().unwrap();
    assert_eq!(
        shared_cache.observation_generation,
        pending.observation_generation
    );
    assert_eq!(shared_cache.observed_at_us, pending.observed_at_us);
    assert_eq!(shared_cache.cache_owners, 1);
    assert!(shared_cache.host_texture_allocations.is_some());
    assert!(shared_cache.host_texture_uploads.is_some());
    let shared_json = serde_json::to_string(shared_cache).unwrap();
    for excluded in ["PRIVATE", "surface", "path", "pixels", "key"] {
        assert!(!shared_json.contains(excluded));
    }
    assert!(pending.native_presentation_dispatch.is_none());
    for unavailable in [
        UnavailableDiagnosticDomain::NativeGpuCompletionTiming,
        UnavailableDiagnosticDomain::GpuDriverAndExternalRendererResources,
        UnavailableDiagnosticDomain::SharedRendererCachePerSurfaceAttribution,
    ] {
        assert!(pending.unavailable_domains.contains(&unavailable));
    }

    // Lock contention is represented as unavailable instead of waiting on
    // a collector. The snapshot still returns through the production owner.
    drop(delayed);
    let staging = session.remote_diagnostic_staging.clone();
    let (held, held_ready) = std::sync::mpsc::sync_channel(1);
    let (release, released) = std::sync::mpsc::sync_channel(1);
    let contention = std::thread::spawn(move || {
        staging.with_snapshot_state_held(|| {
            held.send(()).unwrap();
            released.recv().unwrap();
        });
    });
    held_ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let unavailable = owner_snapshot(
        &mut event_loop,
        &mut session,
        permit(&control, &identity, lease),
    );
    assert!(unavailable.diagnostic_worker.is_none());
    assert!(unavailable.observation_generation > pending.observation_generation);
    release.send(()).unwrap();
    contention.join().unwrap();

    // Model the worker's delayed result at the production commit seam.
    // Revoking its lease first must reject the result without advancing or
    // replacing the retained platform generation.
    let late_permit = permit(&control, &identity, lease);
    control.lock().unwrap().leases_mut().revoke(lease);
    assert!(late_permit.check_live().is_err());
    let (reply, result) = std::sync::mpsc::sync_channel(1);
    session.handle_remote_desktop_request(super::RemoteDesktopRequest::DiagnosticAction {
        permit: late_permit,
        action: DiagnosticAction::RefreshPlatformStatus {
            domain: PlatformRefreshDomain::Audio,
        },
        application_discovery: None,
        platform_refresh: Some(super::PreparedPlatformRefresh {
            domain: PlatformRefreshDomain::Audio,
            data: super::PreparedPlatformRefreshData::Audio(crate::platform::AudioRefresh {
                audio: crate::platform::AudioStatus {
                    available: true,
                    ..Default::default()
                },
                partial: false,
            }),
            observation_started: Instant::now() - Duration::from_millis(5),
            observed: Instant::now(),
            preparation_duration_us: 5_000,
        }),
        reply,
    });
    assert!(
        result
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_err()
    );
    assert_eq!(session.remote_platform_refresh_generation, 41);
    assert_eq!(session.remote_platform_refreshes.len(), 1);
    assert_eq!(session.remote_platform_refreshes[0].generation, 41);
}

#[test]
fn active_remote_lease_owns_one_overlay_per_output_and_revoke_removes_it() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let control = session.remote_control.control();
    let mut control_guard = control.lock().unwrap();
    control_guard.set_enabled(true);
    let pairing = control_guard.start_pairing(10).unwrap();
    let pending = control_guard
        .exchange_short_code(
            &pairing.ceremony_id,
            &pairing.short_code,
            "Indicator test",
            vec![nickel_remote_control::Capability::Observe],
            11,
        )
        .unwrap();
    control_guard
        .approve(
            &pending.id,
            nickel_remote_control::Approval::AllowOnce,
            vec![nickel_remote_control::Capability::Observe],
        )
        .unwrap();
    drop(control_guard);

    session.sync_remote_control_indicators();
    assert!(session.remote_indicator_surfaces.is_empty());
    control
        .lock()
        .unwrap()
        .leases_mut()
        .approve_local(
            pending.id.clone(),
            nickel_remote_control::leases::ResourceScope::FullSession,
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    session.sync_remote_control_indicators();
    assert_eq!(session.remote_indicator_surfaces.len(), 1);
    let indicator = session.remote_indicator_surfaces["file-test"];
    let placement = session.internal_ui.placement(indicator).unwrap();
    assert_eq!(
        placement.role,
        crate::session::InternalSurfaceRole::TrustedControl
    );
    assert_eq!(session.remote_indicator_accessibility.len(), 1);
    assert!(session.internal_ui.remote_access_protected(indicator));
    assert_eq!(
        session
            .internal_ui
            .bounded_application_semantics(indicator)
            .unwrap_err(),
        "hosted application semantics unavailable"
    );
    assert_eq!(placement.output.as_deref(), Some("file-test"));
    assert!(
        session
            .internal_ui
            .surface_at((900.0, 30.0), true)
            .is_some()
    );

    let stop = session
        .internal_ui
        .semantic_nodes(indicator)
        .into_iter()
        .find(|node| node.name.as_deref() == Some("Stop"))
        .expect("trusted indicator exposes an accessible Stop button");
    assert!(stop.actions.contains(&nickel_ui::ActionKind::Activate));
    session
        .internal_ui
        .perform_accessibility_action(
            indicator,
            stop.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        )
        .unwrap();
    assert!(
        session
            .internal_ui
            .application_mut::<super::super::remote_indicator::RemoteIndicator>(indicator)
            .unwrap()
            .stop_requested
    );
    assert!(control.lock().unwrap().revoke(&pending.id));
    session.sync_remote_control_indicators();
    assert!(session.remote_indicator_surfaces.is_empty());
    assert!(session.remote_indicator_accessibility.is_empty());
    assert!(session.internal_ui.placement(indicator).is_none());

    let pairing = control.lock().unwrap().start_pairing(20).unwrap();
    let pending = control
        .lock()
        .unwrap()
        .exchange_short_code(
            &pairing.ceremony_id,
            &pairing.short_code,
            "Lock test",
            vec![nickel_remote_control::Capability::Observe],
            21,
        )
        .unwrap();
    control
        .lock()
        .unwrap()
        .approve(
            &pending.id,
            nickel_remote_control::Approval::AllowOnce,
            vec![nickel_remote_control::Capability::Observe],
        )
        .unwrap();
    control
        .lock()
        .unwrap()
        .leases_mut()
        .approve_local(
            pending.id.clone(),
            nickel_remote_control::leases::ResourceScope::FullSession,
            Instant::now(),
            None,
            false,
            false,
        )
        .unwrap();
    session.sync_remote_control_indicators();
    assert_eq!(session.remote_indicator_surfaces.len(), 1);
    session.locked = true;
    session.remote_control.lock();
    session.sync_remote_control_indicators();
    assert!(session.remote_indicator_surfaces.is_empty());
    assert!(control.lock().unwrap().granted_clients().next().is_none());
}

#[cfg(target_os = "linux")]
struct NativeAtspiStatus {
    session: zbus::blocking::Connection,
    original_enabled: bool,
}

#[cfg(target_os = "linux")]
impl NativeAtspiStatus {
    fn connect() -> Result<Self, String> {
        let session = zbus::blocking::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_secs(1))
            .build()
            .map_err(|error| error.to_string())?;
        let original_enabled = Self::enabled(&session)?;
        Ok(Self {
            session,
            original_enabled,
        })
    }

    fn enabled(session: &zbus::blocking::Connection) -> Result<bool, String> {
        let value: zbus::zvariant::OwnedValue = session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.a11y.Status", "IsEnabled"),
            )
            .map_err(|error| error.to_string())?
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        bool::try_from(value).map_err(|error| error.to_string())
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &(
                    "org.a11y.Status",
                    "IsEnabled",
                    zbus::zvariant::Value::from(enabled),
                ),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn accessibility_bus(&self) -> Result<zbus::blocking::Connection, String> {
        let address: String = self
            .session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.a11y.Bus"),
                "GetAddress",
                &(),
            )
            .map_err(|error| error.to_string())?
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        zbus::blocking::connection::Builder::address(address.as_str())
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_secs(1))
            .build()
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
impl Drop for NativeAtspiStatus {
    fn drop(&mut self) {
        let _ = self.set_enabled(self.original_enabled);
    }
}

#[cfg(target_os = "linux")]
fn atspi_children(
    bus: &zbus::blocking::Connection,
    peer: &str,
    path: &str,
) -> Result<Vec<(String, zbus::zvariant::OwnedObjectPath)>, String> {
    bus.call_method(
        Some(peer),
        path,
        Some("org.a11y.atspi.Accessible"),
        "GetChildren",
        &(),
    )
    .map_err(|error| error.to_string())?
    .body()
    .deserialize()
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn atspi_name(bus: &zbus::blocking::Connection, peer: &str, path: &str) -> Result<String, String> {
    let value: zbus::zvariant::OwnedValue = bus
        .call_method(
            Some(peer),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.atspi.Accessible", "Name"),
        )
        .map_err(|error| error.to_string())?
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    String::try_from(value).map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn atspi_peer_is_current_process(bus: &zbus::blocking::Connection, peer: &str) -> bool {
    bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetConnectionUnixProcessID",
        &(peer,),
    )
    .ok()
    .and_then(|message| message.body().deserialize::<u32>().ok())
        == Some(std::process::id())
}

#[cfg(target_os = "linux")]
fn atspi_action_count(bus: &zbus::blocking::Connection, peer: &str, path: &str) -> Option<i32> {
    let value: zbus::zvariant::OwnedValue = bus
        .call_method(
            Some(peer),
            path,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.atspi.Action", "NActions"),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()?;
    i32::try_from(value).ok()
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires live session and AT-SPI buses; temporarily toggles accessibility status"]
fn native_atspi_consumer_observes_mcp_excluded_indicator_and_invokes_stop() {
    use nickel_remote_control::{DesktopPermit, leases::ResourceScope};

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let status = NativeAtspiStatus::connect().expect("live session accessibility service");
    status
        .set_enabled(false)
        .expect("disable accessibility before adapter registration");

    let (_event_loop, mut session) = internal_shell_test_session();
    let control = session.remote_control.control();
    let now = Instant::now();
    let (identity, lease) = {
        let mut owner = control.lock().unwrap();
        owner.set_enabled(true);
        let identity = owner.connect_identity("Native AT-SPI acceptance").unwrap();
        let watch = owner
            .reserve_connection_watch(&identity.client_id, &identity.token, now)
            .unwrap();
        owner
            .activate_connection_watch(&identity.client_id, &identity.token, watch, false, now)
            .unwrap();
        let lease = owner
            .leases_mut()
            .approve_local(
                identity.client_id.clone(),
                ResourceScope::FullSession,
                now,
                Some(now + Duration::from_secs(120)),
                false,
                false,
            )
            .unwrap();
        (identity, lease)
    };
    session.sync_remote_control_indicators();
    let indicator = *session
        .remote_indicator_surfaces
        .values()
        .next()
        .expect("production trusted indicator");

    let permit = DesktopPermit::from_active_lease(
        control.clone(),
        identity.client_id,
        identity.token,
        lease,
    )
    .unwrap();
    let remote_surfaces = session.list_remote_surfaces(&permit).unwrap();
    let protected_id = format!("internal:{}", indicator.snapshot_token());
    assert!(
        remote_surfaces
            .iter()
            .all(|surface| surface.id != protected_id),
        "MCP surface inventory exposed the trusted indicator"
    );

    // AccessKit starts its session-bus listener asynchronously. Toggle only
    // after the production adapter exists so the property change activates
    // and registers that exact adapter on the real AT-SPI bus.
    std::thread::sleep(Duration::from_millis(100));
    status
        .set_enabled(true)
        .expect("activate live accessibility service");
    let bus = status.accessibility_bus().expect("live AT-SPI bus");
    let deadline = Instant::now() + Duration::from_secs(5);
    let (indicator_peer, indicator_path, stop_peer, stop_path, labels) = loop {
        let applications = atspi_children(
            &bus,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/accessible/root",
        )
        .unwrap_or_default();
        let mut found = None;
        for (peer, application_path) in applications {
            if !atspi_peer_is_current_process(&bus, &peer) {
                continue;
            }
            let Ok(windows) = atspi_children(&bus, &peer, application_path.as_str()) else {
                continue;
            };
            for (window_peer, window_path) in windows {
                if atspi_name(&bus, &window_peer, window_path.as_str()).as_deref()
                    != Ok("Nickel Remote AI Control")
                {
                    continue;
                }
                let children = atspi_children(&bus, &window_peer, window_path.as_str())
                    .expect("indicator children");
                let mut labels = Vec::new();
                let mut stop = None;
                for (child_peer, child_path) in children {
                    let label = atspi_name(&bus, &child_peer, child_path.as_str())
                        .expect("indicator child name");
                    if label == "Stop"
                        && atspi_action_count(&bus, &child_peer, child_path.as_str()) == Some(1)
                    {
                        stop = Some((child_peer.clone(), child_path.clone()));
                    }
                    labels.push(label);
                }
                found = stop.map(|(stop_peer, stop_path)| {
                    (
                        window_peer.clone(),
                        window_path.clone(),
                        stop_peer,
                        stop_path,
                        labels,
                    )
                });
                if found.is_some() {
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        if let Some(found) = found {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "production AT-SPI indicator unavailable"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(labels.iter().any(|label| label == "Stop"));
    let safe_labels = labels
        .iter()
        .filter(|label| !label.starts_with("Peer:"))
        .collect::<Vec<_>>();
    assert!(
        labels
            .iter()
            .any(|label| label == "Client: Native AT-SPI acceptance"),
        "unexpected non-peer indicator labels: {safe_labels:?}"
    );
    assert_eq!(
        atspi_name(&bus, &indicator_peer, indicator_path.as_str()).unwrap(),
        "Nickel Remote AI Control"
    );
    assert_eq!(
        atspi_action_count(&bus, &stop_peer, stop_path.as_str()),
        Some(1)
    );
    let invoked: bool = bus
        .call_method(
            Some(stop_peer.as_str()),
            stop_path.as_str(),
            Some("org.a11y.atspi.Action"),
            "DoAction",
            &(0_i32,),
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap();
    assert!(invoked, "AT-SPI provider rejected Stop");

    let deadline = Instant::now() + Duration::from_secs(2);
    while control.lock().unwrap().leases().iter().next().is_some() {
        session.sync_remote_control_indicators();
        assert!(
            Instant::now() < deadline,
            "owner did not dispatch the native Stop action"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(session.remote_indicator_accessibility.is_empty());
    assert!(session.remote_indicator_surfaces.values().all(|surface| {
        session
            .internal_ui
            .application::<super::super::remote_indicator::RemoteIndicator>(*surface)
            .is_some_and(|indicator| indicator.grants.is_empty() && indicator.stopped_confirmation)
    }));
}

#[test]
fn file_open_focus_close_uses_the_canonical_application_lifecycle() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let directory = tempfile::tempdir().unwrap();
    let request = nickel_file::FileWindowRequest::OpenOrFocus(nickel_file::FileLaunch::Browse(
        directory.path().into(),
    ));
    let action = session
        .internal_shell
        .as_mut()
        .unwrap()
        .file_windows_mut()
        .handle(request.clone());
    session.apply_internal_file_action(action);
    let (&file, &surface) = session.internal_file_surfaces.iter().next().unwrap();
    let window = session.internal_window_for_surface(surface).unwrap();
    assert!(matches!(
        session.ordinary_scene_order().first(),
        Some(super::OrdinarySceneWindow::Internal(id)) if *id == surface
    ));
    assert_eq!(session.internal_ui.focused(), Some(surface));
    let application_deadline = session.internal_ui.surface_deadline(surface).unwrap();
    assert!(
        session.internal_shell_timer.deadline <= Some(application_deadline),
        "the shell timer must wake compositor-hosted file applications"
    );
    let (x, y, width, height) = session.internal_ui.placement(surface).unwrap().geometry;
    assert!(
        session
            .internal_ui
            .application_covers(surface, (f64::from(x + 1), f64::from(y + 1)))
    );
    assert!(!session.internal_ui.application_covers(
        surface,
        (
            f64::from(x + i32::try_from(width).unwrap() + 10),
            f64::from(y + i32::try_from(height).unwrap() + 10)
        )
    ));
    assert!(
        session
            .protocol_windows()
            .iter()
            .any(|item| item.id.0 == window.0 && item.application_id == "nickel-file")
    );
    session.minimize_window(window);
    let action = session
        .internal_shell
        .as_mut()
        .unwrap()
        .file_windows_mut()
        .handle(request.clone());
    session.apply_internal_file_action(action);
    assert!(session.internal_ui.is_visible(surface));
    session.close_window(window);
    assert!(!session.windows.contains(window));
    assert!(!session.internal_file_surfaces.contains_key(&file));
    let action = session
        .internal_shell
        .as_mut()
        .unwrap()
        .file_windows_mut()
        .handle(request);
    assert!(matches!(action, nickel_file::FileWindowAction::Opened(_)));
}

#[test]
fn integrated_file_context_menu_is_a_detached_clickable_overlay() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("rename-me.txt"), b"test").unwrap();
    let action = session
        .internal_shell
        .as_mut()
        .unwrap()
        .file_windows_mut()
        .handle(nickel_file::FileWindowRequest::OpenOrFocus(
            nickel_file::FileLaunch::Browse(directory.path().into()),
        ));
    session.apply_internal_file_action(action);
    let parent = *session.internal_file_surfaces.values().next().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while session
        .internal_ui
        .application::<nickel_file::FileApp>(parent)
        .unwrap()
        .navigation_pending()
        && Instant::now() < deadline
    {
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
    }
    let entry = session
        .internal_ui
        .semantic_nodes(parent)
        .into_iter()
        .find(|node| node.name.as_deref() == Some("rename-me.txt"))
        .expect("file entry");
    let parent_geometry = session.internal_ui.placement(parent).unwrap().geometry;
    session.internal_ui.step(
        parent,
        nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Normalized {
                input: nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
                    device: nickel_input::DeviceId(1),
                    order: nickel_input::EventOrder(1),
                    position: nickel_input::Point { x: 850.0, y: 610.0 },
                    delta: None,
                }),
                clipboard_text: None,
            }],
            ..Default::default()
        },
    );
    session.internal_ui.step(
        parent,
        nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Semantic {
                target: entry.id,
                action: nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::ContextMenu),
            }],
            ..Default::default()
        },
    );
    session.reconcile_internal_file_context_popup();
    let (popup, owner) = session.internal_file_context_popup.expect("detached popup");
    assert_eq!(owner, parent);
    let popup_geometry = session.internal_ui.placement(popup).unwrap().geometry;
    assert!(
        popup_geometry.0 + popup_geometry.2 as i32 > parent_geometry.0 + parent_geometry.2 as i32
    );
    assert_eq!(
        session.internal_ui.placement(popup).unwrap().role,
        crate::session::InternalSurfaceRole::Overlay
    );
    assert!(session.internal_ui.open_overlay(popup).is_some());
    assert!(session.internal_ui.open_overlay(parent).is_none());
    let rename = session
        .internal_ui
        .semantic_nodes(popup)
        .into_iter()
        .find(|node| node.name.as_deref() == Some("Rename"))
        .expect("rename menu item");
    let geometry = session.internal_ui.placement(popup).unwrap().geometry;
    let point = (
        f64::from(geometry.0) + f64::from(rename.bounds.origin.x + rename.bounds.size.width / 2.0),
        f64::from(geometry.1) + f64::from(rename.bounds.origin.y + rename.bounds.size.height / 2.0),
    );
    for edge in [
        nickel_input::KeyEdge::Pressed,
        nickel_input::KeyEdge::Released,
    ] {
        assert!(session.internal_ui.desktop_pointer_input(
            "test-context-menu",
            point,
            crate::session::internal_ui::DesktopPointerAction::Button {
                button: nickel_input::PointerButton::Primary,
                edge,
            },
            Default::default(),
            false,
        ));
    }
    session.reconcile_internal_file_context_popup();
    assert!(session.internal_file_context_popup.is_none());
    assert!(
        session
            .internal_ui
            .application::<nickel_file::FileApp>(parent)
            .unwrap()
            .rename_in_progress()
    );
}

#[test]
fn integrated_file_initial_location_completes_on_shell_timer() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    let directory = tempfile::tempdir().unwrap();
    let action = session
        .internal_shell
        .as_mut()
        .unwrap()
        .file_windows_mut()
        .handle(nickel_file::FileWindowRequest::OpenOrFocus(
            nickel_file::FileLaunch::Browse(directory.path().into()),
        ));
    session.apply_internal_file_action(action);
    let surface = *session.internal_file_surfaces.values().next().unwrap();
    let loading = |session: &super::NickelSession| {
        session
            .internal_ui
            .application::<nickel_file::FileApp>(surface)
            .unwrap()
            .navigation_pending()
    };
    assert!(loading(&session));
    let timeout = Instant::now() + Duration::from_secs(2);
    while loading(&session) && Instant::now() < timeout {
        event_loop
            .dispatch(Duration::from_millis(25), &mut session)
            .unwrap();
    }
    assert!(!loading(&session), "file navigation did not finish");
}

impl nickel_ui::Application for InternalWindowTestApp {
    type Message = ();

    fn update(&mut self, (): ()) {}

    fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
        nickel_ui::Text::new("internal window")
    }

    fn title(&self) -> &str {
        "Codex — Nickel"
    }
}

impl nickel_ui::Application for InternalHitTestApp {
    type Message = ();

    fn update(&mut self, (): ()) {}

    fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
        nickel_ui::Button::new((), "internal action")
    }

    fn title(&self) -> &str {
        "Internal hit test"
    }
}

#[test]
fn admitted_internal_window_produces_real_switcher_preview_pixels() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.internal_ui.insert(
        InternalHitTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (200, 150, 250, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let window = session.register_internal_application(surface).unwrap();
    session.set_switcher_preview_interest(vec![window]);

    let wave = session.begin_preview_render_wave();
    assert!(session.preview_capture_candidates(wave).is_empty());
    let frame = session
        .preview_frames
        .get(&window)
        .expect("internal preview is captured without a Wayland client window");
    assert_eq!((frame.width, frame.height), (168, 135));
    assert_eq!(
        frame.rgba.len(),
        usize::from(frame.width) * usize::from(frame.height) * 4
    );
    assert!(
        frame
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[3] != 0 && pixel[..3] != [0, 0, 0]),
        "the preview must contain the hosted application's rendered pixels"
    );
}

#[test]
fn effective_hit_order_tracks_internal_activation_and_privileged_overlay() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let placement = crate::session::InternalSurfacePlacement {
        role: crate::session::InternalSurfaceRole::Application,
        geometry: (200, 150, 250, 200),
        output: Some("file-test".into()),
    };
    let first = session
        .internal_ui
        .insert(InternalHitTestApp, placement.clone(), 1.0);
    let first_window = session.register_internal_application(first).unwrap();
    let second = session
        .internal_ui
        .insert(InternalHitTestApp, placement, 1.0);
    session.register_internal_application(second).unwrap();
    let point = (320.0, 260.0).into();
    assert!(matches!(
        session.effective_scene_hit_at(point),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == second
    ));
    session.activate_window(first_window);
    assert!(matches!(
        session.effective_scene_hit_at(point),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == first
    ));
    assert!(session.pointer_surface_under(point).is_none());

    let overlay = session.internal_ui.insert(
        InternalHitTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Overlay,
            geometry: (300, 240, 100, 70),
            output: Some("file-test".into()),
        },
        1.0,
    );
    assert!(matches!(
        session.effective_scene_hit_at(point),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == overlay
    ));
    assert!(!session.client_scene_foremost_at(point));
}

#[test]
fn semantic_window_click_raises_only_the_exposed_internal_target() {
    use nickel_session_protocol::{PointerInteraction, TestInput, WindowId as ProtocolWindowId};

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let first = session.internal_ui.insert(
        InternalHitTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (200, 150, 250, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let first_window = session.register_internal_application(first).unwrap();
    let second = session.internal_ui.insert(
        InternalHitTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (260, 150, 250, 200),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let second_window = session.register_internal_application(second).unwrap();
    assert_eq!(
        session.windows.snapshot().last().map(|window| window.id),
        Some(second_window)
    );

    session
        .inject_test_input(TestInput::WindowPointer {
            window: ProtocolWindowId(first_window.0),
            interaction: PointerInteraction::LeftClick,
        })
        .unwrap();
    assert_eq!(
        session.windows.snapshot().last().map(|window| window.id),
        Some(first_window)
    );
    assert!(matches!(
        session.effective_scene_hit_at((300.0, 250.0).into()),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == first
    ));

    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext);
    assert_eq!(session.task_switcher.selected(), Some(&second_window));
    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CancelSwitch);
    assert_eq!(
        session.windows.snapshot().last().map(|window| window.id),
        Some(first_window),
        "cancelling Alt+Tab must preserve the existing front window"
    );
    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext);
    assert_eq!(session.task_switcher.selected(), Some(&second_window));
    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CommitSwitch);
    assert_eq!(
        session.windows.snapshot().last().map(|window| window.id),
        Some(second_window),
        "committing Alt+Tab must raise only its selected window"
    );
    assert!(matches!(
        session.effective_scene_hit_at((300.0, 250.0).into()),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == second
    ));
    session.activate_window(first_window);

    session.minimize_window(first_window);
    let minimized_order = session.ordinary_scene_order();
    assert_eq!(minimized_order.len(), 1);
    assert!(matches!(
        minimized_order.as_slice(),
        [super::OrdinarySceneWindow::Internal(id)] if *id == second
    ));
    assert!(matches!(
        session.effective_scene_hit_at((300.0, 250.0).into()),
        Some(super::OrdinarySceneWindow::Internal(id)) if id == second
    ));

    session.activate_window(first_window);
    let restored_order = session.ordinary_scene_order();
    assert_eq!(restored_order.len(), 2);
    assert!(matches!(
        restored_order.as_slice(),
        [
            super::OrdinarySceneWindow::Internal(front),
            super::OrdinarySceneWindow::Internal(back),
        ] if *front == first && *back == second
    ));

    let original_workspace = session.workspaces.active();
    let other_workspace = session.workspaces.create().unwrap();
    let hide = session.workspaces.switch_to(other_workspace, None).unwrap();
    session.apply_workspace_transition(hide);
    assert!(session.ordinary_scene_order().is_empty());
    assert!(
        session
            .effective_scene_hit_at((300.0, 250.0).into())
            .is_none()
    );
    let show = session
        .workspaces
        .switch_to(original_workspace, None)
        .unwrap();
    session.apply_workspace_transition(show);
    let returned_order = session.ordinary_scene_order();
    assert_eq!(returned_order.len(), 2);
    assert!(matches!(
        returned_order.as_slice(),
        [
            super::OrdinarySceneWindow::Internal(front),
            super::OrdinarySceneWindow::Internal(back),
        ] if *front == first && *back == second
    ));

    session.close_window(first_window);
    let closed_order = session.ordinary_scene_order();
    assert_eq!(closed_order.len(), 1);
    assert!(matches!(
        closed_order.as_slice(),
        [super::OrdinarySceneWindow::Internal(id)] if *id == second
    ));
}

#[test]
fn lock_retires_pending_and_visible_task_switch_peek() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    for x in [200, 500] {
        let surface = session.internal_ui.insert(
            InternalHitTestApp,
            crate::session::InternalSurfacePlacement {
                role: crate::session::InternalSurfaceRole::Application,
                geometry: (x, 150, 250, 200),
                output: Some("file-test".into()),
            },
            1.0,
        );
        session.register_internal_application(surface).unwrap();
    }

    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext);
    assert!(session.task_switcher.session().is_some());
    assert!(session.task_switcher.peek_deadline().is_some());
    assert!(session.poll_task_switcher_peek(Instant::now() + Duration::from_secs(1)));
    assert!(session.preview_highlight.is_some());

    session.lock_session();

    assert!(session.task_switcher.session().is_none());
    assert!(session.task_switcher.peek_deadline().is_none());
    assert!(session.task_switcher.peeked().is_none());
    assert!(session.preview_highlight.is_none());
}

#[test]
fn hosted_app_clipboard_limit_is_ready_before_first_input() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, session) = preview_test_session();
    assert_eq!(
        session.internal_ui.clipboard_limit(),
        session.native_clipboard.text_limit.unwrap()
    );
    assert!(session.internal_ui.clipboard_limit() > 0);
}

#[test]
fn asynchronous_native_paste_rejects_field_transfer_and_return() {
    use image::ImageEncoder;
    use nickel_ui::{UiEvent, id, ui};
    use std::io::Write;
    #[derive(Default)]
    struct Fields {
        first: String,
        second: String,
    }
    impl nickel_ui::Application for Fields {
        type Message = (bool, String);
        fn update(&mut self, (second, text): Self::Message) {
            if second {
                self.second = text;
            } else {
                self.first = text;
            }
        }
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
            ui! { <Column>
                <TextField id={id!(first)} value={&self.first} on_change={|text| (false, text)} />
                <TextField id={id!(second)} value={&self.second} on_change={|text| (true, text)} />
            </Column> }
        }
        fn title(&self) -> &str {
            "Paste field lease"
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let recipient = session.internal_ui.insert(
        Fields::default(),
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (0, 0, 640, 480),
            output: None,
        },
        1.0,
    );
    session.configure_on_screen_keyboard(true, true, 41, false, false, 368);
    session.native_clipboard.text_limit = Some(8);
    session.focus_internal_surface(recipient);
    session.internal_ui.keyboard(UiEvent::FocusNext);
    let epoch = session.on_screen_keyboard_snapshot().epoch;
    let key = crate::session::input::internal_virtual_key(
        'v' as u32,
        &[0xffe3],
        nickel_input::EventOrder(1),
    )
    .unwrap();
    for return_to_original in [false, true] {
        let original = session.internal_ui.focused_field_lease(recipient).unwrap();
        let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let permit = session.native_clipboard.reads.acquire(1).unwrap();
        session
            .begin_native_paste_read(reader.into(), epoch, key.clone(), recipient, 8, permit)
            .unwrap();
        session.internal_ui.keyboard(UiEvent::FocusNext);
        if return_to_original {
            session.internal_ui.keyboard(UiEvent::FocusNext);
        }
        let current = session.internal_ui.focused_field_lease(recipient).unwrap();
        assert_ne!(original.1, current.1);
        assert_eq!(original.0 == current.0, return_to_original);
        assert_eq!(session.on_screen_keyboard_snapshot().epoch, epoch);
        writer.write_all(b"stale").unwrap();
        drop(writer);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
                .unwrap();
        }
        assert!(session.native_clipboard.pending_read.is_none());
        assert_eq!(
            session.native_clipboard.last_failure.as_deref(),
            Some("clipboard paste field changed")
        );
        let fields = session
            .internal_ui
            .application::<Fields>(recipient)
            .unwrap();
        assert!(fields.first.is_empty() && fields.second.is_empty());
    }

    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let permit = session.native_clipboard.reads.acquire(1).unwrap();
    session
        .begin_native_image_read(reader.into(), recipient, permit)
        .unwrap();
    session.internal_ui.keyboard(UiEvent::FocusNext);
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    writer.write_all(&png).unwrap();
    drop(writer);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert!(session.native_clipboard.pending_read.is_none());
    assert_eq!(
        session.native_clipboard.last_failure.as_deref(),
        Some("clipboard paste field changed")
    );

    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let permit = session.native_clipboard.reads.acquire(1).unwrap();
    session
        .begin_native_direct_paste_read(reader.into(), key, recipient, 8, permit)
        .unwrap();
    writer.write_all(b"direct").unwrap();
    drop(writer);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert!(session.native_clipboard.last_failure.is_none());
    let fields = session
        .internal_ui
        .application::<Fields>(recipient)
        .unwrap();
    assert!(fields.first == "direct" || fields.second == "direct");
}

#[test]
fn native_keyboard_leases_follow_internal_recipients_without_seat_focus() {
    use nickel_session_protocol::OnScreenKeyboardInput;
    use nickel_ui::{UiEvent, id, ui};
    use std::io::Write;

    #[derive(Default)]
    struct TypingApp(String);
    impl nickel_ui::Application for TypingApp {
        type Message = String;
        fn update(&mut self, text: String) {
            self.0 = text;
        }
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<String> {
            ui! { <TextField id={id!(query)} value={&self.0} on_change={|text| text} /> }
        }
        fn title(&self) -> &str {
            "Keyboard recipient"
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let recipients = [
        crate::session::InternalSurfaceRole::Application,
        crate::session::InternalSurfaceRole::Overlay,
    ]
    .map(|role| {
        session.internal_ui.insert(
            TypingApp::default(),
            crate::session::InternalSurfacePlacement {
                role,
                geometry: (0, 0, 640, 480),
                output: None,
            },
            1.0,
        )
    });
    session.configure_on_screen_keyboard(true, true, 41, false, false, 368);
    assert!(session.focus_internal_surface(recipients[0]));
    session.internal_ui.keyboard(UiEvent::FocusNext);
    let first = session.on_screen_keyboard_snapshot();
    assert!(first.recipient.is_none());
    assert_eq!(
        first.internal_recipient,
        Some(recipients[0].snapshot_token())
    );
    assert_ne!(first.epoch, first.generation);
    session
        .deliver_on_screen_keyboard_input(
            first.epoch,
            OnScreenKeyboardInput::Text {
                text: "hello".into(),
            },
        )
        .unwrap();
    assert_eq!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[0])
            .unwrap()
            .0,
        "hello"
    );

    // Native clipboard ownership follows real UI copy/cut policy; admission
    // failure must preserve both the selected text and the previous owner.
    session.native_clipboard.text_limit = Some(8);
    let physical = session.seat.get_keyboard().unwrap().modifier_state();
    let chord = |keysym| OnScreenKeyboardInput::Key {
        keysym,
        modifiers: vec![0xffe3],
    };
    session
        .deliver_on_screen_keyboard_input(first.epoch, chord('a' as u32))
        .unwrap();
    session
        .deliver_on_screen_keyboard_input(first.epoch, chord('c' as u32))
        .unwrap();
    {
        let owner =
            smithay::wayland::selection::data_device::current_data_device_selection_userdata(
                &session.seat,
            )
            .unwrap();
        assert!(
            matches!(&*owner, crate::session::handlers::SelectionOwner::NativeText(text) if text.as_ref() == "hello")
        );
    }
    session.native_clipboard.text_limit = Some(4);
    assert!(
        session
            .deliver_on_screen_keyboard_input(first.epoch, chord('x' as u32))
            .is_err()
    );
    assert_eq!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[0])
            .unwrap()
            .0,
        "hello"
    );
    session.native_clipboard.text_limit = Some(8);
    session
        .deliver_on_screen_keyboard_input(first.epoch, chord('x' as u32))
        .unwrap();
    assert!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[0])
            .unwrap()
            .0
            .is_empty()
    );
    session
        .deliver_on_screen_keyboard_input(first.epoch, chord('v' as u32))
        .unwrap();
    assert_eq!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[0])
            .unwrap()
            .0,
        "hello"
    );
    assert_eq!(
        session.seat.get_keyboard().unwrap().modifier_state(),
        physical
    );

    let paste_key = crate::session::input::internal_virtual_key(
        'v' as u32,
        &[0xffe3],
        nickel_input::EventOrder(55),
    )
    .unwrap();
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let permit = session.native_clipboard.reads.acquire(1).unwrap();
    session
        .begin_native_paste_read(
            reader.into(),
            first.epoch,
            paste_key.clone(),
            recipients[0],
            8,
            permit,
        )
        .unwrap();
    writer.write_all(b"!").unwrap();
    drop(writer);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert!(session.native_clipboard.pending_read.is_none());
    assert!(
        session.native_clipboard.last_failure.is_none(),
        "Closed must not overwrite successful completion"
    );
    assert_eq!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[0])
            .unwrap()
            .0,
        "hello!"
    );
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let permit = session.native_clipboard.reads.acquire(1).unwrap();
    session
        .begin_native_paste_read(
            reader.into(),
            first.epoch,
            paste_key,
            recipients[0],
            8,
            permit,
        )
        .unwrap();

    // Both owners have a None Smithay target. Their distinct native leases
    // must still reject a release captured before the focus transfer.
    assert!(session.internal_ui.touch_with_client(
        0,
        (10.0, 10.0),
        crate::session::TouchPhase::Started,
        false,
    ));
    session.reconcile_internal_application_focus();
    assert_eq!(session.internal_ui.focused(), Some(recipients[1]));
    session.internal_ui.keyboard(UiEvent::FocusNext);
    assert!(
        session
            .deliver_on_screen_keyboard_input(
                first.epoch,
                OnScreenKeyboardInput::Text {
                    text: "stale".into()
                }
            )
            .is_err()
    );
    writer.write_all(b"stale").unwrap();
    drop(writer);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while session.native_clipboard.pending_read.is_some() && Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert!(session.native_clipboard.pending_read.is_none());
    assert_eq!(
        session.native_clipboard.last_failure.as_deref(),
        Some("clipboard paste recipient changed")
    );
    assert!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[1])
            .unwrap()
            .0
            .is_empty()
    );
    let second = session.on_screen_keyboard_snapshot();
    assert_ne!(first.epoch, second.epoch);
    session
        .deliver_on_screen_keyboard_input(
            second.epoch,
            OnScreenKeyboardInput::Text { text: "new".into() },
        )
        .unwrap();
    assert_eq!(
        session
            .internal_ui
            .application::<TypingApp>(recipients[1])
            .unwrap()
            .0,
        "new"
    );
    session.surrender_internal_focus();
    assert!(!session.on_screen_keyboard_snapshot().has_recipient());
    assert!(
        session
            .deliver_on_screen_keyboard_input(
                second.epoch,
                OnScreenKeyboardInput::Text {
                    text: "stale".into()
                }
            )
            .is_err()
    );
}

#[test]
fn internal_protection_hides_remote_inventory_without_hiding_local_window() {
    struct ProtectedApp(bool);
    impl nickel_ui::Application for ProtectedApp {
        type Message = ();
        fn update(&mut self, _: ()) {}
        fn remote_access_protected(&self) -> bool {
            self.0
        }
        fn view(&self, _: nickel_ui::ViewContext) -> impl nickel_ui::View<()> {
            nickel_ui::Text::new("fixture")
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.internal_ui.insert(
        ProtectedApp(false),
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );
    let window = session.register_internal_application(surface).unwrap();
    // The registration fallback is presentation metadata, not a verified app identity.
    assert_eq!(session.windows.app_id(window), Some("nickel-codex"));
    assert!(session.remote_verified_application(window).is_none());
    assert!(
        session
            .remote_protocol_windows()
            .iter()
            .any(|entry| entry.id.0 == window.0)
    );
    let observed_windows = session
        .protocol_windows()
        .into_iter()
        .map(|window| session.remote_window_summary(window))
        .collect::<Vec<_>>();
    let observed_apps = session.remote_internal_application_diagnostics(&observed_windows);
    session
        .inject_test_input(nickel_session_protocol::TestInput::PointerMove { x: 100, y: 120 })
        .unwrap();
    let input = session.remote_input_diagnostic(&observed_windows, &observed_apps, 1, 1);
    assert_eq!(
        input.keyboard.unwrap().focused_window,
        Some(window.0.to_string())
    );
    assert_eq!(
        input.pointer_hit_test.unwrap().window,
        Some(window.0.to_string())
    );
    assert!(
        session
            .remote_workspace_diagnostics(&observed_windows)
            .iter()
            .any(|workspace| workspace.windows.contains(&window.0.to_string()))
    );
    assert_eq!(
        session
            .remote_internal_renderer_diagnostics(&observed_apps, 1)
            .len(),
        1
    );
    session
        .internal_ui
        .application_mut::<ProtectedApp>(surface)
        .unwrap()
        .0 = true;
    assert!(session.remote_window_is_protected(window));
    // Previously projected identities cannot outlive a live protection change,
    // including before the next hosted frame reconciles presentation state.
    let input = session.remote_input_diagnostic(&observed_windows, &observed_apps, 2, 2);
    assert!(input.keyboard.is_none());
    assert!(input.pointer_hit_test.is_none());
    assert!(
        session
            .remote_workspace_diagnostics(&observed_windows)
            .iter()
            .all(
                |workspace| !workspace.windows.contains(&window.0.to_string())
                    && workspace.last_focused_window.as_deref()
                        != Some(window.0.to_string().as_str())
            )
    );
    assert!(
        session
            .remote_internal_renderer_diagnostics(&observed_apps, 2)
            .is_empty()
    );
    // Even a caller retaining the local inventory cannot project a protected host.
    let local_windows = session
        .protocol_windows()
        .into_iter()
        .map(|window| session.remote_window_summary(window))
        .collect::<Vec<_>>();
    assert!(
        session
            .remote_internal_application_diagnostics(&local_windows)
            .is_empty()
    );

    assert!(session.internal_ui.step(
        surface,
        nickel_ui::HostBatch {
            application_changed: true,
            ..nickel_ui::HostBatch::default()
        },
    ));
    assert!(
        !session
            .remote_protocol_windows()
            .iter()
            .any(|entry| entry.id.0 == window.0)
    );
    assert!(
        session
            .protocol_windows()
            .iter()
            .any(|entry| entry.id.0 == window.0)
    );
    session
        .internal_ui
        .application_mut::<ProtectedApp>(surface)
        .unwrap()
        .0 = false;
    assert!(session.remote_window_is_protected(window));
    let pending = session.remote_input_diagnostic(&observed_windows, &observed_apps, 3, 3);
    assert!(pending.keyboard.is_none());
    assert!(pending.pointer_hit_test.is_none());
    assert!(session.internal_ui.step(
        surface,
        nickel_ui::HostBatch {
            application_changed: true,
            ..nickel_ui::HostBatch::default()
        },
    ));
    assert!(!session.remote_window_is_protected(window));
    let restored = session.remote_input_diagnostic(&observed_windows, &observed_apps, 4, 4);
    assert_eq!(
        restored.keyboard.unwrap().focused_window,
        Some(window.0.to_string())
    );
    assert_eq!(
        restored.pointer_hit_test.unwrap().window,
        Some(window.0.to_string())
    );
    assert!(
        session
            .remote_protocol_windows()
            .iter()
            .any(|entry| entry.id.0 == window.0)
    );
}

#[test]
fn internal_diagnostics_follow_production_visibility_and_frame_lifecycle() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.internal_ui.insert(
        InternalHitTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (-80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.25,
    );
    let window = session.register_internal_application(surface).unwrap();
    let windows = session
        .remote_protocol_windows()
        .into_iter()
        .map(|window| session.remote_window_summary(window))
        .collect::<Vec<_>>();
    let records = session.remote_internal_application_diagnostics(&windows);
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, format!("internal:{}", surface.snapshot_token()));
    assert_eq!(record.generation, surface.snapshot_token());
    assert_eq!(record.window, window.0.to_string());
    assert_eq!(record.geometry, [-80, 90, 640, 480]);
    assert_eq!(record.output.as_deref(), Some("file-test"));
    assert_eq!(record.scale_factor, 1.25);
    assert!(record.keyboard_focused);
    let (_, semantic_nodes) = session
        .internal_ui
        .bounded_application_semantics(surface)
        .unwrap();
    let semantic_bounds = semantic_nodes.first().unwrap().bounds;
    session
        .inject_test_input(nickel_session_protocol::TestInput::PointerMove {
            x: -80 + (semantic_bounds.origin.x + semantic_bounds.size.width / 2.0).round() as i32,
            y: 90 + (semantic_bounds.origin.y + semantic_bounds.size.height / 2.0).round() as i32,
        })
        .unwrap();
    let input = session.remote_input_diagnostic(&windows, &records, 7, 11);
    assert_eq!(
        input.pointer_hit_test.as_ref().unwrap().window.as_deref(),
        Some(record.window.as_str())
    );
    assert!(
        input
            .pointer_hit_test
            .as_ref()
            .unwrap()
            .semantic_tree_generation
            .is_some(),
        "bounds={semantic_bounds:?} pointer={:?} hit={:?}",
        session.seat.get_pointer().unwrap().current_location(),
        input.pointer_hit_test
    );
    assert!(
        input
            .pointer_hit_test
            .as_ref()
            .unwrap()
            .semantic_node
            .is_some()
    );
    session
        .inject_test_input(nickel_session_protocol::TestInput::PointerMove { x: 0, y: 70 })
        .unwrap();
    let frame_hit = session.remote_input_diagnostic(&windows, &records, 8, 12);
    assert_eq!(
        frame_hit.pointer_hit_test.unwrap().decoration,
        Some(nickel_remote_control::diagnostics::InternalDecorationHit::Titlebar)
    );
    assert!(
        session
            .remote_input_diagnostic(&windows, &[], 9, 13)
            .pointer_hit_test
            .is_none(),
        "unprojected internal applications must remain unavailable"
    );
    assert_eq!(input.observation_generation, 7);
    assert_eq!(input.observed_at_us, 11);
    assert_eq!(
        input.keyboard.unwrap().focused_window.as_deref(),
        Some(record.window.as_str())
    );
    assert!(
        session
            .remote_input_diagnostic(&windows, &[], 8, 12)
            .keyboard
            .is_none()
    );

    assert!(record.redraw_pending);
    assert!(
        session
            .remote_internal_application_diagnostics(&[])
            .is_empty()
    );
    session.internal_ui.step(
        surface,
        nickel_ui::HostBatch {
            application_changed: true,
            ..nickel_ui::HostBatch::default()
        },
    );
    let next = session.remote_internal_application_diagnostics(&windows);
    assert!(next[0].resolved_frame_generation > record.resolved_frame_generation);
    assert_eq!(next[0].generation, record.generation);
    session.internal_ui.set_visible(surface, false);
    // Hidden ordinary applications remain inspectable under the same lease;
    // visibility and renderer suspension are part of the diagnostic state.
    let hidden = session.remote_internal_application_diagnostics(&windows);
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].id, record.id);
    assert_eq!(hidden[0].generation, record.generation);
    assert!(!hidden[0].visible);
    assert!(!hidden[0].keyboard_focused);
    let hidden_renderers = session.remote_internal_renderer_diagnostics(&hidden, 13);
    assert_eq!(hidden_renderers.len(), 1);
    assert_eq!(hidden_renderers[0].mode, "suspended");
    session.internal_ui.set_visible(surface, true);
    assert_eq!(
        session
            .remote_internal_application_diagnostics(&windows)
            .len(),
        1
    );
    session.internal_ui.remove(surface);
    assert!(
        session
            .remote_internal_application_diagnostics(&windows)
            .is_empty()
    );
}

#[test]
fn internal_application_has_canonical_window_lifecycle() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let surface = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );

    let window = session.register_internal_application(surface).unwrap();
    let snapshot = session
        .protocol_windows()
        .into_iter()
        .find(|candidate| candidate.id.0 == window.0)
        .unwrap();
    assert_eq!(snapshot.title, "Codex — Nickel");
    assert_eq!(snapshot.application_id, "nickel-codex");
    let geometry = snapshot.geometry.unwrap();
    assert_eq!((geometry.x, geometry.y), (80, 90));
    assert!(session.workspaces.is_visible(&window));

    session.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext);
    assert!(session.task_switcher.candidates().contains(&window));

    session.minimize_window(window);
    assert!(!session.internal_ui.is_visible(surface));
    assert!(
        session
            .protocol_windows()
            .iter()
            .any(|entry| entry.id.0 == window.0 && entry.minimized)
    );

    session.activate_window(window);
    assert!(session.internal_ui.is_visible(surface));
    assert_eq!(session.internal_ui.focused(), Some(surface));

    let restored = session.internal_ui.placement(surface).cloned().unwrap();
    session.maximize_window(window);
    assert!(session.internal_maximized_restore.contains_key(&window));
    assert_ne!(session.internal_ui.placement(surface), Some(&restored));
    session.maximize_window(window);
    assert!(!session.internal_maximized_restore.contains_key(&window));
    assert_eq!(session.internal_ui.placement(surface), Some(&restored));

    session.close_window(window);
    assert!(!session.windows.contains(window));
    assert!(session.internal_ui.placement(surface).is_none());
}

#[test]
#[ignore = "native XWayland acceptance: requires Xwayland and a writable XDG_RUNTIME_DIR; run alone"]
fn native_x11_launch_acknowledgement_rejects_forged_pid() {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass},
        wrapper::ConnectionExt as _,
    };
    struct FixtureEnvironment(Option<std::ffi::OsString>);
    impl Drop for FixtureEnvironment {
        fn drop(&mut self) {
            // SAFETY: this opt-in native test runs alone, like XWayland startup.
            unsafe {
                match self.0.take() {
                    Some(display) => std::env::set_var("DISPLAY", display),
                    None => std::env::remove_var("DISPLAY"),
                }
            }
        }
    }
    struct OwnedProcess(std::process::Child);
    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let _environment = FixtureEnvironment(std::env::var_os("DISPLAY"));
    let (mut event_loop, mut session) = internal_shell_test_session();
    session.start_xwayland();
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.xwm.is_none() {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < deadline, "owned XWayland did not start");
    }
    let child = OwnedProcess(
        std::process::Command::new("/usr/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let forged_pid = child.0.id();
    session
        .pending_launch_observations
        .push(PendingLaunchObservation {
            generation: 93,
            root_pid: forged_pid,
            root_start_time: super::linux_process_start_time(forged_pid).unwrap(),
            registered_at: Instant::now(),
            deadline: Duration::from_secs(15),
        });
    let display = format!(":{}", session.xwayland_display.unwrap());
    let (stop, stopped) = std::sync::mpsc::channel();
    let client = std::thread::spawn(move || {
        let (connection, screen) = smithay::reexports::x11rb::connect(Some(&display)).unwrap();
        let root = &connection.setup().roots[screen];
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                0,
                window,
                root.root,
                20,
                20,
                200,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().background_pixel(root.white_pixel),
            )
            .unwrap();
        let pid_atom = connection
            .intern_atom(false, b"_NET_WM_PID")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        connection
            .change_property32(
                PropMode::REPLACE,
                window,
                pid_atom,
                AtomEnum::CARDINAL,
                &[forged_pid],
            )
            .unwrap();
        connection
            .change_property8(
                PropMode::REPLACE,
                window,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                b"Forged launch acknowledgement fixture",
            )
            .unwrap();
        connection.map_window(window).unwrap();
        connection.flush().unwrap();
        let _ = stopped.recv_timeout(Duration::from_secs(20));
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let id = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        if let Some(id) = session.x11_windows.values().copied().find(|id| {
            session
                .remote_window_identities
                .get(id)
                .and_then(|identity| identity.current_process_id())
                == Some(std::process::id())
                && session.window_for_registry_id(*id).is_some()
        }) {
            break id;
        }
        assert!(
            Instant::now() < deadline,
            "mapped X11 owner was not verified through XRes"
        );
    };
    let window = session.window_for_registry_id(id).unwrap();
    assert_eq!(window.x11_surface().unwrap().pid(), Some(forged_pid));
    assert_eq!(
        session.pending_launch_observations.len(),
        1,
        "forged PID must not acknowledge the unrelated child"
    );
    // The same production observation accepts the actual connection owner.
    session.pending_launch_observations[0].root_pid = std::process::id();
    session.pending_launch_observations[0].root_start_time =
        super::linux_process_start_time(std::process::id()).unwrap();
    session.observe_pending_launch_window(id);
    assert!(session.pending_launch_observations.is_empty());
    let _ = stop.send(());
    client.join().unwrap();
}

#[test]
#[ignore = "native XWayland clipboard acceptance: requires Xwayland and a writable XDG_RUNTIME_DIR; run alone"]
fn native_xwayland_input_only_requestor_receives_complete_incremental_png() {
    use image::ImageEncoder;

    struct NativeDndSource(Arc<Vec<u8>>);

    impl smithay::utils::IsAlive for NativeDndSource {
        fn alive(&self) -> bool {
            true
        }
    }

    impl smithay::input::dnd::Source for NativeDndSource {
        fn metadata(&self) -> Option<smithay::input::dnd::SourceMetadata> {
            Some(smithay::input::dnd::SourceMetadata {
                mime_types: vec!["image/png".into()],
                dnd_actions: std::iter::once(smithay::input::dnd::DndAction::Copy).collect(),
            })
        }

        fn choose_action(&self, _action: smithay::input::dnd::DndAction) {}

        fn send(&self, mime_type: &str, fd: std::os::fd::OwnedFd) {
            if mime_type != "image/png" {
                return;
            }
            let payload = Arc::clone(&self.0);
            std::thread::spawn(move || {
                use std::io::Write as _;
                use std::os::fd::AsRawFd as _;
                // SAFETY: `fd` is owned by this worker and remains live for the call.
                assert!(
                    unsafe {
                        nix::libc::fcntl(fd.as_raw_fd(), nix::libc::F_SETFL, nix::libc::O_WRONLY)
                    } >= 0
                );
                let mut file = std::fs::File::from(fd);
                file.write_all(&payload).unwrap();
            });
        }

        fn drop_performed(&self) {}
        fn cancel(&self) {}
        fn finished(&self) {}
    }

    struct FixtureEnvironment(Option<std::ffi::OsString>);
    impl Drop for FixtureEnvironment {
        fn drop(&mut self) {
            // SAFETY: this opt-in native test runs alone, like XWayland startup.
            unsafe {
                match self.0.take() {
                    Some(display) => std::env::set_var("DISPLAY", display),
                    None => std::env::remove_var("DISPLAY"),
                }
            }
        }
    }

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let _environment = FixtureEnvironment(std::env::var_os("DISPLAY"));
    let (mut event_loop, mut session) = internal_shell_test_session();
    session.start_xwayland();
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.xwm.is_none() {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < deadline, "owned XWayland did not start");
    }

    let mut pixels = vec![0_u8; 512 * 512 * 4];
    let mut random = 0x4d59_5df4_d0f3_3173_u64;
    for byte in &mut pixels {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        *byte = random as u8;
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 512, 512, image::ExtendedColorType::Rgba8)
        .unwrap();
    assert!(png.len() > 64 * 1024, "fixture must exercise INCR");
    session
        .publish_native_image_clipboard(Arc::new(png.clone()))
        .unwrap();

    let display = format!(":{}", session.xwayland_display.unwrap());
    {
        use smithay::reexports::x11rb::{
            connection::Connection as _,
            protocol::xproto::{ConnectionExt as _, CreateWindowAux, EventMask, WindowClass},
        };
        let (conn, screen) = smithay::reexports::x11rb::connect(Some(&display)).unwrap();
        let conn = Arc::new(conn);
        let root = &conn.setup().roots[screen];
        let requestor = conn.generate_id().unwrap();
        conn.create_window(
            root.root_depth,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            root.root_visual,
            &CreateWindowAux::new().event_mask(EventMask::FOCUS_CHANGE),
        )
        .unwrap()
        .check()
        .unwrap();
        let observations = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let first =
            smithay::xwayland::xwm::RequestorObservation::acquire(&conn, &observations, requestor)
                .unwrap();
        let second =
            smithay::xwayland::xwm::RequestorObservation::acquire(&conn, &observations, requestor)
                .unwrap();
        drop(first);
        let retained = conn
            .get_window_attributes(requestor)
            .unwrap()
            .reply()
            .unwrap()
            .your_event_mask;
        assert!(retained.contains(EventMask::PROPERTY_CHANGE));
        assert!(retained.contains(EventMask::FOCUS_CHANGE));
        drop(second);
        let restored = conn
            .get_window_attributes(requestor)
            .unwrap()
            .reply()
            .unwrap()
            .your_event_mask;
        assert!(!restored.contains(EventMask::PROPERTY_CHANGE));
        assert!(restored.contains(EventMask::FOCUS_CHANGE));
    }
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let client_display = display.clone();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_x11_clipboard(
            &client_display,
            "NICKEL_TEST_SELECTION",
            Duration::ZERO,
            "image/png",
            true,
        );
        let _ = result_tx.send(result);
    });

    let received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match result_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("X11 requestor exited without a result")
            }
        }
        assert!(Instant::now() < deadline + Duration::from_secs(15));
    };
    assert_eq!(received, png);
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    for (index, size) in [0, 1, 65_535, 65_536, 65_537, 196_609]
        .into_iter()
        .enumerate()
    {
        let payload = (0..size)
            .map(|offset| ((offset * 31 + index * 17) & 0xff) as u8)
            .collect::<Vec<_>>();
        session
            .publish_native_image_clipboard(Arc::new(payload.clone()))
            .unwrap();
        let client_display = display.clone();
        let property = format!("NICKEL_TEST_SELECTION_{index}");
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let delay = if size >= 65_536 {
                Duration::from_millis(25)
            } else {
                Duration::default()
            };
            let _ = result_tx.send(
                super::internal_shell_placement_tests::receive_x11_clipboard(
                    &client_display,
                    &property,
                    delay,
                    "image/png",
                    true,
                ),
            );
        });
        let transfer_deadline = Instant::now() + Duration::from_secs(15);
        let received = loop {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            match result_rx.try_recv() {
                Ok(result) => break result.unwrap(),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    panic!("X11 boundary requestor exited without a result")
                }
            }
            assert!(Instant::now() < transfer_deadline);
        };
        assert_eq!(received, payload, "payload mismatch at {size} bytes");
        assert_eq!(
            session
                .xwm
                .as_ref()
                .unwrap()
                .1
                .outgoing_selection_transfer_count(),
            0
        );
    }

    let mut utf8_bytes = vec![b'a'; 65_538];
    utf8_bytes[65_534..].copy_from_slice("💚".as_bytes());
    let utf8_text = String::from_utf8(utf8_bytes.clone()).unwrap();
    session.publish_native_text_selection(utf8_text).unwrap();
    let text_display = display.clone();
    let (text_tx, text_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_x11_clipboard(
            &text_display,
            "NICKEL_TEXT_SELECTION",
            Duration::from_millis(25),
            "text/plain;charset=utf-8",
            false,
        );
        let _ = text_tx.send(result);
    });
    let text_deadline = Instant::now() + Duration::from_secs(15);
    let received_text = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match text_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("X11 InputOutput text requestor exited")
            }
        }
        assert!(Instant::now() < text_deadline);
    };
    assert_eq!(received_text, utf8_bytes);

    let concurrent_text = "two properties share one requestor ".repeat(4_097);
    let concurrent_text_bytes = concurrent_text.as_bytes().to_vec();
    session
        .publish_native_text_selection(concurrent_text)
        .unwrap();
    let concurrent_display = display.clone();
    let (concurrent_tx, concurrent_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_concurrent_x11_text_mimes(
            &concurrent_display,
        );
        let _ = concurrent_tx.send(result);
    });
    let concurrent_deadline = Instant::now() + Duration::from_secs(15);
    let concurrent_received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match concurrent_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("concurrent MIME requestor exited without a result")
            }
        }
        assert!(Instant::now() < concurrent_deadline);
    };
    assert!(
        concurrent_received
            .iter()
            .all(|payload| payload == &concurrent_text_bytes)
    );
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    let retired_payload = (0..196_609)
        .map(|offset| ((offset * 47 + 7) & 0xff) as u8)
        .collect::<Vec<_>>();
    let reused_payload = (0..196_609)
        .map(|offset| ((offset * 43 + 29) & 0xff) as u8)
        .collect::<Vec<_>>();
    session
        .publish_native_image_clipboard(Arc::new(retired_payload))
        .unwrap();
    let reused_display = display.clone();
    let (old_ready_tx, old_ready_rx) = std::sync::mpsc::channel();
    let (old_disconnected_tx, old_disconnected_rx) = std::sync::mpsc::channel();
    let (reconnect_tx, reconnect_rx) = std::sync::mpsc::channel();
    let (reused_tx, reused_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_x11_after_requestor_id_reuse(
            &reused_display,
            old_ready_tx,
            old_disconnected_tx,
            reconnect_rx,
        );
        let _ = reused_tx.send(result);
    });
    let reused_deadline = Instant::now() + Duration::from_secs(15);
    let old_requestor = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match old_ready_rx.try_recv() {
            Ok(requestor) => break requestor,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("old requestor exited before entering INCR")
            }
        }
        assert!(Instant::now() < reused_deadline);
    };
    old_disconnected_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    while session
        .xwm
        .as_ref()
        .unwrap()
        .1
        .outgoing_selection_transfer_count()
        != 0
    {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < reused_deadline);
    }
    session
        .publish_native_image_clipboard(Arc::new(reused_payload.clone()))
        .unwrap();
    reconnect_tx.send(()).unwrap();
    let (new_requestor, reused_received) = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match reused_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("reused requestor exited without a result")
            }
        }
        assert!(Instant::now() < reused_deadline);
    };
    assert_eq!(new_requestor, old_requestor);
    assert_eq!(reused_received, reused_payload);
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    let dnd_payload = (0..196_609)
        .map(|offset| ((offset * 53 + 31) & 0xff) as u8)
        .collect::<Vec<_>>();
    let dnd_display = display.clone();
    let (dnd_ready_tx, dnd_ready_rx) = std::sync::mpsc::channel();
    let (dnd_request_tx, dnd_request_rx) = std::sync::mpsc::channel();
    let (dnd_result_tx, dnd_result_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_x11_dnd_selection(
            &dnd_display,
            dnd_ready_tx,
            dnd_request_rx,
        );
        let _ = dnd_result_tx.send(result);
    });
    let dnd_deadline = Instant::now() + Duration::from_secs(15);
    let dnd_window = dnd_ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let dnd_surface = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        if let Some(surface) = session.space.elements().find_map(|window| {
            window
                .x11_surface()
                .filter(|surface| surface.window_id() == dnd_window)
                .cloned()
        }) {
            break surface;
        }
        assert!(Instant::now() < dnd_deadline);
    };
    let display_handle = session.display_handle.clone();
    let seat = session.seat.clone();
    let source = Arc::new(NativeDndSource(Arc::new(dnd_payload.clone())));
    let offer = smithay::input::dnd::DndFocus::enter(
        &dnd_surface,
        &mut session,
        &display_handle,
        source,
        &seat,
        (1.0, 1.0).into(),
        &smithay::utils::SERIAL_COUNTER.next_serial(),
    );
    assert!(
        offer.is_some(),
        "production Xdnd entry must create an offer"
    );
    dnd_request_tx.send(()).unwrap();
    let dnd_received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match dnd_result_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("X11 DnD requestor exited without a result")
            }
        }
        assert!(Instant::now() < dnd_deadline);
    };
    assert_eq!(dnd_received.len(), dnd_payload.len());
    assert_eq!(dnd_received, dnd_payload);
    drop(offer);
    while session.space.elements().any(|window| {
        window
            .x11_surface()
            .is_some_and(|surface| surface.window_id() == dnd_window)
    }) {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < dnd_deadline);
    }
    assert!(
        !session.xwm.as_ref().unwrap().1.has_active_dnd_offer(),
        "destroying the X11 target must retire its DnD offer"
    );
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    let primary_text_bytes = vec![b'p'; 65_537];
    let primary_text = String::from_utf8(primary_text_bytes.clone()).unwrap();
    smithay::wayland::selection::primary_selection::set_primary_selection(
        &session.display_handle,
        &session.seat,
        vec!["text/plain;charset=utf-8".into()],
        crate::session::handlers::SelectionOwner::NativeText(Arc::new(primary_text)),
    );
    session
        .xwm
        .as_mut()
        .unwrap()
        .1
        .new_selection(
            smithay::wayland::selection::SelectionTarget::Primary,
            Some(vec!["text/plain;charset=utf-8".into()]),
        )
        .unwrap();
    let outgoing_primary_display = display.clone();
    let (outgoing_primary_tx, outgoing_primary_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_x11_selection(
            &outgoing_primary_display,
            "PRIMARY",
            "NICKEL_OUTGOING_PRIMARY",
            Duration::from_millis(25),
            "text/plain;charset=utf-8",
            true,
        );
        let _ = outgoing_primary_tx.send(result);
    });
    let outgoing_primary_deadline = Instant::now() + Duration::from_secs(15);
    let outgoing_primary = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match outgoing_primary_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("outgoing primary requestor exited without a result")
            }
        }
        assert!(Instant::now() < outgoing_primary_deadline);
    };
    assert_eq!(outgoing_primary, primary_text_bytes);

    let simultaneous_payload = (0..196_609)
        .map(|offset| ((offset * 13 + 91) & 0xff) as u8)
        .collect::<Vec<_>>();
    session
        .publish_native_image_clipboard(Arc::new(simultaneous_payload.clone()))
        .unwrap();
    let (simultaneous_tx, simultaneous_rx) = std::sync::mpsc::channel();
    for index in 0..2 {
        let client_display = display.clone();
        let result_tx = simultaneous_tx.clone();
        std::thread::spawn(move || {
            let property = format!("NICKEL_SIMULTANEOUS_SELECTION_{index}");
            let result = super::internal_shell_placement_tests::receive_x11_clipboard(
                &client_display,
                &property,
                Duration::from_millis(50),
                "image/png",
                true,
            );
            let _ = result_tx.send(result);
        });
    }
    drop(simultaneous_tx);
    let simultaneous_deadline = Instant::now() + Duration::from_secs(15);
    let mut received = Vec::new();
    while received.len() != 2 {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        while let Ok(result) = simultaneous_rx.try_recv() {
            received.push(result.unwrap());
        }
        assert!(Instant::now() < simultaneous_deadline);
    }
    assert!(
        received
            .iter()
            .all(|payload| payload == &simultaneous_payload)
    );
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    let predecessor_payload = (0..196_609)
        .map(|offset| ((offset * 23 + 5) & 0xff) as u8)
        .collect::<Vec<_>>();
    let replacement_payload = (0..196_609)
        .map(|offset| ((offset * 29 + 113) & 0xff) as u8)
        .collect::<Vec<_>>();
    session
        .publish_native_image_clipboard(Arc::new(predecessor_payload))
        .unwrap();
    let replacement_display = display.clone();
    let (replacement_tx, replacement_rx) = std::sync::mpsc::channel();
    let (replace_ready_tx, replace_ready_rx) = std::sync::mpsc::channel();
    let (replace_go_tx, replace_go_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::receive_replaced_x11_clipboard(
            &replacement_display,
            "NICKEL_REPLACED_SELECTION",
            replace_ready_tx,
            replace_go_rx,
        );
        let _ = replacement_tx.send(result);
    });
    let replacement_deadline = Instant::now() + Duration::from_secs(15);
    while replace_ready_rx.try_recv().is_err() {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < replacement_deadline);
    }
    session
        .publish_native_image_clipboard(Arc::new(replacement_payload.clone()))
        .unwrap();
    replace_go_tx.send(()).unwrap();
    let replacement_received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match replacement_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("replacement requestor exited without a result")
            }
        }
        assert!(Instant::now() < replacement_deadline);
    };
    assert_eq!(replacement_received, replacement_payload);
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );

    let admission_payload = (0..196_609)
        .map(|offset| ((offset * 17 + 149) & 0xff) as u8)
        .collect::<Vec<_>>();
    session
        .publish_native_image_clipboard(Arc::new(admission_payload))
        .unwrap();
    let admission_display = display.clone();
    let (admission_tx, admission_rx) = std::sync::mpsc::channel();
    let (admission_stop_tx, admission_stop_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        super::internal_shell_placement_tests::exercise_x11_admission_limit(
            &admission_display,
            admission_tx,
            admission_stop_rx,
        );
    });
    let admission_deadline = Instant::now() + Duration::from_secs(20);
    let (accepted, rejected) = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match admission_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("admission requestors exited without a result")
            }
        }
        assert!(Instant::now() < admission_deadline);
    };
    assert_eq!((accepted, rejected), (32, 1));
    let retained = session
        .xwm
        .as_ref()
        .unwrap()
        .1
        .outgoing_selection_transfer_count();
    assert!((1..=32).contains(&retained));
    admission_stop_tx.send(()).unwrap();
    while session
        .xwm
        .as_ref()
        .unwrap()
        .1
        .outgoing_selection_transfer_count()
        != 0
    {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < admission_deadline);
    }

    session
        .publish_native_image_clipboard(Arc::new(simultaneous_payload))
        .unwrap();
    let stalled_display = display.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (aborted_tx, aborted_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::wait_for_x11_incremental_abort(
            &stalled_display,
            ready_tx,
        );
        let _ = aborted_tx.send(result);
    });
    let stalled_deadline = Instant::now() + Duration::from_secs(15);
    while ready_rx.try_recv().is_err() {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < stalled_deadline);
    }
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        1
    );
    let expired = session.xwm.as_mut().unwrap().1.expire_selection_transfers(
        Instant::now() + Duration::from_secs(6),
        &event_loop.handle(),
    );
    assert_eq!(expired, 1);
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        0
    );
    assert!(
        aborted_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_ok()
    );

    let reverse_payload = (0..196_609)
        .map(|offset| ((offset * 19 + 61) & 0xff) as u8)
        .collect::<Vec<_>>();
    let reverse_display = display.clone();
    let reverse_source = reverse_payload.clone();
    let (owner_ready_tx, owner_ready_rx) = std::sync::mpsc::channel();
    let (owner_done_tx, owner_done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::serve_x11_incremental_clipboard(
            &reverse_display,
            "CLIPBOARD",
            reverse_source,
            owner_ready_tx,
        );
        let _ = owner_done_tx.send(result);
    });
    owner_ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let reverse_deadline = Instant::now() + Duration::from_secs(15);
    while !session
        .native_clipboard
        .mime_types
        .iter()
        .any(|mime| mime == "image/png")
    {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(
            Instant::now() < reverse_deadline,
            "X11 clipboard owner was not discovered"
        );
    }
    let (reverse_reader, reverse_writer) = std::os::unix::net::UnixStream::pair().unwrap();
    session
        .xwm
        .as_mut()
        .unwrap()
        .1
        .send_selection(
            smithay::wayland::selection::SelectionTarget::Clipboard,
            "image/png".into(),
            reverse_writer.into(),
        )
        .unwrap();
    let (reverse_tx, reverse_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read as _;
        let mut reader = reverse_reader;
        let mut bytes = Vec::new();
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = reverse_tx.send(result);
    });
    let reverse_received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match reverse_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("native reverse clipboard recipient exited")
            }
        }
        assert!(
            Instant::now() < reverse_deadline,
            "reverse X11 clipboard transfer timed out"
        );
    };
    assert_eq!(reverse_received, reverse_payload);
    assert!(
        owner_done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_ok()
    );

    let primary_payload = (0..65_537)
        .map(|offset| ((offset * 37 + 71) & 0xff) as u8)
        .collect::<Vec<_>>();
    let primary_display = display.clone();
    let primary_source = primary_payload.clone();
    let (primary_ready_tx, primary_ready_rx) = std::sync::mpsc::channel();
    let (primary_done_tx, primary_done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::serve_x11_incremental_clipboard(
            &primary_display,
            "PRIMARY",
            primary_source,
            primary_ready_tx,
        );
        let _ = primary_done_tx.send(result);
    });
    primary_ready_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    for _ in 0..8 {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
    }
    let (primary_reader, primary_writer) = std::os::unix::net::UnixStream::pair().unwrap();
    session
        .xwm
        .as_mut()
        .unwrap()
        .1
        .send_selection(
            smithay::wayland::selection::SelectionTarget::Primary,
            "image/png".into(),
            primary_writer.into(),
        )
        .unwrap();
    let (primary_tx, primary_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read as _;
        let mut reader = primary_reader;
        let mut bytes = Vec::new();
        let result = reader.read_to_end(&mut bytes).map(|_| bytes);
        let _ = primary_tx.send(result);
    });
    let primary_deadline = Instant::now() + Duration::from_secs(15);
    let primary_received = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match primary_rx.try_recv() {
            Ok(result) => break result.unwrap(),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("native primary recipient exited without a result")
            }
        }
        assert!(Instant::now() < primary_deadline);
    };
    assert_eq!(primary_received, primary_payload);
    assert!(
        primary_done_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .is_ok()
    );

    let closed_recipient_payload = (0..196_609)
        .map(|offset| ((offset * 41 + 17) & 0xff) as u8)
        .collect::<Vec<_>>();
    let closed_display = display.clone();
    let (closed_ready_tx, closed_ready_rx) = std::sync::mpsc::channel();
    let (closed_done_tx, closed_done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::serve_x11_incremental_clipboard(
            &closed_display,
            "CLIPBOARD",
            closed_recipient_payload,
            closed_ready_tx,
        );
        let _ = closed_done_tx.send(result);
    });
    closed_ready_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    for _ in 0..8 {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
    }
    let (closed_reader, closed_writer) = std::os::unix::net::UnixStream::pair().unwrap();
    session
        .xwm
        .as_mut()
        .unwrap()
        .1
        .send_selection(
            smithay::wayland::selection::SelectionTarget::Clipboard,
            "image/png".into(),
            closed_writer.into(),
        )
        .unwrap();
    drop(closed_reader);
    let closed_deadline = Instant::now() + Duration::from_secs(15);
    let closed_result = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        match closed_done_rx.try_recv() {
            Ok(result) => break result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("closed-recipient owner exited without a result")
            }
        }
        assert!(Instant::now() < closed_deadline);
    };
    assert!(closed_result.is_ok());
    while session
        .xwm
        .as_ref()
        .unwrap()
        .1
        .incoming_selection_transfer_count()
        != 0
    {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < closed_deadline);
    }
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .incoming_selection_transfer_count(),
        0
    );

    for reject_transfer in [true, false] {
        let fixture_display = display.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (requested_tx, requested_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = super::internal_shell_placement_tests::serve_x11_pending_clipboard_fixture(
                &fixture_display,
                reject_transfer,
                ready_tx,
                requested_tx,
                stop_rx,
            );
            let _ = done_tx.send(result);
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..8 {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
        }
        let fixture_deadline = Instant::now() + Duration::from_secs(15);
        while !session
            .native_clipboard
            .mime_types
            .iter()
            .any(|mime| mime == "image/png")
        {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            assert!(Instant::now() < fixture_deadline);
        }
        let (_reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
        session
            .xwm
            .as_mut()
            .unwrap()
            .1
            .send_selection(
                smithay::wayland::selection::SelectionTarget::Clipboard,
                "image/png".into(),
                writer.into(),
            )
            .unwrap();
        while requested_rx.try_recv().is_err() {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
            assert!(Instant::now() < fixture_deadline);
        }
        if reject_transfer {
            while session
                .xwm
                .as_ref()
                .unwrap()
                .1
                .pending_selection_transfer_count()
                != 0
            {
                event_loop
                    .dispatch(Duration::from_millis(10), &mut session)
                    .unwrap();
                assert!(Instant::now() < fixture_deadline);
            }
        } else {
            assert_eq!(
                session
                    .xwm
                    .as_ref()
                    .unwrap()
                    .1
                    .pending_selection_transfer_count(),
                1
            );
            assert_eq!(
                session.xwm.as_mut().unwrap().1.expire_selection_transfers(
                    Instant::now() + Duration::from_secs(6),
                    &event_loop.handle(),
                ),
                1
            );
            assert_eq!(
                session
                    .xwm
                    .as_ref()
                    .unwrap()
                    .1
                    .pending_selection_transfer_count(),
                0
            );
            let _ = stop_tx.send(());
        }
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .is_ok()
        );
        // Retire the fixture owner's XFixes notification before installing
        // the next owner, so a queued owner-loss event cannot revoke it.
        for _ in 0..8 {
            event_loop
                .dispatch(Duration::from_millis(10), &mut session)
                .unwrap();
        }
    }

    let restart_payload = (0..196_609)
        .map(|offset| ((offset * 7 + 43) & 0xff) as u8)
        .collect::<Vec<_>>();
    session
        .publish_native_image_clipboard(Arc::new(restart_payload))
        .unwrap();
    let restart_display = display;
    let (restart_ready_tx, restart_ready_rx) = std::sync::mpsc::channel();
    let (restart_done_tx, restart_done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = super::internal_shell_placement_tests::wait_for_x11_incremental_abort(
            &restart_display,
            restart_ready_tx,
        );
        let _ = restart_done_tx.send(result);
    });
    let restart_deadline = Instant::now() + Duration::from_secs(15);
    while restart_ready_rx.try_recv().is_err() {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        assert!(Instant::now() < restart_deadline);
    }
    assert_eq!(
        session
            .xwm
            .as_ref()
            .unwrap()
            .1
            .outgoing_selection_transfer_count(),
        1
    );
    session
        .xwm
        .as_mut()
        .unwrap()
        .1
        .shutdown(&event_loop.handle());
    session.xwm.take();
    let registration = session
        .xwayland_registration
        .take()
        .expect("owned XWayland registration");
    event_loop.handle().remove(registration);
    assert!(
        restart_done_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
        "X11 requestor must terminate when XWayland restarts"
    );
}

#[test]
#[ignore = "native Wayland acceptance: requires /usr/bin/zenity and a writable XDG_RUNTIME_DIR"]
fn native_launch_acknowledgement_waits_for_mapped_verified_window() {
    use crate::session::remote_identity::{ProcessIdentity, WindowIdentity};
    use crate::session::window_registry::WindowId;
    struct OwnedDialog(std::process::Child);
    impl Drop for OwnedDialog {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    // Deliver the identity result explicitly to exercise both orderings.
    session.remote_identity_worker = None;
    let config = tempfile::tempdir().unwrap();
    let child = OwnedDialog(
        std::process::Command::new("/usr/bin/zenity")
            .args([
                "--info",
                "--title=Launch acknowledgement native fixture",
                "--text=Native launch fixture",
            ])
            .env("WAYLAND_DISPLAY", &session.socket_name)
            .env("GDK_BACKEND", "wayland")
            .env("GSK_RENDERER", "cairo")
            .env("XDG_CONFIG_HOME", config.path())
            .env_remove("DISPLAY")
            .env_remove("NICKEL_SESSION_TOKEN")
            .env_remove("NICKEL_SESSION_CONTROL")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let id = loop {
        event_loop
            .dispatch(Duration::from_millis(10), &mut session)
            .unwrap();
        if let Some(id) = session
            .protocol_windows()
            .iter()
            .find(|window| window.title == "Launch acknowledgement native fixture")
            .map(|window| WindowId(window.id.0))
            .filter(|id| session.window_for_registry_id(*id).is_some())
        {
            break id;
        }
        assert!(Instant::now() < deadline, "native dialog did not map");
    };
    session
        .pending_launch_observations
        .push(PendingLaunchObservation {
            generation: 91,
            root_pid: child.0.id(),
            root_start_time: super::linux_process_start_time(child.0.id()).unwrap(),
            registered_at: Instant::now(),
            deadline: Duration::from_secs(2),
        });
    let subscriber_path = config.path().join("launch-subscriber.sock");
    let subscriber = std::os::unix::net::UnixDatagram::bind(&subscriber_path).unwrap();
    subscriber.set_nonblocking(true).unwrap();
    session.launcher_subscribers.push(subscriber_path);
    let no_acknowledgement = || {
        let mut bytes = [0; 2048];
        assert_eq!(
            subscriber.recv(&mut bytes).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "ineligible or duplicate observation must not emit launch feedback"
        );
    };
    session.observe_pending_launch_window(id);
    no_acknowledgement();
    assert_eq!(
        session.pending_launch_observations.len(),
        1,
        "mapped window without verified identity must wait"
    );
    let window = session.window_for_registry_id(id).unwrap();
    let location = session.space.element_location(&window).unwrap();
    session.space.unmap_elem(&window);
    session.remote_window_identities.insert(
        id,
        WindowIdentity::Verified(ProcessIdentity::inspect(child.0.id()).unwrap()),
    );
    session.observe_pending_launch_window(id);
    assert_eq!(
        session.pending_launch_observations.len(),
        1,
        "verified but unmapped window must wait"
    );
    no_acknowledgement();
    session.space.map_element(window, location, false);
    session.remote_window_identities.insert(
        id,
        WindowIdentity::Verified(ProcessIdentity::inspect(std::process::id()).unwrap()),
    );
    session.observe_pending_launch_window(id);
    assert_eq!(
        session.pending_launch_observations.len(),
        1,
        "an unrelated verified owner cannot satisfy the launched child's lineage"
    );
    no_acknowledgement();
    session.remote_window_identities.insert(
        id,
        WindowIdentity::Verified(ProcessIdentity::inspect(child.0.id()).unwrap()),
    );
    session.observe_pending_launch_window(id);
    assert!(
        session.pending_launch_observations.is_empty(),
        "mapped verified process should acknowledge once"
    );
    let mut bytes = [0; 2048];
    let size = subscriber.recv(&mut bytes).unwrap();
    let response =
        nickel_session_protocol::decode::<nickel_session_protocol::ServerEnvelope>(&bytes[..size])
            .unwrap();
    assert_eq!(response.request_id, 0);
    assert!(matches!(
        response.message,
        nickel_session_protocol::ServerMessage::Event(
            nickel_session_protocol::Event::PendingLaunchWindow {
                generation: 91,
                descendant: false,
                observed_after_ms: 0..=2000,
            }
        )
    ));
    session.observe_pending_launch_window(id);
    assert!(session.pending_launch_observations.is_empty());
    no_acknowledgement();

    let slow_path = config.path().join("stalled-launch-subscriber.sock");
    let slow = std::os::unix::net::UnixDatagram::bind(&slow_path).unwrap();
    slow.set_nonblocking(true).unwrap();
    let mut fillers = vec![super::notification_socket().unwrap()];
    let mut filled = false;
    for _ in 0..10_000 {
        match fillers.last().unwrap().send_to(b"occupied", &slow_path) {
            Ok(_) => (),
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                let fresh = super::notification_socket().unwrap();
                match fresh.send_to(b"occupied", &slow_path) {
                    Ok(_) => fillers.push(fresh),
                    Err(error) => {
                        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
                        filled = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(filled, "fixture must actually saturate the receiver queue");
    session.launcher_subscribers.insert(0, slow_path.clone());
    // Bound failure time if blocking sends regress: release queue capacity
    // after one second, then fail the latency assertion rather than hanging.
    let recovery = slow.try_clone().unwrap();
    let (cancel, wait) = std::sync::mpsc::channel();
    let recovery_thread = std::thread::spawn(move || {
        if wait.recv_timeout(Duration::from_secs(1)).is_err() {
            let mut bytes = [0; 2048];
            while recovery.recv(&mut bytes).is_ok() {}
        }
    });
    let started = Instant::now();
    session.notify_pending_launch_expired(92);
    let elapsed = started.elapsed();
    let _ = cancel.send(());
    recovery_thread.join().unwrap();
    assert!(
        elapsed < Duration::from_millis(750),
        "stalled subscriber delayed owner by {elapsed:?}"
    );
    assert!(!session.launcher_subscribers.contains(&slow_path));
    let size = subscriber.recv(&mut bytes).unwrap();
    let delivered =
        nickel_session_protocol::decode::<nickel_session_protocol::ServerEnvelope>(&bytes[..size])
            .unwrap();
    assert!(
        matches!(
            delivered.message,
            nickel_session_protocol::ServerMessage::Event(
                nickel_session_protocol::Event::PendingLaunchExpired { generation: 92 }
            )
        ),
        "healthy subscriber must still receive the event"
    );
}

#[test]
fn late_window_callback_leaves_expiry_owned_by_the_registered_timer() {
    let now = Instant::now();
    let pending = PendingLaunchObservation {
        generation: 7,
        root_pid: std::process::id(),
        root_start_time: 1,
        registered_at: now - Duration::from_millis(101),
        deadline: Duration::from_millis(100),
    };

    assert_eq!(
        pending_launch_window_disposition(&pending, now, std::process::id()),
        PendingLaunchWindowDisposition::AwaitExpiry
    );
}

#[test]
fn shell_behavior_values_are_typed_and_desktop_counts_are_validated() {
    let mut settings = nickel_core::shell_settings::ShellSettings::default();
    assert_eq!(
        shell_behavior_value(&settings, ShellBehaviorSetting::BarDisplayScope),
        ShellBehaviorValue::Toggle(true)
    );
    assert!(
        apply_shell_behavior_value(
            &mut settings,
            ShellBehaviorSetting::BarWindowScope,
            ShellBehaviorValue::Toggle(false),
        )
        .is_ok()
    );
    assert!(!settings.all_windows_on_every_bar);
    assert!(
        apply_shell_behavior_value(
            &mut settings,
            ShellBehaviorSetting::DesktopCount,
            ShellBehaviorValue::Count(0),
        )
        .is_err()
    );
    assert!(
        apply_shell_behavior_value(
            &mut settings,
            ShellBehaviorSetting::BarDisplayScope,
            ShellBehaviorValue::Count(2),
        )
        .is_err()
    );
}

#[test]
fn shell_behavior_transactions_reject_stale_topology_and_concurrent_writers() {
    let current = nickel_core::shell_settings::ShellSettings::default();
    let mut transaction = ShellBehaviorTransaction {
        setting: ShellBehaviorSetting::BarDisplayScope,
        prior: ShellBehaviorValue::Toggle(true),
        requested: ShellBehaviorValue::Toggle(false),
        topology_generation: 4,
    };
    assert_eq!(
        prepare_shell_behavior_update(&current, 5, &transaction),
        Err("stale output topology generation")
    );
    transaction.topology_generation = 5;
    transaction.prior = ShellBehaviorValue::Toggle(false);
    assert_eq!(
        prepare_shell_behavior_update(&current, 5, &transaction),
        Err("shell setting changed before this transaction was applied")
    );
    transaction.prior = ShellBehaviorValue::Toggle(true);
    let requested = prepare_shell_behavior_update(&current, 5, &transaction).unwrap();
    assert!(!requested.bar_on_all_displays);
    assert!(
        current.bar_on_all_displays,
        "planning must not mutate authority"
    );
}

#[test]
fn pointer_identity_churn_returns_all_collections_to_baseline() {
    let mut hints = HashMap::new();
    let mut locks = HashSet::new();
    let mut origins = HashMap::new();

    for surface in 0_u16..300 {
        hints.insert(surface, Point::from((1.0, 2.0)));
        locks.insert(surface);
        origins.insert(surface, Point::from((3.0, 4.0)));
        let restored = retire_pointer_surface(&mut hints, &mut locks, &mut origins, &surface);
        assert_eq!(restored, Some(Point::from((4.0, 6.0))));
        assert!(hints.is_empty());
        assert!(locks.is_empty());
        assert!(origins.is_empty());
    }

    assert_eq!(hints.capacity(), 0);
    assert_eq!(locks.capacity(), 0);
    assert_eq!(origins.capacity(), 0);
    hints.insert(301, Point::from((5.0, 6.0)));
    assert_eq!(hints.len(), 1, "new hints remain admissible after churn");
}

#[test]
fn closed_displaced_windows_do_not_preserve_output_history() {
    let mut outputs = HashMap::new();
    for index in 0_u64..300 {
        let id = super::WindowId(index + 1);
        outputs.insert(
            format!("virtual-{index}"),
            vec![DisplacedWindow {
                id,
                relative_location: Point::from((10, 20)),
                rescue_location: Point::from((30, 40)),
                rescue_revision: nickel_core::geometry_authority::GeometryRevision::INITIAL,
            }],
        );
        retire_displaced_window(&mut outputs, id);
        assert!(outputs.is_empty());
    }
    assert_eq!(outputs.capacity(), 0);
}

#[test]
fn displaced_mapped_minimized_and_hidden_windows_retire_independently() {
    let mapped = super::WindowId(1);
    let minimized = super::WindowId(2);
    let hidden = super::WindowId(3);
    let displaced = |id| DisplacedWindow {
        id,
        relative_location: Point::from((10, 20)),
        rescue_location: Point::from((30, 40)),
        rescue_revision: nickel_core::geometry_authority::GeometryRevision::INITIAL,
    };
    let mut outputs = HashMap::from([(
        "removed-output".to_owned(),
        vec![displaced(mapped), displaced(minimized), displaced(hidden)],
    )]);

    retire_displaced_window(&mut outputs, mapped);
    assert_eq!(outputs["removed-output"].len(), 2);
    retire_displaced_window(&mut outputs, minimized);
    assert_eq!(outputs["removed-output"].len(), 1);
    retire_displaced_window(&mut outputs, hidden);
    assert!(outputs.is_empty());
    assert_eq!(outputs.capacity(), 0);
}

#[test]
fn destroying_a_shell_surface_retires_only_its_registration() {
    let retired = ObjectId::null();
    let retained = retired.clone();
    let mut registrations = vec![RegisteredShellRole {
        role: ShellRole::Launcher,
        output: None,
        surface: retired.clone(),
    }];
    retire_shell_surface(&mut registrations, &retired);
    assert!(registrations.is_empty());
    assert_eq!(registrations.capacity(), 0);

    // Re-registration after independent destruction is not blocked by a
    // historical singleton slot.
    registrations.push(RegisteredShellRole {
        role: ShellRole::Launcher,
        output: None,
        surface: retained,
    });
    assert_eq!(registrations.len(), 1);
}

#[test]
fn live_surface_role_transitions_invalidate_historical_readiness() {
    let surface = ObjectId::null();
    let registrations = vec![RegisteredShellRole {
        role: ShellRole::Launcher,
        output: None,
        surface: surface.clone(),
    }];

    assert!(!shell_registration_role_changed(
        &registrations,
        &surface,
        Some(ShellRole::Launcher)
    ));
    assert!(shell_registration_role_changed(
        &registrations,
        &surface,
        Some(ShellRole::ControlCenter)
    ));
    assert!(shell_registration_role_changed(
        &registrations,
        &surface,
        None
    ));
}

#[test]
fn disconnected_output_roles_are_dormant_until_the_output_returns() {
    let registrations = [
        RegisteredShellRole {
            role: ShellRole::Desktop,
            output: Some("winit".into()),
            surface: ObjectId::null(),
        },
        RegisteredShellRole {
            role: ShellRole::Desktop,
            output: Some("DP-test".into()),
            surface: ObjectId::null(),
        },
        RegisteredShellRole {
            role: ShellRole::Panel,
            output: Some("DP-test".into()),
            surface: ObjectId::null(),
        },
        RegisteredShellRole {
            role: ShellRole::Lock,
            output: Some("DP-test".into()),
            surface: ObjectId::null(),
        },
    ];
    let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
    assert!(registrations.iter().all(|registration| {
        shell_registration_is_active(registration, &connected, &connected)
    }));

    let after_disconnect = HashSet::from(["winit".to_owned()]);
    assert!(shell_registration_is_active(
        &registrations[0],
        &after_disconnect,
        &after_disconnect,
    ));
    assert!(registrations[1..].iter().all(|registration| {
        !shell_registration_is_active(registration, &after_disconnect, &after_disconnect)
    }));

    // Keeping the slots dormant preserves the shell's bounded reconnect
    // grace: the same native surfaces become authoritative again if the
    // named output returns instead of requiring an app-id transition.
    assert!(registrations.iter().all(|registration| {
        shell_registration_is_active(registration, &connected, &connected)
    }));
}

#[test]
fn panel_registration_tracks_the_configured_output_set() {
    let registration = RegisteredShellRole {
        role: ShellRole::Panel,
        output: Some("DP-test".into()),
        surface: ObjectId::null(),
    };
    let connected = HashSet::from(["winit".to_owned(), "DP-test".to_owned()]);
    let primary_only = HashSet::from(["winit".to_owned()]);
    assert!(!shell_registration_is_active(
        &registration,
        &connected,
        &primary_only,
    ));
    assert!(shell_registration_is_active(
        &registration,
        &connected,
        &connected,
    ));
}

#[test]
fn locked_test_output_disconnect_projects_readiness_to_live_topology() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "DP-test".into(),
            logical_width: 640,
            logical_height: 480,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    for role in [ShellRole::Desktop, ShellRole::Panel, ShellRole::Lock] {
        session
            .registered_shell_role_slots
            .push(RegisteredShellRole {
                role,
                output: Some("DP-test".into()),
                surface: ObjectId::null(),
            });
    }
    let connected = session.protocol_shell_readiness();
    assert_eq!((connected.outputs, connected.desktops), (1, 1));
    assert_eq!((connected.panels, connected.locks), (1, 1));

    session.locked = true;
    session
        .apply_test_output(TestOutput::Disconnect {
            name: "DP-test".into(),
        })
        .unwrap();

    let disconnected = session.protocol_shell_readiness();
    assert_eq!((disconnected.outputs, disconnected.desktops), (0, 0));
    assert_eq!((disconnected.panels, disconnected.locks), (0, 0));
    assert!(session.registered_shell_role_slots.len() == 3);
}

#[test]
fn touch_output_hint_uses_named_logical_geometry_and_never_falls_back_after_removal() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    for (name, scale_120) in [("fallback", 120), ("touchscreen", 180)] {
        session
            .apply_test_output(TestOutput::Connect {
                name: name.into(),
                logical_width: 800,
                logical_height: 600,
                scale_120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
    }
    let output = session
        .space
        .outputs()
        .find(|output| output.name() == "touchscreen")
        .unwrap()
        .clone();
    session.space.map_output(&output, (-800, -120));
    let geometry = session.touch_output_geometry(Some("touchscreen")).unwrap();
    assert_eq!(geometry.loc, (-800, -120).into());
    assert_eq!(geometry.size, (800, 600).into());
    assert_ne!(session.touch_output_geometry(None).unwrap(), geometry);
    session
        .apply_test_output(TestOutput::Disconnect {
            name: "touchscreen".into(),
        })
        .unwrap();
    assert!(session.touch_output_geometry(Some("touchscreen")).is_none());
    assert!(session.touch_output_geometry(None).is_some());
}

#[test]
fn removed_touch_output_cancels_native_target_and_drops_later_tail() {
    use nickel_session_protocol::TestInput;

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "fallback".into(),
            logical_width: 800,
            logical_height: 600,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    session
        .apply_test_output(TestOutput::Connect {
            name: "touchscreen".into(),
            logical_width: 800,
            logical_height: 600,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    let contact = Some(9).into();
    let native_slot =
        session
            .client_touch_slots
            .begin("nickel-synthetic-input", contact, Some("touchscreen"));
    session.active_touch_slots.insert(native_slot);

    session
        .apply_test_output(TestOutput::Disconnect {
            name: "touchscreen".into(),
        })
        .unwrap();

    assert!(session.active_touch_slots.is_empty());
    assert_eq!(
        session
            .client_touch_slots
            .get("nickel-synthetic-input", contact),
        None
    );
    assert!(!session.client_touch_slots.owns_output("touchscreen"));

    // These go through the production session input reducer. With no retained native target
    // binding they stop before delivery (whose first observable side effect is recording the
    // interaction output) and cannot recreate the retired contact.
    session.last_interaction_output_name = None;
    session
        .inject_test_input(TestInput::TouchMotion {
            slot: 9,
            x: 10,
            y: 10,
        })
        .unwrap();
    session
        .inject_test_input(TestInput::TouchUp { slot: 9 })
        .unwrap();
    assert!(session.last_interaction_output_name.is_none());
    assert!(session.active_touch_slots.is_empty());
    assert_eq!(
        session
            .client_touch_slots
            .get("nickel-synthetic-input", contact),
        None
    );
}

#[test]
fn keyboard_reservation_resize_and_close_change_only_the_owner_output() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    for name in ["keyboard-owner", "unaffected"] {
        session
            .apply_test_output(TestOutput::Connect {
                name: name.into(),
                logical_width: 1280,
                logical_height: 720,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
    }
    session.configure_on_screen_keyboard(true, true, 0, false, false, 368);
    let outputs = session.protocol_outputs();
    let owner = session.on_screen_keyboard.output_name.clone().unwrap();
    for output in &outputs {
        assert_eq!(
            output.work_area.height,
            if output.name == owner { 352 } else { 664 }
        );
    }
    session.configure_on_screen_keyboard(true, true, 0, false, true, 280);
    assert_eq!(
        session.on_screen_keyboard.output_name.as_deref(),
        Some(owner.as_str())
    );
    for output in session.protocol_outputs() {
        assert_eq!(
            output.work_area.height,
            if output.name == owner { 440 } else { 664 }
        );
        assert_eq!(
            output.work_area.y,
            if output.name == owner { 280 } else { 0 }
        );
    }
    session.configure_on_screen_keyboard(true, false, 0, false, true, 280);
    assert!(
        session
            .protocol_outputs()
            .iter()
            .all(|output| output.work_area.height == 664)
    );
}

#[test]
fn output_identification_local_replacement_survives_stale_expiry_and_exhaustion() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let first = session.start_output_identification(None).unwrap();
    let first_timer = session.identify_outputs_timer.unwrap();
    session.begin_output_identification();
    assert_ne!(session.identify_outputs_timer, Some(first_timer));
    let replacement = session.identify_outputs_generation;
    assert!(replacement > first);
    assert!(!session.expire_output_identification(first));
    assert!(session.identify_outputs_until.is_some());
    #[cfg(any(feature = "backend-udev", feature = "backend-winit"))]
    {
        let output = session.space.outputs().next().unwrap().clone();
        assert_eq!(
            session.output_identification_index(&output),
            Some((replacement, 0))
        );
        session.locked = true;
        assert_eq!(session.output_identification_index(&output), None);
        session.locked = false;
    }
    session.identify_outputs_generation = u64::MAX;
    let until = session.identify_outputs_until;
    let timer = session.identify_outputs_timer;
    assert!(session.start_output_identification(None).is_err());
    assert_eq!(session.identify_outputs_timer, timer);
    assert_eq!(session.identify_outputs_generation, u64::MAX);
    assert_eq!(session.identify_outputs_until, until);
    assert!(session.expire_output_identification(u64::MAX));
    assert!(session.identify_outputs_until.is_none());
    assert!(session.identify_outputs_timer.is_none());
}

#[test]
fn only_the_latest_output_identification_generation_may_expire() {
    assert!(identification_expiry_is_current(7, 7));
    assert!(!identification_expiry_is_current(8, 7));
    assert!(!identification_expiry_is_current(7, 8));
}

#[test]
fn connection_cleanup_precedes_first_request_from_full_ordinary_queue() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let wake = session.remote_cleanup_wake.clone();
    let control = session.remote_control.control();
    let lease = {
        let mut authority = control.lock().unwrap();
        authority.set_enabled(true);
        let client = authority.connect_identity("Cleanup priority test").unwrap();
        let then = Instant::now() - Duration::from_secs(61);
        let watch = authority
            .reserve_connection_watch(&client.client_id, &client.token, then)
            .unwrap();
        authority
            .activate_connection_watch(&client.client_id, &client.token, watch, false, then)
            .unwrap();
        authority
            .leases_mut()
            .approve_local(
                client.client_id.clone(),
                nickel_remote_control::leases::ResourceScope::FullSession,
                then,
                None,
                false,
                false,
            )
            .unwrap()
    };
    let (sender, receiver) = smithay::reexports::calloop::channel::sync_channel(32);
    for sequence in 0..32 {
        assert!(
            sender
                .try_send(super::RemoteDesktopRequest::NativeKeyboardState {
                    source: smithay::input::keyboard::KeyboardSource::new_focus_bound_auxiliary(),
                    sequence,
                    result: Err("stale native query".into()),
                })
                .is_ok()
        );
    }
    wake.notify();
    // Same production handler used by the registered ordinary channel. Do
    // not dispatch the separate wake source: priority must also hold here.
    session.handle_remote_desktop_request(receiver.try_recv().unwrap());
    assert!(!wake.take_pending());
    assert!(
        control
            .lock()
            .unwrap()
            .leases()
            .iter()
            .all(|entry| entry.id != lease)
    );
}

#[test]
fn connection_cleanup_setup_failure_preserves_owner_fallback() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (event_loop, mut session) = internal_shell_test_session();
    let wake = super::NickelSession::register_remote_connection_cleanup_wake(
        &event_loop.handle(),
        Err(std::io::Error::other("descriptor unavailable")),
    );
    session.remote_cleanup_wake = wake.clone();
    wake.notify();
    assert!(wake.take_wake_failure());
    // Exercise the same owner drain used by the retained periodic timer.
    session.service_remote_connection_cleanup();
    assert!(!wake.take_pending());
    wake.notify();
    assert!(wake.take_wake_failure());
    session.service_remote_connection_cleanup();
    assert!(!wake.take_pending());
}

#[test]
fn connection_cleanup_eventfd_wakes_idle_production_owner() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    let wake = session.remote_cleanup_wake.clone();
    wake.notify();
    event_loop
        .dispatch(Duration::from_millis(50), &mut session)
        .unwrap();
    assert!(!wake.take_pending());
}

static PREVIEW_SESSION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn preview_test_session() -> (
    EventLoop<'static, super::NickelSession>,
    super::NickelSession,
) {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let session = super::NickelSession::new(&mut event_loop, display, true);
    (event_loop, session)
}

#[test]
fn launcher_focus_deadline_runs_on_session_executor_and_withdraws_without_restore() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    session.launcher_visibility.set(true);
    session.launcher_restore_window = Some(super::WindowId(41));
    let now = session.start_time.elapsed();
    let request = session.launcher_focus.request_at(
        ObjectId::null(),
        nickel_core::focus::FocusTargetLifetime::EmbeddedInTarget,
        None,
        nickel_core::focus::FocusScope::Launcher,
        nickel_core::focus::FocusSecurityEpoch(0),
        now,
        Duration::from_millis(1),
    );
    let deadline = session.launcher_focus.record(&request).unwrap().deadline;
    session.schedule_launcher_focus_deadline(request.transaction, deadline);

    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();

    assert_eq!(
        session.launcher_focus.phase(&request),
        Some(nickel_core::focus::FocusRequestPhase::TimedOut)
    );
    assert!(!session.launcher_visibility.is_visible());
    assert_eq!(session.launcher_restore_window, None);
}

#[test]
fn stale_session_deadline_cannot_withdraw_newer_launcher_focus_intent() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    session.launcher_visibility.set(true);
    let now = session.start_time.elapsed();
    let old = session.launcher_focus.request_at(
        ObjectId::null(),
        nickel_core::focus::FocusTargetLifetime::EmbeddedInTarget,
        None,
        nickel_core::focus::FocusScope::Launcher,
        nickel_core::focus::FocusSecurityEpoch(0),
        now,
        Duration::from_millis(1),
    );
    let old_deadline = session.launcher_focus.record(&old).unwrap().deadline;
    session.schedule_launcher_focus_deadline(old.transaction, old_deadline);
    let current = session.launcher_focus.request_at(
        ObjectId::null(),
        nickel_core::focus::FocusTargetLifetime::EmbeddedInTarget,
        None,
        nickel_core::focus::FocusScope::Launcher,
        nickel_core::focus::FocusSecurityEpoch(0),
        now,
        Duration::from_secs(1),
    );

    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();

    assert!(session.launcher_visibility.is_visible());
    assert_eq!(
        session.launcher_focus.phase(&old),
        Some(nickel_core::focus::FocusRequestPhase::Superseded)
    );
    assert_eq!(
        session.launcher_focus.phase(&current),
        Some(nickel_core::focus::FocusRequestPhase::Pending)
    );
}

struct IdleInternalHost;

#[test]
fn native_media_notification_bypasses_legacy_subscribers_and_preserves_focus() {
    use nickel_session_protocol::ConsumerControl;
    struct RecordingMediaHost(std::sync::Mutex<Vec<ConsumerControl>>);
    impl SessionHost for RecordingMediaHost {
        fn dispatch(&self, _: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }
        fn consumer_control(&self, control: ConsumerControl) -> bool {
            self.0.lock().unwrap().push(control);
            true
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let (_system_tx, system_rx) = crate::platform::status_mailbox::channel();
    let host = Arc::new(RecordingMediaHost(std::sync::Mutex::new(Vec::new())));
    session
        .enable_internal_shell_with_system_updates(host.clone(), system_rx)
        .unwrap();
    let focus = session.seat.get_keyboard().unwrap().current_focus();
    session.notify_consumer_control(ConsumerControl::VolumeUp);
    // If native dispatch fell through to legacy delivery, this nonexistent
    // subscriber would be removed on send failure.
    let subscriber = std::path::PathBuf::from("/nonexistent-nickel-media-test/subscriber.sock");
    session.launcher_subscribers.push(subscriber.clone());
    for control in [
        ConsumerControl::VolumeDown,
        ConsumerControl::VolumeMute,
        ConsumerControl::PlayPause,
        ConsumerControl::Next,
    ] {
        session.notify_consumer_control(control);
    }
    assert_eq!(session.launcher_subscribers, vec![subscriber]);
    assert_eq!(
        *host.0.lock().unwrap(),
        vec![
            ConsumerControl::VolumeUp,
            ConsumerControl::VolumeDown,
            ConsumerControl::VolumeMute,
            ConsumerControl::PlayPause,
            ConsumerControl::Next
        ]
    );
    assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), focus);
    use smithay::backend::input::KeyState;
    let before = host.0.lock().unwrap().len();
    session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
    let old = session.held_consumer_controls[&ConsumerControl::VolumeUp].0;
    session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
    assert_eq!(host.0.lock().unwrap().len(), before + 1);
    session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Released);
    session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Pressed);
    let current = session.held_consumer_controls[&ConsumerControl::VolumeUp].0;
    assert_ne!(old, current);
    assert!(!session.consumer_repeat_is_current(ConsumerControl::VolumeUp, old));
    assert!(session.consumer_repeat_is_current(ConsumerControl::VolumeUp, current));
    session.consumer_control_key(ConsumerControl::VolumeDown, KeyState::Pressed);
    assert!(session.consumer_repeat_is_current(ConsumerControl::VolumeUp, current));
    session.consumer_control_key(ConsumerControl::VolumeDown, KeyState::Released);

    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while host.0.lock().unwrap().len() == before + 3 && Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert_eq!(
        host.0.lock().unwrap().len(),
        before + 4,
        "only the current hold repeats"
    );
    session.consumer_control_key(ConsumerControl::VolumeUp, KeyState::Released);
    assert!(session.held_consumer_controls.is_empty());
    let deadline = Instant::now() + std::time::Duration::from_millis(100);
    while Instant::now() < deadline {
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(10)), &mut session)
            .unwrap();
    }
    assert_eq!(host.0.lock().unwrap().len(), before + 4);
    session.consumer_control_key(ConsumerControl::VolumeMute, KeyState::Pressed);
    assert!(
        session.held_consumer_controls[&ConsumerControl::VolumeMute]
            .1
            .is_none()
    );
    session.cancel_consumer_control_repeats();
    assert!(session.held_consumer_controls.is_empty());
}

impl SessionHost for IdleInternalHost {
    fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
        Ok(())
    }

    fn secure_storage_state(
        &self,
    ) -> Result<crate::platform::SecureStorageState, SessionRequestError> {
        Ok(crate::platform::SecureStorageState::Ready)
    }

    fn request_secure_storage_retry(&self) -> Result<(), SessionRequestError> {
        Ok(())
    }
}

#[test]
fn unchanged_internal_desktop_has_no_sixty_hertz_poll_or_redraw_loop() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let (_system_tx, system_rx) = crate::platform::status_mailbox::channel();
    session
        .enable_internal_shell_with_system_updates(Arc::new(IdleInternalHost), system_rx)
        .expect("headless internal shell");

    // Consume the intentionally immediate initialization wakeup. The
    // stable shell then owns a real application deadline well beyond a
    // frame interval (keyboard discovery currently supplies the nearest).
    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();
    let settled = session.internal_shell_timer_counters();

    // Damage/state paths may redundantly ask to maintain the schedule.
    // Sixty such calls must keep the one existing one-shot instead of
    // manufacturing a 60 Hz timer or redraw stream.
    for _ in 0..60 {
        session.schedule_internal_shell_deadline();
    }
    let after_rearm = session.internal_shell_timer_counters();
    assert_eq!(after_rearm.armed, settled.armed);
    assert_eq!(after_rearm.polls, settled.polls);
    assert_eq!(after_rearm.redraw_requests, settled.redraw_requests);

    event_loop
        .dispatch(Duration::from_millis(75), &mut session)
        .unwrap();
    let after_idle = session.internal_shell_timer_counters();
    assert!(after_idle.polls.saturating_sub(settled.polls) <= 1);
    assert_eq!(after_idle.redraw_requests, settled.redraw_requests);
}

#[test]
fn protocol_snapshot_change_wakes_the_internal_bar_projection() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "test".into(),
            logical_width: 1280,
            logical_height: 720,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    session
        .enable_internal_shell(Arc::new(IdleInternalHost))
        .expect("headless internal shell");
    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();
    let settled = session.internal_shell_timer_counters();

    session.notify_protocol_snapshot();

    let notified = session.internal_shell_timer_counters();
    assert_eq!(notified.armed, settled.armed + 1);
    assert_eq!(notified.cancelled, settled.cancelled + 1);
}

#[test]
fn first_native_launcher_open_retains_search_focus_through_key_delivery() {
    first_native_launcher_open_accepts_typing(false);
}

#[test]
fn first_native_panel_launcher_open_accepts_typing() {
    first_native_launcher_open_accepts_typing(true);
}

fn first_native_launcher_open_accepts_typing(pointer: bool) {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    if pointer {
        let panel = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surface(crate::winit_shell::SurfaceRole::Panel, Some("file-test"))
            .unwrap()
            .id;
        let runtime = session.internal_shell_surfaces[&panel];
        let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
        session
            .inject_test_input(nickel_session_protocol::TestInput::PointerMove {
                x: geometry.0 + 20,
                y: geometry.1 + 28,
            })
            .unwrap();
    }
    for state in [
        nickel_session_protocol::InputState::Pressed,
        nickel_session_protocol::InputState::Released,
    ] {
        let input = if pointer {
            nickel_session_protocol::TestInput::PointerButton {
                button: nickel_session_protocol::TestPointerButton::Left,
                state,
            }
        } else {
            nickel_session_protocol::TestInput::Key {
                key: nickel_session_protocol::TestKey::LeftMeta,
                state,
            }
        };
        session.inject_test_input(input).unwrap();
    }
    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();
    let shell = session.internal_shell.as_mut().unwrap();
    assert!(shell.launcher_visible());
    let launcher = shell
        .surface(crate::winit_shell::SurfaceRole::Launcher, None)
        .unwrap()
        .id;
    assert!(
        shell.focused_field_lease(launcher).is_some(),
        "first opening must focus search"
    );
    assert_eq!(
        session.internal_ui.focused(),
        session.internal_shell_surfaces.get(&launcher).copied()
    );
    for state in [
        nickel_session_protocol::InputState::Pressed,
        nickel_session_protocol::InputState::Released,
    ] {
        session
            .inject_test_input(nickel_session_protocol::TestInput::Key {
                key: nickel_session_protocol::TestKey::A,
                state,
            })
            .unwrap();
    }
    let shell = session.internal_shell.as_mut().unwrap();
    assert!(shell.focused_field_lease(launcher).is_some());
    assert!(
        shell
            .scene(launcher)
            .unwrap()
            .iter()
            .any(|command| matches!(
                command, nickel_ui::backend::PaintCommand::Text { text, .. } if text == "a"
            )),
        "typed character must reach the launcher scene"
    );
}

#[test]
fn internal_launcher_owns_keyboard_until_it_is_hidden() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "test".into(),
            logical_width: 1280,
            logical_height: 720,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    session
        .enable_internal_shell(Arc::new(IdleInternalHost))
        .expect("headless internal shell");
    let application = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("test".into()),
        },
        1.0,
    );
    session.register_internal_application(application).unwrap();

    assert!(session.toggle_internal_launcher());
    let launcher = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
        .and_then(|surface| session.internal_shell_surfaces.get(&surface.id))
        .copied()
        .unwrap();
    assert_eq!(session.internal_ui.focused(), Some(launcher));
    assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), None);

    assert!(session.toggle_internal_launcher());
    assert_eq!(session.internal_ui.focused(), Some(application));
}

#[test]
fn visible_shell_role_without_controller_lease_is_not_a_controller_recipient() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    assert!(session.toggle_internal_launcher());
    let launcher = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
        .and_then(|surface| session.internal_shell_surfaces.get(&surface.id))
        .copied()
        .unwrap();

    session.revoke_controller_role_lease();
    assert!(session.internal_ui.is_visible(launcher));
    assert_eq!(session.native_controller_route().target, None);

    assert!(session.focus_internal_surface(launcher));
    assert_eq!(session.native_controller_route().target, Some(launcher));
    let seat_request = session.seat_focus.acknowledged().unwrap();
    assert_eq!(
        session.seat_focus.phase(seat_request),
        Some(nickel_core::focus::FocusRequestPhase::Realized)
    );

    session.internal_ui.set_visible(launcher, false);
    assert_eq!(session.native_controller_route().target, None);
}

#[test]
fn rejected_native_handoff_clears_stale_internal_focus_and_lease() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    assert!(session.toggle_internal_launcher());
    let launcher = session.internal_ui.focused().unwrap();
    assert_eq!(session.native_controller_route().target, Some(launcher));

    // Simulate authority changing between target selection and realization.
    session.locked = true;
    assert!(!session.focus_internal_surface(launcher));
    assert_eq!(session.internal_ui.focused(), None);
    assert_eq!(session.native_controller_route().target, None);
    assert!(session.seat_focus.acknowledged().is_none());
}

#[test]
fn denied_ordinary_seat_focus_preserves_locked_internal_owner_and_controller_lease() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.lock_session();
    let lock = session.internal_ui.focused().expect("focused lock surface");
    assert_eq!(session.native_controller_route().target, Some(lock));
    let prior_seat_focus = session.seat.get_keyboard().unwrap().current_focus();

    // An ordinary XDG focus request must be rejected before it can mutate
    // either protected focus projection or the controller routing lease.
    assert!(!session.realize_seat_focus(None, nickel_core::focus::FocusScope::Ordinary));

    assert_eq!(
        session.seat.get_keyboard().unwrap().current_focus(),
        prior_seat_focus
    );
    assert_eq!(session.internal_ui.focused(), Some(lock));
    assert_eq!(session.native_controller_route().target, Some(lock));
    assert!(session.seat_focus.acknowledged().is_none());
}

#[test]
fn mapped_metadata_focus_preserves_locked_internal_owner_and_controller_lease() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.lock_session();
    let lock = session.internal_ui.focused().expect("focused lock surface");
    assert_eq!(session.native_controller_route().target, Some(lock));
    let prior_seat_focus = session.seat.get_keyboard().unwrap().current_focus();

    // Model a mapped Codex window becoming identifiable through an app-id
    // metadata change while the lock surface owns protected input.
    assert!(!session.complete_deferred_metadata_focus(None));

    assert_eq!(
        session.seat.get_keyboard().unwrap().current_focus(),
        prior_seat_focus
    );
    assert_eq!(session.internal_ui.focused(), Some(lock));
    assert_eq!(session.native_controller_route().target, Some(lock));
    assert!(session.seat_focus.acknowledged().is_none());
}

#[test]
fn controller_batch_drops_old_route_tail_after_launcher_changes_recipient() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
    let event = nickel_ui::ControllerEnvelope {
        device: nickel_input::controller::ControllerId(7),
        action: Some(nickel_ui::ControllerAction::Launcher),
        edge: nickel_input::KeyEdge::Pressed,
        repeat: false,
        family: nickel_ui::ControllerFamily::Xbox,
        evidence: nickel_ui::ControllerSourceEvidence {
            seat: 0,
            source_namespace: "test".into(),
            backend: "test".into(),
            native: nickel_input::NativeCode::Numeric(7),
            fingerprint: None,
            identity_capability: "native",
            physical: nickel_ui::ControllerPhysicalControl::Button(
                nickel_input::controller::ControllerButton::Guide,
            ),
            backend_order: 1,
            produced_unix_ms: 1,
        },
    };

    session.handle_brokered_controller_batch(
        vec![
            event.clone(),
            nickel_ui::ControllerEnvelope {
                repeat: true,
                ..event
            },
        ],
        false,
    );

    assert!(
        session.internal_shell.as_ref().unwrap().launcher_visible(),
        "the first transition must invalidate, not retarget, the queued second action"
    );
    assert!(session.controller_routing_epoch >= 2);
}

#[test]
fn queued_confirm_keeps_pre_launcher_recipient_epoch_across_batches() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let queued_epoch = session.refresh_controller_route().0;
    let launcher = nickel_ui::ControllerEnvelope {
        device: nickel_input::controller::ControllerId(7),
        action: Some(nickel_ui::ControllerAction::Launcher),
        edge: nickel_input::KeyEdge::Pressed,
        repeat: false,
        family: nickel_ui::ControllerFamily::Xbox,
        evidence: nickel_ui::ControllerSourceEvidence {
            seat: 0,
            source_namespace: "test".into(),
            backend: "test".into(),
            native: nickel_input::NativeCode::Numeric(7),
            fingerprint: None,
            identity_capability: "native",
            physical: nickel_ui::ControllerPhysicalControl::Button(
                nickel_input::controller::ControllerButton::Guide,
            ),
            backend_order: 1,
            produced_unix_ms: 1,
        },
    };
    let confirm = nickel_ui::ControllerEnvelope {
        action: Some(nickel_ui::ControllerAction::Confirm),
        evidence: nickel_ui::ControllerSourceEvidence {
            native: nickel_input::NativeCode::Numeric(0),
            physical: nickel_ui::ControllerPhysicalControl::Button(
                nickel_input::controller::ControllerButton::South,
            ),
            backend_order: 2,
            ..launcher.evidence.clone()
        },
        ..launcher.clone()
    };

    session.handle_brokered_controller_batch_for_route(vec![launcher], false, queued_epoch);
    assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
    let launcher_surface = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
        .and_then(|surface| session.internal_shell_surfaces.get(&surface.id))
        .copied()
        .unwrap();
    assert!(session.focus_internal_surface(launcher_surface));
    assert_ne!(session.controller_routing_epoch, queued_epoch);
    assert_eq!(
        session
            .controller_published_routing_epoch
            .load(std::sync::atomic::Ordering::Acquire),
        session.controller_routing_epoch,
        "the focus transition must publish its fence before later controller collection"
    );

    session.handle_brokered_controller_batch_for_route(vec![confirm], false, queued_epoch);
    assert!(
        session.internal_shell.as_ref().unwrap().launcher_visible(),
        "the separately queued confirm must be rejected at the recipient-change barrier"
    );
}

#[test]
fn broker_overflow_requests_neutral_probe_and_rearms_internal_owner() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let event = nickel_ui::ControllerEnvelope {
        device: nickel_input::controller::ControllerId(7),
        action: None,
        edge: nickel_input::KeyEdge::Pressed,
        repeat: false,
        family: nickel_ui::ControllerFamily::Xbox,
        evidence: nickel_ui::ControllerSourceEvidence {
            seat: 0,
            source_namespace: "test".into(),
            backend: "test".into(),
            native: nickel_input::NativeCode::Numeric(7),
            fingerprint: None,
            identity_capability: "native",
            physical: nickel_ui::ControllerPhysicalControl::Button(
                nickel_input::controller::ControllerButton::Guide,
            ),
            backend_order: 1,
            produced_unix_ms: 1,
        },
    };
    let epoch = session.refresh_controller_route().0;
    session.handle_brokered_controller_batch_for_route(
        vec![event; nickel_session_protocol::controller_broker::DEFAULT_CONTROLLER_QUEUE_LIMIT + 1],
        false,
        epoch,
    );
    assert!(session.controller_broker.active_lease().is_none());
    assert!(
        session
            .controller_neutral_probe_requested
            .load(std::sync::atomic::Ordering::Acquire)
    );

    session.handle_brokered_controller_batch_for_route(Vec::new(), true, epoch);
    assert_eq!(
        session
            .controller_broker
            .active_lease()
            .map(|lease| lease.host),
        Some(ControllerHostId(0))
    );
}

#[test]
fn timed_out_security_takeover_rearms_internal_on_same_steady_neutral_batch() {
    use nickel_session_protocol::controller_broker::{BrokerMessage, HostId, TransferStatus};

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let predecessor_host = HostId(41);
    let destination_host = HostId(42);
    let predecessor_connection = session.controller_broker.attach(predecessor_host);
    let destination_connection = session.controller_broker.attach(destination_host);

    let TransferStatus::Pending { cutoff, .. } =
        session
            .controller_broker
            .begin_transfer(predecessor_host, predecessor_connection, 0, 100)
    else {
        panic!("initial external transfer must start");
    };
    let internal_lease = session
        .controller_broker
        .drain(HostId(0), session.controller_internal_connection)
        .into_iter()
        .find_map(|message| match message {
            BrokerMessage::Revoke { lease_epoch, .. } => Some(lease_epoch),
            _ => None,
        })
        .unwrap();
    session.controller_broker.acknowledge_quiescence(
        HostId(0),
        session.controller_internal_connection,
        internal_lease,
        cutoff,
    );
    session.controller_broker.set_neutral(true);
    let predecessor_lease = session.controller_broker.active_lease().unwrap();

    session.controller_broker.set_neutral(false);
    let TransferStatus::Pending {
        cutoff: predecessor_cutoff,
        ..
    } = session
        .controller_broker
        .begin_transfer(destination_host, destination_connection, 10, 1)
    else {
        panic!("second external transfer must start");
    };
    session.controller_broker.expire_transfer(11);
    session.begin_controller_security_takeover();
    session.controller_broker.acknowledge_quiescence(
        predecessor_host,
        predecessor_connection,
        predecessor_lease.epoch,
        predecessor_cutoff,
    );

    session.handle_brokered_controller_batch(Vec::new(), true);
    assert_eq!(
        session
            .controller_broker
            .active_lease()
            .map(|lease| lease.host),
        Some(HostId(0))
    );
}

#[test]
fn controller_host_lease_fails_closed_without_exact_focused_client() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let peer = 424_242;
    let attached = session.handle_controller_host_request(
        peer,
        nickel_session_protocol::ControllerHostRequest::Attach,
    );
    let generation = match attached {
        nickel_session_protocol::ServerMessage::ControllerHost(
            nickel_session_protocol::ControllerHostResponse::Attached {
                connection_generation,
                ..
            },
        ) => connection_generation,
        other => panic!("unexpected attach response: {other:?}"),
    };
    assert!(matches!(
        session.handle_controller_host_request(
            peer,
            nickel_session_protocol::ControllerHostRequest::RequestLease {
                connection_generation: generation,
            },
        ),
        nickel_session_protocol::ServerMessage::ControllerHost(
            nickel_session_protocol::ControllerHostResponse::LeaseFailed
        )
    ));
}

#[test]
fn focused_controller_host_retries_after_unrelated_internal_handoff() {
    use nickel_session_protocol::controller_broker::{ControllerBroker, HostId, TransferStatus};

    let mut broker = ControllerBroker::<nickel_session_protocol::ControllerEnvelopePayload>::new(8);
    let internal = broker.attach(HostId(0));
    let a = broker.attach(HostId(41));
    let b = broker.attach(HostId(42));
    let predecessor = broker.grant(HostId(41), a).unwrap();
    broker.set_neutral(false);
    let internal_pending = broker.begin_transfer(
        HostId(0),
        internal,
        10,
        nickel_session_protocol::controller_broker::DEFAULT_TRANSFER_DEADLINE_MS,
    );
    let TransferStatus::Pending { cutoff, .. } = internal_pending else {
        panic!("A to internal handoff must start");
    };

    assert_eq!(
        super::begin_controller_host_transfer(&mut broker, HostId(42), b, internal, 11),
        TransferStatus::Failed,
        "host B must retry instead of polling the internal destination's lease"
    );
    assert_eq!(
        broker.begin_transfer(
            HostId(0),
            internal,
            12,
            nickel_session_protocol::controller_broker::DEFAULT_TRANSFER_DEADLINE_MS,
        ),
        internal_pending,
        "the unrelated request must preserve the original revocation and destination"
    );

    assert_eq!(
        broker.acknowledge_quiescence(HostId(41), a, predecessor.epoch, cutoff),
        internal_pending
    );
    assert_eq!(broker.set_neutral(true).unwrap().host, HostId(0));
    let TransferStatus::Granted(granted) =
        super::begin_controller_host_transfer(&mut broker, HostId(42), b, internal, 13)
    else {
        panic!("host B retry must complete the bounded internal handoff");
    };
    assert_eq!(granted.host, HostId(42));
    assert_eq!(granted.connection_generation, b);
}

#[test]
fn shell_diagnostics_follow_owner_scene_visibility_and_exclude_lock() {
    use nickel_remote_control::diagnostics::ShellDiagnosticRole;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let (records, truncated) = session.remote_shell_surface_diagnostics();
    assert!(!truncated);
    assert!(
        records
            .iter()
            .any(|record| matches!(record.role, ShellDiagnosticRole::Desktop))
    );
    assert!(
        records
            .iter()
            .any(|record| matches!(record.role, ShellDiagnosticRole::Panel))
    );
    assert!(
        !records
            .iter()
            .any(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
    );
    session.set_launcher_visible(true);
    let (shown, _) = session.remote_shell_surface_diagnostics();
    let launcher = shown
        .iter()
        .find(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
        .unwrap();
    let shell = session.internal_shell.as_ref().unwrap();
    let owner = shell
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
        .unwrap();
    let owner_id = owner.id;
    let (tree_generation, semantics) = shell.bounded_shell_semantics(owner_id).unwrap();
    assert!(tree_generation > 0);
    assert!(semantics.iter().any(|node| node.focused));
    assert!(
        semantics
            .iter()
            .any(|node| node.name.as_deref() == Some("Home"))
    );
    assert_eq!(
        shell.bounded_shell_semantics(owner_id).unwrap().0,
        tree_generation,
        "observation must not rebuild or focus another viewport"
    );
    let runtime = session.internal_shell_surfaces[&owner.id];
    let placement = session.internal_ui.placement(runtime).unwrap();
    assert_eq!(launcher.generation, runtime.snapshot_token());
    assert_eq!(launcher.scene_generation, owner.scene_generation);
    assert_eq!(
        launcher.geometry,
        [
            i64::from(placement.geometry.0),
            i64::from(placement.geometry.1),
            i64::from(placement.geometry.2),
            i64::from(placement.geometry.3)
        ]
    );
    assert!(launcher.keyboard_focused);
    let input = session.remote_input_diagnostic(&[], &[], 41, 42);
    let recipient = input.keyboard.unwrap();
    assert!(recipient.focused_window.is_none());
    let focused_surface = recipient.focused_surface.unwrap();
    assert_eq!(focused_surface.id, launcher.id);
    assert_eq!(focused_surface.generation, launcher.generation);
    let placement = placement.clone();
    session
        .internal_ui
        .configure_surface(runtime, placement, 1.5);
    let (scaled, _) = session.remote_shell_surface_diagnostics();
    let scaled = scaled
        .iter()
        .find(|record| record.id == launcher.id)
        .unwrap();
    assert_eq!(scaled.scale_factor, 1.5);
    assert!(scaled.redraw_pending);
    assert_eq!(scaled.generation, launcher.generation);
    let renderer = session
        .remote_shell_renderer_diagnostics(42)
        .into_iter()
        .find(|record| record.surface == launcher.id)
        .unwrap();
    let actual = session.internal_ui.renderer_diagnostics(runtime).unwrap();
    assert_eq!(renderer.surface_generation, launcher.generation);
    assert_eq!(renderer.observed_at_us, 42);
    assert_eq!(renderer.gpu_frames, actual.gpu_frames);
    assert_eq!(renderer.fallback_frames, actual.fallback_frames);
    assert_eq!(
        renderer.software_frame_bytes,
        actual.software_frame_bytes as u64
    );
    session.set_launcher_visible(false);
    assert!(
        session
            .internal_shell
            .as_ref()
            .unwrap()
            .bounded_shell_semantics(owner_id)
            .is_err()
    );
    assert!(
        session
            .remote_shell_renderer_diagnostics(43)
            .iter()
            .all(|record| record.surface != launcher.id)
    );
    assert!(
        !session
            .remote_shell_surface_diagnostics()
            .0
            .iter()
            .any(|record| matches!(record.role, ShellDiagnosticRole::Launcher))
    );
    session.locked = true;
    assert!(session.remote_shell_surface_diagnostics().0.is_empty());
    assert!(session.remote_shell_renderer_diagnostics(44).is_empty());
    let input = session.remote_input_diagnostic(&[], &[], 45, 46);
    assert!(input.keyboard.is_none());
    assert!(input.pointer.is_none());
    assert!(input.pointer_hit_test.is_none());
}

#[test]
fn remote_panel_effect_is_staged_without_changing_local_host_dispatch() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let shell = session.internal_shell.as_mut().unwrap();
    assert!(!shell.launcher_visible());
    let commands = shell
        .shell_mut()
        .stage_remote_shell_effect(
            crate::live_shell::remote_semantics::RemoteShellEffect::Panel(
                crate::live_shell::PanelAction::Launcher,
                Some("file-test".into()),
            ),
        )
        .unwrap();
    assert!(matches!(
        commands.as_slice(),
        [crate::platform::ShellCommand::Show]
    ));
    assert!(
        !shell.launcher_visible(),
        "staging must not apply visibility or defer unguarded work"
    );
    session.set_launcher_visible(true);
    assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
}

#[test]
fn ordinary_shell_semantic_mutation_updates_real_launcher_and_rejects_stale_tree() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    session.set_launcher_visible(true);
    let shell = session.internal_shell.as_mut().unwrap();
    let id = shell
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Launcher)
        .unwrap()
        .id;
    let (generation, nodes) = shell.bounded_shell_semantics(id).unwrap();
    let ordinal = nodes
        .iter()
        .position(|node| node.role == Some(nickel_ui::SemanticRole::TextField))
        .unwrap();
    let action = |text: &str| {
        nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(text.into()))
    };
    let outcome = shell
        .perform_bounded_shell_action(
            id,
            generation,
            ordinal,
            action("semantic launcher query"),
            2048,
        )
        .unwrap();
    assert!(outcome.effects.is_empty());
    assert!(outcome.host.semantic_failures.is_empty());
    let (next, nodes) = shell.bounded_shell_semantics(id).unwrap();
    assert_ne!(next, generation);
    assert!(nodes.iter().any(|node| matches!(&node.value, Some(nickel_ui::SemanticValueSnapshot::Text(text)) if text == "semantic launcher query")));
    assert!(
        shell
            .perform_bounded_shell_action(id, generation, ordinal, action("stale query"), 2048)
            .is_err()
    );
    session.set_launcher_visible(false);
    assert!(
        session
            .internal_shell
            .as_mut()
            .unwrap()
            .perform_bounded_shell_action(id, next, ordinal, action("hidden query"), 2048)
            .is_err()
    );
}

#[test]
fn launcher_protocol_visibility_updates_hosted_scene_and_restores_focus() {
    use nickel_session_protocol::Command;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let application = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (80, 90, 640, 480),
            output: Some("file-test".into()),
        },
        1.0,
    );
    session.register_internal_application(application).unwrap();
    for command in [
        Command::SetLauncherVisible { visible: true },
        Command::SetLauncherVisible { visible: true },
    ] {
        assert!(matches!(
            session.handle_protocol_command(command, None, 0),
            ServerMessage::Ack
        ));
        assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
        assert!(session.launcher_visibility.is_visible());
        assert!(
            session
                .protocol_shell_surfaces()
                .iter()
                .any(|surface| surface.role == ShellRole::Launcher && surface.geometry.is_some())
        );
        assert_ne!(session.internal_ui.focused(), Some(application));
    }
    for command in [
        Command::SetLauncherVisible { visible: false },
        Command::SetLauncherVisible { visible: false },
    ] {
        assert!(matches!(
            session.handle_protocol_command(command, None, 0),
            ServerMessage::Ack
        ));
        assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
        assert!(!session.launcher_visibility.is_visible());
        assert_eq!(session.internal_ui.focused(), Some(application));
        assert!(
            session
                .protocol_shell_surfaces()
                .iter()
                .any(|surface| surface.role == ShellRole::Launcher && surface.geometry.is_none())
        );
    }
    session.handle_protocol_command(Command::ToggleLauncher, None, 0);
    assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
    session.handle_protocol_command(Command::ToggleLauncher, None, 0);
    assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
    assert_eq!(session.internal_ui.focused(), Some(application));
}

#[test]
fn internal_shell_protocol_geometry_uses_authoritative_global_placement() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, session) = internal_shell_test_session();
    let panel = session
        .protocol_shell_surfaces()
        .into_iter()
        .find(|surface| surface.role == ShellRole::Panel)
        .expect("internal panel snapshot");

    assert_eq!(
        panel.geometry,
        Some(nickel_session_protocol::Geometry {
            x: 0,
            y: 664,
            width: 1280,
            height: 56,
        })
    );
}

#[test]
fn session_lock_drives_and_focuses_the_compositor_owned_lock_surface() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let lock = session
        .internal_shell
        .as_ref()
        .unwrap()
        .surfaces()
        .iter()
        .find(|surface| surface.role == crate::winit_shell::SurfaceRole::Lock)
        .unwrap()
        .id;

    assert!(!session.internal_shell.as_ref().unwrap().visible(lock));
    assert!(!session.internal_shell_surfaces.contains_key(&lock));

    session.lock_session();

    assert!(session.locked);
    assert!(session.internal_shell.as_ref().unwrap().visible(lock));
    let focused = session.internal_ui.focused().expect("focused lock surface");
    assert!(
        session
            .internal_shell
            .as_ref()
            .unwrap()
            .surfaces()
            .iter()
            .any(
                |surface| surface.role == crate::winit_shell::SurfaceRole::Lock
                    && session.internal_shell_surfaces.get(&surface.id) == Some(&focused)
            )
    );
    assert_eq!(session.seat.get_keyboard().unwrap().current_focus(), None);

    session.unlock_session();

    assert!(!session.locked);
    assert!(!session.internal_shell.as_ref().unwrap().visible(lock));
    assert!(!session.internal_shell_surfaces.contains_key(&lock));
}

#[test]
fn task_switcher_opens_on_the_pointer_output() {
    use nickel_session_protocol::{TestInput, TestOutput};

    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "secondary".into(),
            logical_width: 1000,
            logical_height: 800,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    let secondary = session
        .space
        .outputs()
        .find(|output| output.name() == "secondary")
        .unwrap()
        .clone();
    session.space.map_output(&secondary, (-1000, 0));
    session
        .inject_test_input(TestInput::PointerMove { x: -500, y: 200 })
        .unwrap();

    assert_eq!(
        session.task_switcher_output_name().as_deref(),
        Some("secondary")
    );
}

#[test]
fn native_screenshot_captures_and_opens_on_the_invoking_pointer_output() {
    use crate::session_host::DesktopCapturePoll;
    use crate::winit_shell::SurfaceRole;
    use nickel_session_protocol::{InputState, TestInput, TestKey, TestPointerButton};
    #[derive(Default)]
    struct CaptureHost(std::sync::Mutex<Vec<Option<String>>>);
    impl SessionHost for CaptureHost {
        fn dispatch(&self, _command: ShellCommand) -> Result<(), SessionRequestError> {
            Ok(())
        }
        fn capture_desktop(&self, output: Option<&str>) -> DesktopCapturePoll {
            self.0.lock().unwrap().push(output.map(str::to_owned));
            DesktopCapturePoll::Ready(Ok(crate::platform::DesktopCapture {
                image: image::RgbaImage::new(1000, 800),
            }))
        }
    }
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    for (name, width, height, scale_120) in
        [("primary", 1280, 720, 120), ("secondary", 1000, 800, 180)]
    {
        session
            .apply_test_output(TestOutput::Connect {
                name: name.into(),
                logical_width: width,
                logical_height: height,
                scale_120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
    }
    let secondary = session
        .space
        .outputs()
        .find(|output| output.name() == "secondary")
        .unwrap()
        .clone();
    session.space.map_output(&secondary, (-1000, -120));
    let geometry = session.space.output_geometry(&secondary).unwrap();
    let host = Arc::new(CaptureHost::default());
    let (_sender, receiver) = crate::platform::status_mailbox::channel();
    session
        .enable_internal_shell_with_system_updates(host.clone(), receiver)
        .unwrap();
    session
        .inject_test_input(TestInput::PointerMove { x: -900, y: 100 })
        .unwrap();
    for state in [InputState::Pressed, InputState::Released] {
        session
            .inject_test_input(TestInput::Key {
                key: TestKey::PrintScreen,
                state,
            })
            .unwrap();
    }
    // Moving during the capture delay must not change the target display.
    session
        .inject_test_input(TestInput::PointerMove { x: 100, y: 100 })
        .unwrap();
    let shell = session.internal_shell.as_mut().unwrap();
    shell.poll(Instant::now() + Duration::from_millis(100));
    let screenshot = shell.surface(SurfaceRole::Screenshot, None).unwrap().id;
    assert!(shell.visible(screenshot));
    assert_eq!(*host.0.lock().unwrap(), vec![Some("secondary".into())]);
    session.sync_internal_shell();
    let runtime = session.internal_shell_surfaces[&screenshot];
    let placement = session.internal_ui.placement(runtime).unwrap();
    assert_eq!(placement.output.as_deref(), Some("secondary"));
    assert_eq!(
        placement.geometry,
        (
            geometry.loc.x,
            geometry.loc.y,
            geometry.size.w as u32,
            geometry.size.h as u32
        )
    );
    assert_eq!(session.internal_ui.focused(), Some(runtime));

    session
        .inject_test_input(TestInput::PointerMove {
            x: geometry.loc.x + 180,
            y: geometry.loc.y + 180,
        })
        .unwrap();
    assert_eq!(
        session
            .internal_ui
            .surface_at(
                (
                    f64::from(geometry.loc.x + 180),
                    f64::from(geometry.loc.y + 180),
                ),
                true,
            )
            .map(|(id, _)| id),
        Some(runtime),
        "native screenshot must be the pointer hit target"
    );
    session
        .inject_test_input(TestInput::PointerButton {
            button: TestPointerButton::Left,
            state: InputState::Pressed,
        })
        .unwrap();
    assert!(
        session
            .internal_shell
            .as_ref()
            .unwrap()
            .pointer_interaction_active(),
        "native screenshot press must retain pointer capture"
    );
    session
        .inject_test_input(TestInput::PointerMove {
            x: geometry.loc.x + 620,
            y: geometry.loc.y + 520,
        })
        .unwrap();
    session
        .inject_test_input(TestInput::PointerButton {
            button: TestPointerButton::Left,
            state: InputState::Released,
        })
        .unwrap();
    assert!(
        !session
            .internal_shell
            .as_ref()
            .unwrap()
            .pointer_interaction_active(),
        "native screenshot release must finish pointer capture"
    );
}

#[test]
fn native_screenshot_clipboard_retains_each_payload_for_repeated_paste() {
    use crate::session::{SessionAuthorityRequest, handlers::SelectionOwner};
    use smithay::wayland::selection::{
        SelectionHandler, SelectionTarget, data_device::current_data_device_selection_userdata,
    };
    use std::io::Read;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([23, 45, 67, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = Arc::new(png.into_inner());
    // Screenshot paths must work even when no text editor owns a copy limit.
    session.native_clipboard.text_limit = None;
    for request in [
        SessionAuthorityRequest::PublishClipboardImage(png.clone()),
        SessionAuthorityRequest::PublishClipboardText("/tmp/screenshot.png".into()),
        SessionAuthorityRequest::PublishClipboardImage(png.clone()),
    ] {
        assert!(matches!(
            session.handle_authority_request(request),
            nickel_session_protocol::ServerMessage::Ack
        ));
        let owner = current_data_device_selection_userdata(&session.seat)
            .unwrap()
            .clone();
        let (mime, expected) = match &owner {
            SelectionOwner::NativeImage(bytes) => ("image/png", bytes.as_slice()),
            SelectionOwner::NativeText(text) => ("text/plain;charset=utf-8", text.as_bytes()),
            _ => panic!("clipboard must stay compositor-owned"),
        };
        assert!(
            session
                .native_clipboard
                .mime_types
                .iter()
                .any(|value| value == mime)
        );
        for _ in 0..3 {
            let (mut reader, writer) = std::os::unix::net::UnixStream::pair().unwrap();
            reader
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let seat = session.seat.clone();
            SelectionHandler::send_selection(
                &mut session,
                SelectionTarget::Clipboard,
                mime.into(),
                writer.into(),
                seat,
                &owner,
            );
            let mut received = Vec::new();
            reader.read_to_end(&mut received).unwrap();
            assert_eq!(received, expected);
        }
    }
}

#[test]
fn native_screenshot_claims_keyboard_on_show_and_escape_hides_without_clicking() {
    use crate::winit_shell::SurfaceRole;
    use nickel_session_protocol::{InputState, ShortcutAction, TestInput, TestKey};
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let shell = session.internal_shell.as_mut().unwrap();
    shell.global_shortcut(ShortcutAction::ShowScreenshotTool);
    shell.poll(Instant::now() + Duration::from_millis(100));
    let screenshot = shell.surface(SurfaceRole::Screenshot, None).unwrap().id;
    assert!(shell.visible(screenshot));
    session.sync_internal_shell();
    let runtime = session.internal_shell_surfaces[&screenshot];
    assert_eq!(session.internal_ui.focused(), Some(runtime));
    assert!(session.remote_desktop_events.snapshot().events.iter().any(
            |event| matches!(
                event.event,
                nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                    surface_generation,
                    role: nickel_remote_control::desktop_events::ShellEventRole::Screenshot,
                    visible: true,
                } if surface_generation == runtime.snapshot_token()
            )
        ));
    let record = session
        .remote_shell_surface_diagnostics()
        .0
        .into_iter()
        .find(|record| record.generation == runtime.snapshot_token())
        .expect("ordinary screenshot transient is projected");
    assert!(matches!(
        record.role,
        nickel_remote_control::diagnostics::ShellDiagnosticRole::Screenshot
    ));
    let (tree_generation, nodes) = session
        .internal_shell
        .as_ref()
        .unwrap()
        .bounded_shell_semantics(screenshot)
        .expect("bounded screenshot semantics");
    assert!(tree_generation > 0);
    assert!(nodes.len() <= nickel_remote_control::semantics::MAX_RESOLVED_NODES);
    for state in [InputState::Pressed, InputState::Released] {
        session
            .inject_test_input(TestInput::Key {
                key: TestKey::Escape,
                state,
            })
            .unwrap();
    }
    assert!(!session.internal_shell.as_ref().unwrap().visible(screenshot));
    assert!(!session.internal_ui.is_visible(runtime));
    assert_ne!(session.internal_ui.focused(), Some(runtime));
    assert!(session.remote_desktop_events.snapshot().events.iter().any(
            |event| matches!(
                event.event,
                nickel_remote_control::desktop_events::DesktopEventKind::ShellSurfaceVisibilityChanged {
                    surface_generation,
                    role: nickel_remote_control::desktop_events::ShellEventRole::Screenshot,
                    visible: false,
                } if surface_generation == runtime.snapshot_token()
            )
        ));
}

#[test]
fn control_center_hides_on_client_or_internal_focus_transfer_and_stays_hidden() {
    use crate::winit_shell::SurfaceRole;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    let application = session.internal_ui.insert(
        InternalWindowTestApp,
        crate::session::InternalSurfacePlacement {
            role: crate::session::InternalSurfaceRole::Application,
            geometry: (800, 100, 300, 300),
            output: Some("file-test".into()),
        },
        1.0,
    );
    for client in [true, false] {
        session
            .internal_shell
            .as_mut()
            .unwrap()
            .global_shortcut(nickel_session_protocol::ShortcutAction::ShowControlCenter);
        session.sync_internal_shell();
        let control = session
            .internal_shell
            .as_ref()
            .unwrap()
            .surface(SurfaceRole::ControlCenter, None)
            .unwrap()
            .id;
        let runtime = session.internal_shell_surfaces[&control];
        assert_eq!(session.internal_ui.focused(), Some(runtime));
        assert!(session.internal_shell.as_ref().unwrap().visible(control));
        let handled = session
            .internal_ui
            .pointer_button_with_client((900.0, 200.0), true, client);
        assert_eq!(handled, !client);
        session.flush_internal_shell_input();
        assert!(!session.internal_shell.as_ref().unwrap().visible(control));
        assert!(!session.internal_ui.is_visible(runtime));
        assert_eq!(
            session.internal_ui.focused(),
            (!client).then_some(application)
        );
        session.flush_internal_shell_input();
        session.sync_internal_shell();
        assert!(!session.internal_shell.as_ref().unwrap().visible(control));
        assert_eq!(
            session.internal_ui.focused(),
            (!client).then_some(application)
        );
    }
}

#[test]
fn launcher_sidebar_press_is_not_dismissed_for_a_client_underneath() {
    use nickel_session_protocol::{InputState, TestInput, TestPointerButton};
    use nickel_ui::backend::PaintCommand;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = internal_shell_test_session();
    assert!(session.toggle_internal_launcher());
    let shell = session.internal_shell.as_mut().unwrap();
    let launcher = shell
        .surface(crate::winit_shell::SurfaceRole::Launcher, None)
        .unwrap()
        .id;
    // The native launcher owns its sidebar even if an ordinary client scene
    // lies underneath the same global point.
    let bounds = shell
        .scene(launcher)
        .unwrap()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::Text { bounds, text, .. } if text == "Places" => Some(*bounds),
            _ => None,
        })
        .min_by(|a, b| a.origin.x.total_cmp(&b.origin.x))
        .expect("Places sidebar label");
    let runtime = session.internal_shell_surfaces[&launcher];
    let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
    session
        .inject_test_input(TestInput::PointerMove {
            x: geometry.0 + (bounds.origin.x + bounds.size.width / 2.0) as i32,
            y: geometry.1 + (bounds.origin.y + bounds.size.height / 2.0) as i32,
        })
        .unwrap();
    // This boundary is invoked when the native client scene occupies the point.
    // The foreground launcher must keep ownership despite that underlying client.
    assert!(!session.dismiss_internal_launcher_for_client_press());
    for state in [InputState::Pressed, InputState::Released] {
        session
            .inject_test_input(TestInput::PointerButton {
                button: TestPointerButton::Left,
                state,
            })
            .unwrap();
        assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
        assert_eq!(session.internal_ui.focused(), Some(runtime));
    }
    assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
}

#[test]
fn native_launcher_all_applications_tile_accepts_pointer_activation() {
    use nickel_session_protocol::{InputState, TestInput, TestPointerButton};
    use nickel_ui::backend::PaintCommand;
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = internal_shell_test_session();
    assert!(session.toggle_internal_launcher());
    let shell = session.internal_shell.as_mut().unwrap();
    let launcher = shell
        .surface(crate::winit_shell::SurfaceRole::Launcher, None)
        .unwrap()
        .id;
    assert!(shell.scene(launcher).unwrap().iter().any(|command| {
        matches!(command, PaintCommand::Text { text, .. } if text == "All applications")
    }));
    let runtime = session.internal_shell_surfaces[&launcher];
    let geometry = session.internal_ui.placement(runtime).unwrap().geometry;
    let target = session
        .internal_shell
        .as_ref()
        .unwrap()
        .bounded_shell_semantics(launcher)
        .unwrap()
        .1
        .into_iter()
        .find(|node| node.name.as_deref() == Some("All applications"))
        .unwrap();
    let point = (
        geometry.0 + (target.bounds.origin.x + target.bounds.size.width / 2.0) as i32,
        geometry.1 + (target.bounds.origin.y + target.bounds.size.height / 2.0) as i32,
    );
    assert_eq!(
        session
            .internal_ui
            .surface_at((f64::from(point.0), f64::from(point.1)), true)
            .map(|(id, _)| id),
        Some(runtime),
        "launcher tile must own its pointer coordinate"
    );
    session
        .inject_test_input(TestInput::PointerMove {
            x: point.0,
            y: point.1,
        })
        .unwrap();
    for state in [InputState::Pressed, InputState::Released] {
        session
            .inject_test_input(TestInput::PointerButton {
                button: TestPointerButton::Left,
                state,
            })
            .unwrap();
    }
    event_loop
        .dispatch(Duration::from_millis(25), &mut session)
        .unwrap();
    session.flush_internal_shell_input();
    session.sync_internal_shell();
    // The application view places its return link beneath its heading,
    // reversing the painted order seen on the favorites dashboard.
    let scene = session
        .internal_shell
        .as_mut()
        .unwrap()
        .scene(launcher)
        .unwrap();
    let text_y = |label| {
        scene.iter().find_map(|command| match command {
            PaintCommand::Text { bounds, text, .. } if text == label => Some(bounds.origin.y),
            _ => None,
        })
    };
    assert!(
        text_y("All applications").unwrap() < text_y("Pinned & recent").unwrap(),
        "pointer activation should show the application heading above the return link"
    );
}

#[test]
fn ordinary_client_press_dismisses_internal_launcher_without_restoring_displaced_focus() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "test".into(),
            logical_width: 1280,
            logical_height: 720,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    session
        .enable_internal_shell(Arc::new(IdleInternalHost))
        .expect("headless internal shell");

    assert!(session.toggle_internal_launcher());
    assert!(session.internal_shell.as_ref().unwrap().launcher_visible());
    assert!(session.dismiss_internal_launcher_for_client_press());
    assert!(!session.internal_shell.as_ref().unwrap().launcher_visible());
    assert!(session.launcher_restore_window.is_none());
}

#[test]
fn applying_multi_output_fractional_scale_rebuilds_internal_surfaces_at_native_scale() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    for name in ["one", "quarter", "half", "double"] {
        session
            .apply_test_output(TestOutput::Connect {
                name: name.into(),
                logical_width: 1200,
                logical_height: 900,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
    }
    session
        .enable_internal_shell(Arc::new(IdleInternalHost))
        .unwrap();

    session
        .apply_output_layout(nickel_session_protocol::OutputLayout {
            primary: "quarter".into(),
            placements: vec![
                nickel_session_protocol::OutputPlacement {
                    name: "one".into(),
                    x: -1200,
                    y: 0,
                    enabled: true,
                    scale_120: 120,
                    mode: None,
                },
                nickel_session_protocol::OutputPlacement {
                    name: "quarter".into(),
                    x: 0,
                    y: -120,
                    enabled: true,
                    scale_120: 150,
                    mode: None,
                },
                nickel_session_protocol::OutputPlacement {
                    name: "half".into(),
                    x: 960,
                    y: 0,
                    enabled: true,
                    scale_120: 180,
                    mode: None,
                },
                nickel_session_protocol::OutputPlacement {
                    name: "double".into(),
                    x: 1760,
                    y: -80,
                    enabled: true,
                    scale_120: 240,
                    mode: None,
                },
            ],
        })
        .unwrap();

    let outputs = session.protocol_outputs();
    for (name, x, y, width, height, scale_120) in [
        ("one", 0, 120, 1200, 900, 120),
        ("quarter", 1200, 0, 960, 720, 150),
        ("half", 2160, 120, 800, 600, 180),
        ("double", 2960, 40, 600, 450, 240),
    ] {
        let output = outputs.iter().find(|output| output.name == name).unwrap();
        assert_eq!(
            (
                output.geometry.x,
                output.geometry.y,
                output.geometry.width,
                output.geometry.height,
                output.scale_120,
            ),
            (x, y, width, height, scale_120)
        );
    }

    // Display settings normalize their saved layout to a non-negative
    // origin. Compositor-global coordinates can nevertheless be negative
    // while topology is changing, so move the already scaled outputs as a
    // group and ensure internal chrome follows that authoritative space.
    let shifted_outputs = session.space.outputs().cloned().collect::<Vec<_>>();
    for output in shifted_outputs {
        let geometry = session.space.output_geometry(&output).unwrap();
        let shifted = (geometry.loc.x - 1400, geometry.loc.y - 300).into();
        output.change_current_state(None, None, None, Some(shifted));
        session.space.map_output(&output, shifted);
    }
    session.reconcile_internal_shell_outputs();
    session.space.refresh();
    let outputs = session.protocol_outputs();
    assert!(outputs.iter().any(|output| output.geometry.x < 0));
    assert!(outputs.iter().any(|output| output.geometry.y < 0));

    let shell = session.internal_shell.as_ref().unwrap();
    for (name, expected) in [
        ("one", 1.0_f32),
        ("quarter", 1.25_f32),
        ("half", 1.5_f32),
        ("double", 2.0_f32),
    ] {
        let surface = shell
            .surface(crate::winit_shell::SurfaceRole::Desktop, Some(name))
            .unwrap();
        let runtime = session.internal_shell_surfaces[&surface.id];
        assert_eq!(session.internal_ui.scale_factor(runtime), Some(expected));

        let panel = shell
            .surface(crate::winit_shell::SurfaceRole::Panel, Some(name))
            .unwrap();
        let panel_runtime = session.internal_shell_surfaces[&panel.id];
        let placement = session.internal_ui.placement(panel_runtime).unwrap();
        let output = outputs.iter().find(|output| output.name == name).unwrap();
        assert_eq!(placement.geometry.0, output.geometry.x);
        assert_eq!(
            placement.geometry.1,
            output.geometry.y + output.geometry.height
                - i32::try_from(crate::winit_shell::PANEL_HEIGHT).unwrap()
        );
    }
}

#[test]
fn output_scale_change_reprojects_stationary_absolute_pointer_without_new_motion() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "absolute".into(),
            logical_width: 1200,
            logical_height: 900,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    session.last_absolute_pointer_anchor = Some(super::AbsolutePointerAnchor {
        output_name: "absolute".into(),
        normalized_x: 0.75,
        normalized_y: 0.5,
    });

    session
        .apply_output_layout(nickel_session_protocol::OutputLayout {
            primary: "absolute".into(),
            placements: vec![nickel_session_protocol::OutputPlacement {
                name: "absolute".into(),
                x: 0,
                y: 0,
                enabled: true,
                scale_120: 240,
                mode: None,
            }],
        })
        .unwrap();

    let location = session.seat.get_pointer().unwrap().current_location();
    assert_eq!(location, (450.0, 225.0).into());
    assert_eq!(
        session.output_name_at(location).as_deref(),
        Some("absolute")
    );
}

#[test]
fn applying_output_layout_preserves_independent_vertical_offsets() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    for name in ["left", "right"] {
        session
            .apply_test_output(TestOutput::Connect {
                name: name.into(),
                logical_width: 800,
                logical_height: 600,
                scale_120: 120,
                transform: OutputTransform::Normal,
            })
            .unwrap();
    }
    session
        .apply_output_layout(nickel_session_protocol::OutputLayout {
            primary: "left".into(),
            placements: vec![
                nickel_session_protocol::OutputPlacement {
                    name: "left".into(),
                    x: 0,
                    y: 0,
                    enabled: true,
                    scale_120: 120,
                    mode: None,
                },
                nickel_session_protocol::OutputPlacement {
                    name: "right".into(),
                    x: 800,
                    y: 175,
                    enabled: true,
                    scale_120: 120,
                    mode: None,
                },
            ],
        })
        .unwrap();

    let right = session
        .protocol_outputs()
        .into_iter()
        .find(|output| output.name == "right")
        .unwrap();
    assert_eq!((right.geometry.x, right.geometry.y), (800, 175));
}

#[test]
fn swapped_outputs_translate_windows_with_their_physical_output() {
    let geometry = |x| Geometry {
        x,
        y: 0,
        width: 800,
        height: 600,
    };
    let previous = HashMap::from([
        ("left".to_owned(), geometry(0)),
        ("right".to_owned(), geometry(800)),
    ]);
    let current = HashMap::from([
        ("left".to_owned(), geometry(800)),
        ("right".to_owned(), geometry(0)),
    ]);
    let window_on_left = Geometry {
        x: 120,
        y: 75,
        width: 500,
        height: 400,
    };
    let window_on_right = Geometry {
        x: 940,
        y: 90,
        width: 500,
        height: 400,
    };

    assert_eq!(
        super::output_layout_translation(window_on_left, &previous, &current),
        Some((800, 0))
    );
    assert_eq!(
        super::output_layout_translation(window_on_right, &previous, &current),
        Some((-800, 0))
    );
}

#[cfg(not(feature = "backend-udev"))]
#[test]
fn nested_backend_rejects_a_mode_it_cannot_apply_without_mutating_output() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    session
        .apply_test_output(TestOutput::Connect {
            name: "test".into(),
            logical_width: 1280,
            logical_height: 720,
            scale_120: 120,
            transform: OutputTransform::Normal,
        })
        .unwrap();
    let before = session.protocol_outputs();

    let result = session.apply_output_layout(nickel_session_protocol::OutputLayout {
        primary: "test".into(),
        placements: vec![nickel_session_protocol::OutputPlacement {
            name: "test".into(),
            x: 0,
            y: 0,
            enabled: true,
            scale_120: 120,
            mode: Some(nickel_session_protocol::OutputMode {
                width: 1024,
                height: 768,
                refresh_millihz: 60_000,
            }),
        }],
    });

    assert_eq!(
        result,
        Err("layout contains an unknown, duplicate, or invalidly scaled output")
    );
    assert_eq!(session.protocol_outputs(), before);
}

#[test]
fn fractional_scale_is_published_with_required_viewporter_protocol() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, session) = preview_test_session();

    // Retaining both global handles is the Smithay contract that keeps the
    // paired protocols advertised for the session lifetime.
    assert_ne!(
        session.fractional_scale_manager_state.global(),
        session.viewporter_state.global()
    );
}

#[test]
fn ordinary_session_exposes_a_restricted_settings_adapter_without_pid_authority() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let session = super::NickelSession::new(&mut event_loop, display, false);

    let control = session.compatibility_control.as_ref().unwrap();
    assert!(control.socket_path.exists());
    assert_eq!(control.protocol_token.len(), 64);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&control.socket_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!session.is_authenticated_shell_pid(std::process::id()));
}

#[test]
fn explicit_test_control_owns_compatibility_pid_state() {
    let (_event_loop, session) = preview_test_session();
    let control = session
        .compatibility_control
        .as_ref()
        .expect("test control should install the compatibility adapter");

    assert_ne!(control.protocol_token, "");
    assert_eq!(control.expected_shell_pid, 0);
    assert!(control.authenticated_shell_pids.is_empty());
    assert!(control.socket_path.exists());
}

#[test]
fn output_global_settles_before_disable_and_remains_until_final_grace_expires() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let output = Output::new(
        "deferred-test".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Nickel".into(),
            model: "Deferred test output".into(),
            serial_number: "deferred-test".into(),
        },
    );
    let global = output.create_global::<super::NickelSession>(&session.display_handle);
    let retained = global.clone();
    let started = Instant::now();

    session.defer_output_global_retirement_at("deferred-test".into(), global, started);
    let active = session
        .display_handle
        .backend_handle()
        .global_info(retained.clone())
        .expect("settling global remains advertised");
    assert!(!active.disabled);

    session.reap_output_global_retirements(
        started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE - Duration::from_millis(1),
    );
    assert!(
        !session
            .display_handle
            .backend_handle()
            .global_info(retained.clone())
            .unwrap()
            .disabled
    );
    session.reap_output_global_retirements(started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
    assert!(
        session
            .display_handle
            .backend_handle()
            .global_info(retained.clone())
            .expect("disabled global remains bindable during final grace")
            .disabled
    );
    assert!(
        session
            .display_handle
            .backend_handle()
            .global_info(retained.clone())
            .is_ok()
    );
    session.reap_output_global_retirements(
        started + OUTPUT_GLOBAL_BIND_SETTLE_GRACE + OUTPUT_GLOBAL_DISABLED_GRACE,
    );
    assert!(
        session
            .display_handle
            .backend_handle()
            .global_info(retained)
            .is_err()
    );
}

#[test]
fn same_name_reconnect_waits_to_publish_until_the_old_global_is_disabled() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let connect = || TestOutput::Connect {
        name: "same".into(),
        logical_width: 640,
        logical_height: 480,
        scale_120: 120,
        transform: OutputTransform::Normal,
    };

    session.apply_test_output(connect()).unwrap();
    let old = session.virtual_test_outputs["same"].1.clone().unwrap();
    session
        .apply_test_output(TestOutput::Disconnect {
            name: "same".into(),
        })
        .unwrap();
    session.apply_test_output(connect()).unwrap();

    assert!(session.virtual_test_outputs["same"].1.is_none());
    assert!(
        !session
            .display_handle
            .backend_handle()
            .global_info(old.clone())
            .unwrap()
            .disabled
    );

    session.reap_output_global_retirements(Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE);
    assert!(
        session
            .display_handle
            .backend_handle()
            .global_info(old)
            .unwrap()
            .disabled
    );
    let replacement = session.virtual_test_outputs["same"].1.clone().unwrap();
    assert!(
        !session
            .display_handle
            .backend_handle()
            .global_info(replacement)
            .unwrap()
            .disabled
    );
}

#[test]
fn rapid_same_name_reconnect_keeps_only_the_latest_live_generation_unpublished() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let connect = || TestOutput::Connect {
        name: "repeat".into(),
        logical_width: 640,
        logical_height: 480,
        scale_120: 120,
        transform: OutputTransform::Normal,
    };

    session.apply_test_output(connect()).unwrap();
    for _ in 0..64 {
        session
            .apply_test_output(TestOutput::Disconnect {
                name: "repeat".into(),
            })
            .unwrap();
        session.apply_test_output(connect()).unwrap();
        assert!(session.virtual_test_outputs["repeat"].1.is_none());
        assert_eq!(session.pending_output_global_retirements.len(), 1);
    }

    let first_disable = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
    session.reap_output_global_retirements(first_disable);
    assert!(session.virtual_test_outputs["repeat"].1.is_some());
    assert_eq!(session.pending_output_global_retirements.len(), 1);

    session
        .apply_test_output(TestOutput::Disconnect {
            name: "repeat".into(),
        })
        .unwrap();
    session.apply_test_output(connect()).unwrap();
    assert!(session.virtual_test_outputs["repeat"].1.is_none());
    assert_eq!(session.pending_output_global_retirements.len(), 2);

    let second_disable = first_disable + OUTPUT_GLOBAL_DISABLED_GRACE;
    session.reap_output_global_retirements(second_disable);
    assert!(session.virtual_test_outputs["repeat"].1.is_some());
    assert_eq!(session.pending_output_global_retirements.len(), 1);
    session.reap_output_global_retirements(second_disable + OUTPUT_GLOBAL_DISABLED_GRACE);
    assert_eq!(session.pending_output_global_retirements.len(), 0);
}

#[test]
fn shutdown_drops_pending_and_unpublished_same_name_generations() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (event_loop, mut session) = preview_test_session();
    let connect = || TestOutput::Connect {
        name: "shutdown".into(),
        logical_width: 640,
        logical_height: 480,
        scale_120: 120,
        transform: OutputTransform::Normal,
    };
    session.apply_test_output(connect()).unwrap();
    session
        .apply_test_output(TestOutput::Disconnect {
            name: "shutdown".into(),
        })
        .unwrap();
    session.apply_test_output(connect()).unwrap();
    assert_eq!(session.pending_output_global_retirements.len(), 1);
    assert!(session.virtual_test_outputs["shutdown"].1.is_none());
    drop(session);
    drop(event_loop);
}

#[test]
fn rapid_virtual_output_churn_applies_backpressure_until_reap() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let connect = |name: String| TestOutput::Connect {
        name,
        logical_width: 640,
        logical_height: 480,
        scale_120: 120,
        transform: OutputTransform::Normal,
    };

    for generation in 0..MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS {
        let name = format!("rapid-{generation}");
        session
            .apply_test_output(connect(name.clone()))
            .expect("churn within the deferred-global bound is admitted");
        session
            .apply_test_output(TestOutput::Disconnect { name })
            .expect("admitted output disconnects into the grace queue");
    }
    assert_eq!(
        session.pending_output_global_retirements.len(),
        MAX_PENDING_OUTPUT_GLOBAL_RETIREMENTS
    );
    assert_eq!(
        session.apply_test_output(connect("backpressured".into())),
        Err("output global retirement backlog is full")
    );

    let disable_at = Instant::now() + OUTPUT_GLOBAL_BIND_SETTLE_GRACE;
    session.reap_output_global_retirements(disable_at);
    assert_eq!(
        session.apply_test_output(connect("still-backpressured".into())),
        Err("output global retirement backlog is full")
    );
    session.reap_output_global_retirements(disable_at + OUTPUT_GLOBAL_DISABLED_GRACE);
    session
        .apply_test_output(connect("after-reap".into()))
        .expect("global admission resumes after the grace queue drains");
}

#[test]
fn failed_capture_rolls_the_real_frame_and_allocation_back_into_session() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.preview_admitted.insert(id);
    session.preview_frames.insert(
        id,
        super::PreviewFrame {
            width: 75,
            height: super::PREVIEW_HEIGHT as u16,
            rgba: vec![41; 75 * super::PREVIEW_HEIGHT * 4],
        },
    );
    let allocation = session.preview_frames[&id].rgba.as_ptr();

    let (rgba, had_frame) = session.take_preview_capture_buffer(id);
    session.preview_capture_failed(id, rgba, had_frame);

    assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
    assert_eq!(session.preview_frames[&id].rgba[0], 41);
    assert_eq!(session.preview_frames[&id].width, 75);
    assert_eq!(session.preview_frames[&id].height, 135);
    assert_eq!(session.preview_counters.evictions, 0);
    assert_eq!(session.preview_counters.capture_failures, 1);
}

#[test]
fn preview_source_churn_and_failed_capture_preserve_presentation_generation() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![id]);
    session.store_preview(
        id,
        super::PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![41; 4],
        },
    );
    let presented = session.preview_counters.presentation_generation;
    let allocation = session.preview_frames[&id].rgba.as_ptr();

    // Exercise the same content invalidation used by surface commits. Neither
    // repeated commits nor a failed replacement changes the retained pixels.
    for _ in 0..1000 {
        session.invalidate_preview_content(id);
    }
    assert!(session.preview_dirty.contains(&id));
    assert_eq!(session.preview_counters.invalidations, 1000);
    assert_eq!(session.preview_counters.presentation_generation, presented);
    let (pixels, dimensions) = session.take_preview_capture_buffer(id);
    session.preview_capture_failed(id, pixels, dimensions);
    assert_eq!(session.preview_counters.presentation_generation, presented);
    assert_eq!(session.preview_frames[&id].rgba.as_ptr(), allocation);
    assert_eq!(session.preview_frames[&id].rgba, vec![41; 4]);

    session.store_preview(
        id,
        super::PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![42; 4],
        },
    );
    assert_eq!(
        session.preview_counters.presentation_generation,
        presented + 1
    );
    session.reassociate_preview_surface(id);
    assert!(!session.preview_frames.contains_key(&id));
    assert_eq!(
        session.preview_counters.presentation_generation,
        presented + 2
    );
    session.reassociate_preview_surface(id);
    assert_eq!(
        session.preview_counters.presentation_generation,
        presented + 2
    );
}

#[test]
fn retiring_preview_scratch_does_not_invalidate_presented_pixels() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![id]);
    let (pixels, dimensions) = session.take_preview_capture_buffer(id);
    session.preview_capture_failed(id, pixels, dimensions);
    session.clear_switcher_preview_interest();
    assert_eq!(session.preview_counters.presentation_generation, 0);
    assert_eq!(session.preview_bytes(), 0);
    assert_eq!(session.preview_counters.evictions, 1);

    session.set_switcher_preview_interest(vec![id]);
    session.store_preview(
        id,
        super::PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![41; 4],
        },
    );
    session.clear_switcher_preview_interest();
    assert_eq!(session.preview_counters.presentation_generation, 2);
    assert_eq!(session.preview_bytes(), 0);

    session.set_switcher_preview_interest(vec![id]);
    session.store_preview(
        id,
        super::PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![42; 4],
        },
    );
    session.clear_all_previews();
    assert_eq!(session.preview_counters.presentation_generation, 4);
    session.clear_all_previews();
    assert_eq!(session.preview_counters.presentation_generation, 4);
}

#[test]
fn fitted_preview_frame_is_stored_at_its_actual_dimensions() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.preview_admitted.insert(id);
    let width = 210;
    let height = super::PREVIEW_HEIGHT as u16;

    session.store_preview(
        id,
        super::PreviewFrame {
            width,
            height,
            rgba: vec![17; usize::from(width) * usize::from(height) * 4],
        },
    );

    let stored = &session.preview_frames[&id];
    assert_eq!((stored.width, stored.height), (width, height));
    assert_eq!(stored.rgba.len(), 113_400);
    assert_eq!(session.preview_counters.captures, 1);
}

#[cfg(feature = "backend-udev")]
#[test]
fn unavailable_preview_renderer_exhausts_shared_retry_budget_without_retiring_pixels() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let ids = (0..3)
        .map(|_| {
            session
                .windows
                .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                .unwrap()
        })
        .collect::<Vec<_>>();
    session.preview_admitted.extend(ids.iter().copied());
    for id in &ids[1..] {
        session.store_preview(
            *id,
            super::PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![41; 4],
            },
        );
    }
    session.preview_dirty.insert(ids[1]);
    let old_pixels = session.preview_frames[&ids[1]].rgba.as_ptr();
    let generation = session.preview_counters.presentation_generation;
    let started = Instant::now();
    for attempt in 0..5 {
        let now = started + Duration::from_secs(attempt);
        session.ready_preview_retries(now);
        session.preview_renderer_unavailable(now);
        assert_eq!(session.preview_counters.capture_failures, (attempt + 1) * 2);
        // Output/frame activity during cooldown must not charge more failures
        // or turn renderer lookup failure into an unbounded retry loop.
        for _ in 0..100 {
            session.preview_renderer_unavailable(now);
        }
        assert_eq!(session.preview_counters.capture_failures, (attempt + 1) * 2);
    }
    session.preview_renderer_unavailable(started + Duration::from_secs(60));
    assert_eq!(session.preview_counters.capture_failures, 10);
    assert!(session.preview_retry_pending.is_empty());
    assert!(!session.preview_capture_work_pending());
    assert_eq!(session.preview_frames[&ids[1]].rgba.as_ptr(), old_pixels);
    assert_eq!(session.preview_counters.presentation_generation, generation);
    assert!(!session.preview_failures.contains_key(&ids[2]));
}

#[test]
fn fourteen_first_capture_failures_retain_exactly_the_declared_capacity() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let ids = (0..PREVIEW_ENTRY_CAPACITY)
        .map(|_| {
            session
                .windows
                .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                .unwrap()
        })
        .collect::<Vec<_>>();
    session.preview_admitted.extend(ids.iter().copied());
    for id in ids {
        let (rgba, had_frame) = session.take_preview_capture_buffer(id);
        assert!(had_frame.is_none());
        session.preview_capture_failed(id, rgba, None);
    }

    assert_eq!(session.preview_bytes(), PREVIEW_BYTE_CAPACITY);
    assert_eq!(
        session.preview_counters.peak_bytes,
        PREVIEW_BYTE_CAPACITY as u64
    );
    assert_eq!(session.preview_spares.len(), PREVIEW_ENTRY_CAPACITY);
}

#[test]
fn invalid_mapped_length_leaves_the_capture_lease_untouched() {
    let pixels = vec![23; PREVIEW_FRAME_BYTES];
    let allocation = pixels.as_ptr();
    assert!(!preview_mapping_has_exact_size(
        &vec![0; PREVIEW_FRAME_BYTES - 1],
        super::PREVIEW_WIDTH as u16,
        super::PREVIEW_HEIGHT as u16,
    ));
    assert_eq!(pixels.as_ptr(), allocation);
    assert_eq!(pixels[0], 23);
}

#[test]
fn preview_capture_dimensions_are_bounded_and_preserve_window_aspect() {
    for (source, expected) in [
        ((3440, 1440), (240, 100)),
        ((1920, 1080), (240, 135)),
        ((1600, 1200), (180, 135)),
        ((1000, 1000), (135, 135)),
        ((900, 1600), (75, 135)),
        ((1919, 1079), (240, 134)),
    ] {
        let fitted = super::preview_capture_dimensions(source.0, source.1)
            .unwrap_or_else(|| panic!("positive source {source:?} has capture dimensions"));
        assert_eq!(fitted, expected, "source {source:?}");
        assert!(usize::from(fitted.0) <= super::PREVIEW_WIDTH);
        assert!(usize::from(fitted.1) <= super::PREVIEW_HEIGHT);
        if usize::from(fitted.0) == super::PREVIEW_WIDTH {
            let ideal_height = source.1 as f64 * f64::from(fitted.0) / source.0 as f64;
            assert!((ideal_height - f64::from(fitted.1)).abs() <= 1.0);
        } else {
            assert_eq!(usize::from(fitted.1), super::PREVIEW_HEIGHT);
            let ideal_width = source.0 as f64 * f64::from(fitted.1) / source.1 as f64;
            assert!((ideal_width - f64::from(fitted.0)).abs() <= 1.0);
        }
    }
    assert_eq!(super::preview_capture_dimensions(0, 1080), None);
    assert_eq!(super::preview_capture_dimensions(1920, -1), None);
}

#[test]
fn stale_retry_epoch_cannot_consume_new_generation_pending_work() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let old = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![old]);
    session.record_preview_failure(old, std::time::Instant::now());
    session.schedule_preview_retry();
    let stale_epoch = session.preview_retry_scheduled.unwrap().0;

    session.clear_all_previews();
    let current = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![current]);
    session.preview_content_generation.insert(current, 10);
    session.record_preview_failure(current, std::time::Instant::now());
    session.schedule_preview_retry();
    assert_ne!(session.preview_retry_scheduled.unwrap().0, stale_epoch);

    event_loop
        .dispatch(std::time::Duration::from_millis(150), &mut session)
        .unwrap();
    assert_eq!(session.preview_content_generation[&current], 10);
    assert!(session.preview_retry_pending.is_empty());
}

#[test]
fn nested_retry_waits_for_capture_cadence_and_fires_only_once() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (mut event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![id]);
    session.preview_content_generation.insert(id, 4);
    session.record_preview_failure(id, std::time::Instant::now());
    session.schedule_preview_retry_after(std::time::Duration::from_millis(200));

    event_loop
        .dispatch(std::time::Duration::from_millis(30), &mut session)
        .unwrap();
    assert_eq!(session.preview_content_generation[&id], 4);
    assert_eq!(session.preview_retry_pending.len(), 1);
    assert!(session.preview_retry_scheduled.is_some());

    event_loop
        .dispatch(std::time::Duration::from_millis(220), &mut session)
        .unwrap();
    assert_eq!(session.preview_content_generation[&id], 4);
    assert!(session.preview_retry_pending.is_empty());
    assert!(session.preview_retry_scheduled.is_none());

    event_loop
        .dispatch(std::time::Duration::from_millis(30), &mut session)
        .unwrap();
    assert_eq!(session.preview_content_generation[&id], 4);
    assert!(session.preview_retry_scheduled.is_none());
}

#[test]
fn preview_failure_backoff_survives_source_churn_and_stops_after_five_attempts() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.set_switcher_preview_interest(vec![id]);
    let mut now = std::time::Instant::now();
    for delay_ms in [100, 200, 400, 800] {
        assert!(session.preview_retry_ready(id, now));
        session.record_preview_failure(id, now);
        // Video commits coalesce source content but cannot bypass cooldown.
        for _ in 0..1000 {
            session.invalidate_preview_content(id);
        }
        let before = now + std::time::Duration::from_millis(delay_ms - 1);
        assert!(!session.preview_retry_ready(id, before));
        assert!(!session.ready_preview_retries(before));
        now += std::time::Duration::from_millis(delay_ms);
        assert!(session.preview_retry_ready(id, now));
        assert!(session.ready_preview_retries(now));
        assert!(!session.ready_preview_retries(now));
    }
    session.record_preview_failure(id, now);
    let later = now + std::time::Duration::from_secs(3600);
    session.invalidate_preview_content(id);
    assert!(!session.preview_retry_ready(id, later));
    assert!(!session.ready_preview_retries(later));
    assert!(session.preview_retry_pending.is_empty());
    session.schedule_preview_retry();
    assert!(session.preview_retry_scheduled.is_none());
    assert_eq!(session.preview_failures.len(), 1);

    session.reassociate_preview_surface(id);
    assert!(session.preview_retry_ready(id, later));
    session.record_preview_failure(id, later);
    session.clear_switcher_preview_interest();
    assert!(session.preview_failures.is_empty());
    session.set_switcher_preview_interest(vec![id]);
    assert!(session.preview_retry_ready(id, later));
}

#[test]
fn preview_retry_drains_only_due_windows_and_success_retires_failure_state() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let ids = (0..2)
        .map(|_| {
            session
                .windows
                .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                .unwrap()
        })
        .collect::<Vec<_>>();
    session.set_switcher_preview_interest(ids.clone());
    let now = std::time::Instant::now();
    session.record_preview_failure(ids[0], now);
    session.record_preview_failure(ids[1], now + std::time::Duration::from_millis(50));
    assert!(session.ready_preview_retries(now + std::time::Duration::from_millis(100)));
    assert!(!session.preview_retry_pending.contains(&ids[0]));
    assert!(session.preview_retry_pending.contains(&ids[1]));
    session.store_preview(
        ids[1],
        super::PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![41; 4],
        },
    );
    assert!(!session.preview_failures.contains_key(&ids[1]));
    assert!(session.preview_retry_pending.is_empty());
    session.clear_all_previews();
    assert!(session.preview_failures.is_empty());
}

#[test]
fn real_session_reconcile_preserves_seven_frames_for_each_visible_consumer() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let switcher = (0..7)
        .map(|_| {
            session
                .windows
                .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let overlay = (0..7)
        .map(|_| {
            session
                .windows
                .insert(crate::session::window_registry::WindowAdmission::Ordinary)
                .unwrap()
        })
        .collect::<Vec<_>>();
    session.set_switcher_preview_interest(switcher.clone());
    session.set_overlay_preview_interest(overlay.clone());
    for id in switcher.iter().chain(&overlay).copied() {
        session.preview_frames.insert(
            id,
            super::PreviewFrame {
                width: super::PREVIEW_WIDTH as u16,
                height: super::PREVIEW_HEIGHT as u16,
                rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
            },
        );
    }

    session.clear_switcher_preview_interest();

    assert!(
        overlay
            .iter()
            .all(|id| session.preview_frames.contains_key(id))
    );
    assert_eq!(session.preview_bytes(), 7 * PREVIEW_FRAME_BYTES);
}

#[test]
fn real_preview_query_and_encode_counters_partition_the_aggregate() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.preview_admitted.insert(id);
    session.preview_frames.insert(
        id,
        super::PreviewFrame {
            width: super::PREVIEW_WIDTH as u16,
            height: super::PREVIEW_HEIGHT as u16,
            rgba: vec![5; PREVIEW_FRAME_BYTES],
        },
    );
    let message = session.handle_protocol_query(Query::Preview {
        window: nickel_session_protocol::WindowId(id.0),
    });
    assert!(matches!(message, ServerMessage::Preview(_)));
    let framed = nickel_session_protocol::encode(&ServerEnvelope {
        request_id: 9,
        message,
    })
    .unwrap();
    session.record_preview_protocol_encoding(
        framed.len() - nickel_session_protocol::FRAME_HEADER_BYTES,
        framed.len(),
    );

    let counters = session.preview_counters;
    assert_eq!(
        counters.protocol_copy_bytes,
        counters.protocol_raw_copy_bytes
            + counters.protocol_base64_bytes
            + counters.protocol_json_payload_bytes
            + counters.protocol_framed_copy_bytes
    );
    assert_eq!(counters.protocol_raw_copy_bytes, PREVIEW_FRAME_BYTES as u64);
}

#[test]
fn real_session_attempt_generation_blocks_other_nodes_until_explicit_retry() {
    let _guard = PREVIEW_SESSION_TEST_LOCK.lock().unwrap();
    let (_event_loop, mut session) = preview_test_session();
    let id = session
        .windows
        .insert(crate::session::window_registry::WindowAdmission::Ordinary)
        .unwrap();
    session.preview_content_generation.insert(id, 6);
    let first_node_wave = session.begin_preview_render_wave();
    assert!(record_preview_capture_attempt(
        &mut session.preview_attempted,
        id,
        6,
        first_node_wave
    ));
    let second_node_wave = session.begin_preview_render_wave();
    assert!(!record_preview_capture_attempt(
        &mut session.preview_attempted,
        id,
        6,
        second_node_wave
    ));

    session.preview_admitted.insert(id);
    session.preview_renderer_failed(id);
    assert!(!session.ready_preview_retries(std::time::Instant::now()));
    assert!(
        session.ready_preview_retries(
            std::time::Instant::now() + std::time::Duration::from_millis(100)
        )
    );
    let retry_wave = session.begin_preview_render_wave();
    assert!(record_preview_capture_attempt(
        &mut session.preview_attempted,
        id,
        6,
        retry_wave
    ));
}

#[test]
fn preview_workload_is_bounded_around_the_selected_window() {
    let ids = (0..nickel_session_protocol::MAX_WINDOWS as u64)
        .map(super::WindowId)
        .collect::<Vec<_>>();
    let selected = nickel_session_protocol::MAX_WINDOWS / 2;
    let admitted = bounded_preview_ids(ids, selected);

    assert_eq!(admitted.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
    assert!(admitted.contains(&super::WindowId(selected as u64)));
    assert_eq!(
        PREVIEW_BYTE_CAPACITY,
        PREVIEW_ENTRY_CAPACITY * PREVIEW_FRAME_BYTES
    );
    assert_eq!(PREVIEW_BYTE_CAPACITY, 1_814_400);
}

#[test]
fn preview_workload_clamps_at_both_candidate_edges() {
    let ids = (0..12).map(super::WindowId).collect::<Vec<_>>();
    assert_eq!(bounded_preview_ids(ids.clone(), 0), ids[..7]);
    assert_eq!(bounded_preview_ids(ids.clone(), 11), ids[5..]);
}

#[test]
fn independent_preview_consumers_share_the_budget_without_destroying_interest() {
    let switcher = (1..=7).map(super::WindowId).collect::<Vec<_>>();
    let overlay = (8..=(nickel_session_protocol::MAX_WINDOWS as u64 + 8))
        .map(super::WindowId)
        .collect::<Vec<_>>();

    let overlapping = admitted_preview_ids(&switcher, &overlay);
    assert_eq!(overlapping.len(), PREVIEW_ENTRY_CAPACITY);
    assert!(switcher.iter().all(|id| overlapping.contains(id)));
    let mut frames = overlapping
        .iter()
        .map(|id| {
            (
                *id,
                super::PreviewFrame {
                    width: super::PREVIEW_WIDTH as u16,
                    height: super::PREVIEW_HEIGHT as u16,
                    rgba: vec![id.0 as u8; PREVIEW_FRAME_BYTES],
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let after_overlay_dismissal = admitted_preview_ids(&switcher, &[]);
    assert_eq!(after_overlay_dismissal, switcher.iter().copied().collect());
    let after_switcher_dismissal = admitted_preview_ids(&[], &overlay);
    assert_eq!(after_switcher_dismissal.len(), PREVIEW_ENTRY_CAPACITY);
    frames.retain(|id, _| after_switcher_dismissal.contains(id));
    assert_eq!(frames.len(), PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER);
    assert!(
        overlay[..PREVIEW_ENTRIES_PER_VISIBLE_CONSUMER]
            .iter()
            .all(|id| protocol_preview_from_cached(
                nickel_session_protocol::WindowId(id.0),
                frames.get(id)
            )
            .is_some())
    );
}

#[test]
fn failed_capture_is_attempted_once_per_generation_across_outputs_and_nodes() {
    let id = super::WindowId(7);
    let mut attempted = HashMap::new();
    assert!(record_preview_capture_attempt(&mut attempted, id, 3, 11));
    for _ in 0..3 {
        assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 11));
    }
    assert!(!record_preview_capture_attempt(&mut attempted, id, 3, 12));
    assert!(record_preview_capture_attempt(&mut attempted, id, 4, 12));
}

#[test]
fn preview_churn_never_admits_more_than_the_declared_process_ceiling() {
    for generation in 0..(nickel_session_protocol::MAX_WINDOWS as u64 * 2) {
        let switcher = (generation..generation + 7)
            .map(super::WindowId)
            .collect::<Vec<_>>();
        let overlay = (generation + 7..generation + 1_031)
            .map(super::WindowId)
            .collect::<Vec<_>>();
        let admitted = admitted_preview_ids(&switcher, &overlay);
        assert!(admitted.len() <= PREVIEW_ENTRY_CAPACITY);
        assert!(admitted.len() * PREVIEW_FRAME_BYTES <= PREVIEW_BYTE_CAPACITY);
    }
}

#[test]
fn damage_advances_only_the_authoritative_window_content_generation() {
    let damaged = super::WindowId(1);
    let unchanged = super::WindowId(2);
    let mut generations = HashMap::from([(damaged, 3), (unchanged, 8)]);
    let mut attempted = HashMap::from([(damaged, (3, 4)), (unchanged, (8, 4))]);

    assert_eq!(
        advance_preview_content_generation(&mut generations, &mut attempted, damaged),
        4
    );
    assert_eq!(generations[&unchanged], 8);
    assert!(!attempted.contains_key(&damaged));
    assert_eq!(attempted[&unchanged], (8, 4));
}

#[test]
fn not_ready_query_has_no_interest_side_effect() {
    let switcher = vec![super::WindowId(1)];
    let overlay = vec![super::WindowId(2)];
    let before = admitted_preview_ids(&switcher, &overlay);
    assert!(protocol_preview_from_cached(nickel_session_protocol::WindowId(99), None).is_none());
    assert_eq!(admitted_preview_ids(&switcher, &overlay), before);
}

#[test]
fn replacement_reuses_the_retired_frame_allocation() {
    let pixels = vec![7; PREVIEW_FRAME_BYTES];
    let allocation = pixels.as_ptr();
    let replacement = reuse_preview_pixels(pixels, &vec![9; PREVIEW_FRAME_BYTES]);
    assert_eq!(replacement.as_ptr(), allocation);
    assert_eq!(replacement.len(), PREVIEW_FRAME_BYTES);
    assert!(replacement.iter().all(|pixel| *pixel == 9));
}

#[test]
fn only_the_exact_supervised_shell_pid_can_register() {
    let current = std::process::id();
    assert_eq!(
        shell_registration_rejection(current, current, current + 1, true),
        Some(ShellRegistrationRejection::ClaimedPeerMismatch)
    );
    assert_eq!(
        shell_registration_rejection(0, current, current, true),
        Some(ShellRegistrationRejection::NoActiveGeneration)
    );
    assert_eq!(
        shell_registration_rejection(current + 1, current, current, true),
        Some(ShellRegistrationRejection::OutsideActiveGeneration)
    );
    assert_eq!(
        shell_registration_rejection(current, current, current, false),
        Some(ShellRegistrationRejection::OutsideSessionUser)
    );
    assert_eq!(
        shell_registration_rejection(current, current, current, true),
        None
    );
}

#[test]
fn privileged_shell_commands_require_the_registered_shell_pid() {
    for command in [
        Command::LogOut,
        Command::ObservePendingLaunch {
            generation: 7,
            root_pid: 42,
            root_start_time: 99,
            deadline_ms: 100,
        },
        Command::CancelPendingLaunch { generation: 7 },
        Command::Unlock,
        Command::SessionAction {
            action: SessionAction::Lock,
        },
        Command::SessionAction {
            action: SessionAction::PowerOff,
        },
        Command::FocusShellRole {
            role: ShellRole::ControlCenter,
        },
        Command::RestoreApplicationFocus,
    ] {
        assert!(command_requires_shell_identity(&command));
    }
    assert!(!command_requires_shell_identity(&Command::ToggleLauncher));
}

#[test]
fn process_lineage_accepts_self_and_a_live_descendant_only() {
    let start = super::linux_process_start_time(std::process::id()).unwrap();
    assert!(super::process_descends_from(
        std::process::id(),
        std::process::id(),
        start,
    ));
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "sleep 2"])
        .spawn()
        .expect("spawn lineage fixture");
    assert!(super::process_descends_from(
        child.id(),
        std::process::id(),
        start,
    ));
    assert!(!super::process_descends_from(
        child.id(),
        std::process::id(),
        start.wrapping_add(1),
    ));
    assert!(!super::process_descends_from(
        std::process::id(),
        child.id(),
        super::linux_process_start_time(child.id()).unwrap(),
    ));
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn ordinary_shell_focus_includes_the_interactive_screenshot_overlay() {
    for role in [
        ShellRole::ControlCenter,
        ShellRole::ProjectMenu,
        ShellRole::Preview,
        ShellRole::ContextMenu,
        ShellRole::Screenshot,
    ] {
        assert!(shell_role_accepts_ordinary_focus(role));
    }
    for role in [
        ShellRole::Desktop,
        ShellRole::Panel,
        ShellRole::Launcher,
        ShellRole::Lock,
        ShellRole::Notification,
    ] {
        assert!(!shell_role_accepts_ordinary_focus(role));
    }
}

#[test]
fn explicit_nested_test_control_can_cross_lock_and_logout_boundaries() {
    assert!(test_control_may_invoke(&Command::LogOut));
    assert!(test_control_may_invoke(&Command::Unlock));
    assert!(test_control_may_invoke(&Command::SessionAction {
        action: SessionAction::Lock,
    }));
    assert!(!test_control_may_invoke(&Command::SessionAction {
        action: SessionAction::PowerOff,
    }));
}

#[test]
fn surface_identity_registration_requires_the_authenticated_shell() {
    assert!(command_requires_shell_identity(
        &Command::RegisterShellSurface {
            identity: nickel_session_protocol::ShellSurfaceIdentity {
                application_id: "io.nickel.shell.surface.42.1".into(),
                role: ShellRole::Desktop,
                output: Some("DP-1".into()),
            },
        }
    ));
}

#[test]
fn disconnected_surfaces_stop_inhibiting_idle_policy() {
    let mut inhibitors = HashMap::from([("alive", 2), ("disconnected", 1)]);
    retain_live_idle_inhibitors(&mut inhibitors, |surface| *surface == "alive");
    assert_eq!(inhibitors, HashMap::from([("alive", 2)]));
}

#[test]
fn descriptive_output_names_resolve_to_authoritative_connector_names() {
    let outputs = vec!["DVI-I-1".into(), "DP-3".into()];

    assert_eq!(
        output_index_for_shell_surface("Unknown - Odyssey G40B - DP-3", &outputs),
        Some(1)
    );
    assert_eq!(
        output_index_for_shell_surface("Unknown - MB16A - DVI-I-1", &outputs),
        Some(0)
    );
}

#[test]
fn descriptive_output_matching_rejects_missing_or_ambiguous_connectors() {
    assert_eq!(
        output_index_for_shell_surface("Unknown - DisplayPort-1", &["DP-1".into()]),
        None
    );
    assert_eq!(
        output_index_for_shell_surface(
            "Unknown - Display - DP-1",
            &["DP-1".into(), "Display - DP-1".into()]
        ),
        None
    );
}

#[test]
fn output_rescue_clamps_windows_to_the_authoritative_work_area() {
    let work_area = Geometry {
        x: 100,
        y: 40,
        width: 800,
        height: 500,
    };
    assert_eq!(
        clamp_window_location((850, 500).into(), (300, 200).into(), work_area),
        (600, 340).into()
    );
    assert_eq!(
        clamp_window_location((-20, -30).into(), (300, 200).into(), work_area),
        (100, 40).into()
    );
}

#[test]
fn snap_restore_clamps_complete_rect_without_reusing_snapped_size() {
    let restore = Geometry {
        x: 850,
        y: 500,
        width: 300,
        height: 200,
    };
    let work_area = Geometry {
        x: 100,
        y: 40,
        width: 800,
        height: 500,
    };

    let desired = clamped_restore_geometry(restore, work_area, false);

    assert_eq!((desired.x, desired.y), (600, 340));
    assert_eq!((desired.width, desired.height), (300, 200));
}

#[test]
fn output_reconnect_does_not_restore_over_a_newer_desired_placement() {
    use nickel_core::geometry_authority::{GeometryAuthority, GeometryConstraints, Presentation};

    let rescued = Geometry {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    let mut authority = GeometryAuthority::new(rescued, Presentation::Normal);
    let rescue_revision = authority.revisions().placement;
    assert!(output_rescue_revision_is_current(
        &authority,
        rescue_revision
    ));

    authority.set_placement(
        Geometry { x: 400, ..rescued },
        GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    assert!(!output_rescue_revision_is_current(
        &authority,
        rescue_revision
    ));
}

#[test]
fn snap_restore_cannot_overwrite_a_newer_geometry_revision() {
    use nickel_core::geometry_authority::{GeometryAuthority, GeometryConstraints, Presentation};

    let original = Geometry {
        x: 40,
        y: 30,
        width: 800,
        height: 600,
    };
    let mut authority = GeometryAuthority::new(original, Presentation::Normal);
    let restore = super::RevisionedPlacementRestore {
        geometry: smithay::utils::Rectangle::new(
            (original.x, original.y).into(),
            (original.width, original.height).into(),
        ),
        last_owned_revision: authority.revisions().placement,
    };
    assert!(placement_restore_is_current(&authority, restore));

    authority.set_placement(
        Geometry { x: 900, ..original },
        GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    assert!(!placement_restore_is_current(&authority, restore));
}

#[test]
fn x11_request_binds_existing_snap_revision_without_reauthorizing() {
    use nickel_core::geometry_authority::{GeometryAuthority, GeometryConstraints, Presentation};

    let desired = Geometry {
        x: 0,
        y: 0,
        width: 960,
        height: 1080,
    };
    let mut authority = GeometryAuthority::new(desired, Presentation::Normal);
    let authorized = authority.authorize_placement(
        desired,
        GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    let before = authority.revisions();

    assert_eq!(
        super::revisions_for_authorized_x11_request(&authority, desired, authorized.revision,),
        Some(before)
    );
    assert_eq!(authority.revisions(), before);

    authority.authorize_placement(
        desired,
        GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    assert_eq!(
        super::revisions_for_authorized_x11_request(&authority, desired, authorized.revision,),
        None
    );
}

#[test]
fn internal_maximize_restore_cannot_overwrite_newer_placement() {
    use nickel_core::geometry_authority::{GeometryAuthority, GeometryConstraints, Presentation};

    let initial = Geometry {
        x: 20,
        y: 30,
        width: 700,
        height: 500,
    };
    let mut authority = GeometryAuthority::new(initial, Presentation::Maximized);
    let restore = super::RevisionedInternalRestore {
        placement: crate::session::InternalSurfacePlacement {
            role: crate::session::internal_ui::InternalSurfaceRole::Application,
            geometry: (20, 30, 700, 500),
            output: Some("DP-1".into()),
        },
        last_owned_revision: authority.revisions().placement,
    };
    assert!(internal_restore_is_current(&authority, &restore));
    authority.set_placement(
        Geometry { x: 900, ..initial },
        GeometryConstraints {
            min_width: 1,
            min_height: 1,
            max_width: None,
            max_height: None,
        },
    );
    assert!(!internal_restore_is_current(&authority, &restore));
}

#[test]
fn maximized_server_frame_exactly_fits_the_work_area() {
    let work_area = Geometry {
        x: -1920,
        y: 30,
        width: 1920,
        height: 1010,
    };

    let content = maximized_content_geometry(work_area, true);

    assert_eq!(
        crate::session::window_frame::outer_geometry(content),
        work_area
    );
}

#[test]
fn initial_managed_x11_content_keeps_its_frame_inside_the_work_area() {
    let work_area = Geometry {
        x: 0,
        y: 0,
        width: 1920,
        height: 1024,
    };
    let content = clamp_decorated_content_to_work_area(
        Geometry {
            x: 0,
            y: 0,
            width: 1200,
            height: 800,
        },
        work_area,
    );

    let outer = crate::session::window_frame::outer_geometry(content);
    assert_eq!(outer.x, work_area.x);
    assert_eq!(outer.y, work_area.y);
    assert_eq!(content.width, 1200);
    assert_eq!(content.height, 800);
}

#[test]
fn maximized_client_decorated_window_receives_the_whole_work_area() {
    let work_area = Geometry {
        x: 1920,
        y: -200,
        width: 1280,
        height: 700,
    };

    assert_eq!(maximized_content_geometry(work_area, false), work_area);
}

#[test]
fn maximized_content_geometry_clamps_undersized_work_areas() {
    let work_area = Geometry {
        x: 7,
        y: 11,
        width: 1,
        height: 1,
    };

    let content = maximized_content_geometry(work_area, true);

    assert_eq!(content.width, 1);
    assert_eq!(content.height, 1);
}

#[test]
fn restored_drag_preserves_horizontal_pointer_proportion() {
    let current = maximized_content_geometry(
        Geometry {
            x: 0,
            y: 0,
            width: 1200,
            height: 700,
        },
        true,
    );
    let restore = Geometry {
        x: 80,
        y: 90,
        width: 600,
        height: 400,
    };
    let work_area = Geometry {
        x: 0,
        y: 0,
        width: 1200,
        height: 700,
    };

    let left = restored_drag_content_geometry(
        current,
        restore,
        Point::from((120.0, 18.0)),
        true,
        work_area,
    );
    let center = restored_drag_content_geometry(
        current,
        restore,
        Point::from((600.0, 18.0)),
        true,
        work_area,
    );
    let right = restored_drag_content_geometry(
        current,
        restore,
        Point::from((1080.0, 18.0)),
        true,
        work_area,
    );

    assert!(left.x < center.x);
    assert!(center.x < right.x);
    assert_eq!(left.width, restore.width);
    assert_eq!(center.height, restore.height);
}

#[test]
fn restored_drag_keeps_titlebar_reachable_on_negative_output() {
    let work_area = Geometry {
        x: -1920,
        y: -200,
        width: 1920,
        height: 1000,
    };
    let geometry = restored_drag_content_geometry(
        maximized_content_geometry(work_area, true),
        Geometry {
            x: 10,
            y: 10,
            width: 900,
            height: 700,
        },
        Point::from((-1910.0, -195.0)),
        true,
        work_area,
    );
    let outer = crate::session::window_frame::outer_geometry(geometry);

    assert!(outer.x + outer.width >= work_area.x + 32);
    assert!(outer.y >= work_area.y);
    assert!(outer.y < work_area.y + work_area.height);
}

#[test]
fn drag_icon_uses_pointer_output_and_output_local_coordinates() {
    let left = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());
    let right = smithay::utils::Rectangle::new((1920, 0).into(), (1920, 1080).into());
    let pointer = smithay::utils::Point::from((2012.4, 84.6));

    assert_eq!(drag_icon_location(pointer, left), None);
    assert_eq!(drag_icon_location(pointer, right), Some((92, 85).into()));
}

#[test]
fn transient_output_hit_testing_uses_both_global_axes() {
    let upper = smithay::utils::Rectangle::new((0, -1080).into(), (1920, 1080).into());
    let lower = smithay::utils::Rectangle::new((0, 0).into(), (1920, 1080).into());

    assert!(output_contains_logical_point(upper, 960, -40));
    assert!(!output_contains_logical_point(lower, 960, -40));
    assert!(!output_contains_logical_point(upper, 960, 1040));
    assert!(output_contains_logical_point(lower, 960, 1040));
}
