//! Opt-in native ordinary-client acceptance using existing repository examples.
use super::*;

pub(super) fn movement_enabled() -> bool {
    env::args().any(|argument| argument == "--movement")
}

fn xwayland_enabled() -> bool {
    env::args().any(|argument| argument == "--xwayland-ordinary-scopes")
}

fn hold_receipts_enabled() -> bool {
    movement_enabled() || xwayland_enabled()
}

const COUNTER_CATALOG_ID: &str = "nickel-native-movement-counter";
const MOVEMENT_OUTPUT: &str = "native-movement-secondary";

pub(super) fn prepare_movement_catalog(runtime: &Path, binaries: &Path) -> Result<(), String> {
    let executable = binaries.join("examples/standalone");
    if !executable.is_file() {
        return Err("movement acceptance requires the standalone repository example".into());
    }
    let executable = executable.to_str().ok_or("fixture path is not UTF-8")?;
    // This test-created desktop entry points to a real executable. It supplies
    // launch catalog data only; native identity still comes from production.
    let quoted = executable
        .chars()
        .map(|character| match character {
            '\\' | '"' | '`' | '$' => format!("\\{character}"),
            _ => character.to_string(),
        })
        .collect::<String>();
    let applications = runtime.join("data/applications");
    fs::create_dir_all(&applications).map_err(|error| error.to_string())?;
    fs::write(applications.join(format!("{COUNTER_CATALOG_ID}.desktop")), format!(
        "[Desktop Entry]\nType=Application\nName=Nickel native movement counter\nExec=\"{quoted}\"\nTerminal=false\n"
    )).map_err(|error| error.to_string())
}

pub(super) fn exercise(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    bootstrap: u64,
) -> Result<u64, String> {
    let backend = if xwayland_enabled() {
        FixtureBackend::Xwayland
    } else {
        FixtureBackend::Wayland
    };
    session_message(
        environment,
        Request::Command(Command::SetLauncherVisible { visible: false }),
    )?;
    let mut recipient = OrdinaryClient::spawn(environment, backend, "keyboard_recipient", "first")?;
    let first = wait_for_window(address, identity, bootstrap, &[], &mut recipient)?;
    if first["title"] != "Keyboard recipient acceptance" {
        return Err("owned keyboard fixture published an unexpected window".into());
    }
    backend.require_x11_class(&first)?;
    // Title identifies the expected fixture only. Authority comes exclusively
    // from the production owner's native process/executable evidence.
    let application = first["verified_application"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("native owner could not verify the keyboard fixture application")?
        .to_owned();
    let first_resource = native_resource(&first, "id")?;
    let mut unrelated = OrdinaryClient::spawn(environment, backend, "standalone", "unrelated")?;
    let other = wait_for_window(
        address,
        identity,
        bootstrap,
        std::slice::from_ref(&first_resource.id),
        &mut unrelated,
    )?;
    if other["title"] != "Nickel UI Counter"
        || other["verified_application"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id == application)
    {
        return Err(
            "unrelated example lacks a distinct owner-verified application identity".into(),
        );
    }
    backend.require_x11_class(&other)?;
    if matches!(backend, FixtureBackend::Xwayland)
        && other["application_id"] == first["application_id"]
    {
        return Err("distinct X11 fixtures published the same WM_CLASS".into());
    }
    let movement = if movement_enabled() {
        Some(Movement::prepare(
            environment,
            address,
            identity,
            bootstrap,
            &first,
            &other,
        )?)
    } else {
        None
    };
    revoke_scope(environment, bootstrap)?;

    let scope = RemoteResourceScope::Window(first_resource.clone());
    let lease = approve_scope(environment, address, identity, scope.clone(), false)?;
    let approved = remote_snapshot(environment)?;
    let scope_test = OrdinaryScope {
        environment,
        address,
        identity,
        lease,
        scope: &scope,
        approved: &approved,
    };
    scope_test.input(&first, &mut recipient, "abc")?;
    scope_test.pointer_and_capture(&first, &mut recipient)?;
    if let Some(movement) = &movement {
        movement.follow(&scope_test, &first, &mut recipient)?;
    }
    deny_unrelated(environment, address, identity, lease, &other)?;
    require_unchanged_scope_approval(&approved, &remote_snapshot(environment)?, lease, &scope)?;
    println!(
        "PASS: native ordinary window scope, one approval, client-confirmed key and pointer input, identity-bound PNG capture, unrelated window denied"
    );
    revoke_scope(environment, lease)?;
    if matches!(backend, FixtureBackend::Xwayland) {
        xwayland_held_input(environment, address, identity, &first, &mut recipient)?;
    }

    let scope = RemoteResourceScope::Application(application.clone());
    let lease = approve_scope(environment, address, identity, scope.clone(), false)?;
    let approved = remote_snapshot(environment)?;
    let scope_test = OrdinaryScope {
        environment,
        address,
        identity,
        lease,
        scope: &scope,
        approved: &approved,
    };
    scope_test.input(&first, &mut recipient, "def")?;
    scope_test.pointer_and_capture(&first, &mut recipient)?;
    if let Some(movement) = &movement {
        movement.follow(&scope_test, &first, &mut recipient)?;
        movement.deny_counter_launch(&scope_test)?;
    }
    // A later process/window must inherit through verified executable identity;
    // it did not exist when the application lease was approved.
    let mut second = OrdinaryClient::spawn(environment, backend, "keyboard_recipient", "second")?;
    let additional = wait_for_window(address, identity, lease, &[first_resource.id], &mut second)?;
    if additional["verified_application"] != application {
        return Err("additional native window does not match the approved application".into());
    }
    scope_test.input(&additional, &mut second, "ghi")?;
    deny_unrelated(environment, address, identity, lease, &other)?;
    let windows = scope_call(
        address,
        identity,
        "list_windows",
        json!({"lease_id": lease}),
    )?;
    let windows = windows
        .as_array()
        .ok_or("application inventory is not an array")?;
    if windows.len() != 2
        || windows
            .iter()
            .any(|window| window["verified_application"] != application)
    {
        return Err(
            "application lease inventory did not remain confined to its two native windows".into(),
        );
    }
    require_unchanged_scope_approval(&approved, &remote_snapshot(environment)?, lease, &scope)?;
    println!(
        "PASS: native application scope, one approval, client-confirmed key and pointer input, identity-bound PNG capture, later same-executable window inherited authority, unrelated executable denied"
    );
    if let Some(movement) = &movement {
        movement.application_held_input(
            environment,
            address,
            identity,
            lease,
            &application,
            &first,
            &other,
            &mut recipient,
        )?;
    }
    revoke_scope(environment, lease)?;
    if let Some(movement) = &movement {
        movement.output_boundary(environment, address, identity, &first, &mut recipient)?;
        movement.authorized_output_launch(environment, address, identity)?;
        movement.held_input(environment, address, identity, &first, &mut recipient)?;
        movement.cleanup(environment)?;
    }
    drop(second);
    drop(unrelated);
    drop(recipient);
    approve_scope(
        environment,
        address,
        identity,
        RemoteResourceScope::FullSession,
        true,
    )
}

fn xwayland_held_input(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    window: &Value,
    client: &mut OrdinaryClient,
) -> Result<(), String> {
    let contender = connect_identity(address, "native-xwayland-held-contender")?;
    let watch = ConnectionWatch::start(address, &contender)?;
    let scope = RemoteResourceScope::Window(native_resource(window, "id")?);
    let result = (|| {
        for kind in [HeldKind::Key, HeldKind::Drag] {
            let owner_lease = approve_scope(environment, address, identity, scope.clone(), false)?;
            let contender_lease = approve_overlapping_window(
                environment,
                address,
                &contender,
                owner_lease,
                native_resource(window, "id")?,
            )?;
            scope_call(
                address,
                identity,
                "focus_window",
                window_arguments(owner_lease, window),
            )?;

            let mut receipts = client.hold_receipts(kind)?;
            if receipts.0 != receipts.1 {
                return Err("X11 fixture already has unbalanced held input".into());
            }
            start_hold(address, identity, owner_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_contender_denied(address, &contender, contender_lease, window, kind)?;
            require_non_owner_release_denied(address, &contender, contender_lease, window, kind)?;
            client.wait_hold_receipts(kind, receipts)?;
            finish_hold(address, identity, owner_lease, window, kind, HoldStep::End)?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;

            start_hold(address, identity, owner_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_non_owner_release_denied(address, &contender, contender_lease, window, kind)?;
            client.wait_hold_receipts(kind, receipts)?;
            finish_hold(
                address,
                identity,
                owner_lease,
                window,
                kind,
                HoldStep::Cancel,
            )?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;

            start_hold(address, identity, owner_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            session_message(
                environment,
                Request::Command(Command::ManageRemoteLease {
                    lease_id: owner_lease,
                    action: RemoteLeaseAction::Revoke,
                }),
            )?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_retired_lease(
                environment,
                owner_lease,
                contender_lease,
                RemoteLeaseTransition::Revoked,
            )?;
            require_stale_owner_denied(address, identity, owner_lease, window, kind)?;

            scope_call(
                address,
                &contender,
                "focus_window",
                window_arguments(contender_lease, window),
            )?;
            start_hold(address, &contender, contender_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_stale_owner_denied(address, identity, owner_lease, window, kind)?;
            client.wait_hold_receipts(kind, receipts)?;
            keep_alive_hold(address, &contender, contender_lease, window, kind)?;
            finish_hold(
                address,
                &contender,
                contender_lease,
                window,
                kind,
                HoldStep::End,
            )?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;

            let expiring_lease = approve_overlapping_window_for(
                environment,
                address,
                identity,
                contender_lease,
                native_resource(window, "id")?,
                1,
            )?;
            scope_call(
                address,
                identity,
                "focus_window",
                window_arguments(expiring_lease, window),
            )?;
            start_hold(address, identity, expiring_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            wait_for_lease_retirement(
                environment,
                address,
                expiring_lease,
                RemoteLeaseTransition::Expired,
                contender_lease,
                Instant::now() + Duration::from_secs(5),
            )?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_stale_owner_denied(address, identity, expiring_lease, window, kind)?;

            scope_call(
                address,
                &contender,
                "focus_window",
                window_arguments(contender_lease, window),
            )?;
            start_hold(address, &contender, contender_lease, window, kind)?;
            receipts.0 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            require_stale_owner_denied(address, identity, expiring_lease, window, kind)?;
            client.wait_hold_receipts(kind, receipts)?;
            keep_alive_hold(address, &contender, contender_lease, window, kind)?;
            finish_hold(
                address,
                &contender,
                contender_lease,
                window,
                kind,
                HoldStep::End,
            )?;
            receipts.1 += 1;
            client.wait_hold_receipts(kind, receipts)?;
            revoke_scope(environment, contender_lease)?;
            println!(
                "PASS: native X11 {} hold has client-confirmed owner release/cancel, overlapping key/drag/focus denial, revocation and expiry release, and stale-owner isolation",
                kind.marker()
            );
        }
        Ok(())
    })();
    let watch_result = watch.finish();
    result.and(watch_result)
}

fn start_hold(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
    kind: HeldKind,
) -> Result<(), String> {
    let (method, request) = kind.request(lease, window, HoldStep::Start)?;
    scope_call(address, identity, method, request).map(|_| ())
}

fn keep_alive_hold(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
    kind: HeldKind,
) -> Result<(), String> {
    let (method, request) = kind.request(lease, window, HoldStep::Continue)?;
    scope_call(address, identity, method, request).map(|_| ())
}

fn finish_hold(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
    kind: HeldKind,
    step: HoldStep,
) -> Result<(), String> {
    let (method, request) = kind.request(lease, window, step)?;
    scope_call(address, identity, method, request).map(|_| ())
}

fn require_non_owner_release_denied(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
    kind: HeldKind,
) -> Result<(), String> {
    for step in [HoldStep::End, HoldStep::Cancel] {
        let (method, request) = kind.request(lease, window, step)?;
        let response = mcp_call(address, identity, method, request)?;
        require_tool_error("non-owner held-input release", &response)?;
    }
    Ok(())
}

fn require_stale_owner_denied(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
    kind: HeldKind,
) -> Result<(), String> {
    for step in [HoldStep::Continue, HoldStep::End, HoldStep::Cancel] {
        let (method, request) = kind.request(lease, window, step)?;
        let response = mcp_call(address, identity, method, request)?;
        require_tool_error("retired held-input owner", &response)?;
    }
    Ok(())
}

fn require_retired_lease(
    environment: &SessionEnvironment,
    retired_lease: u64,
    retained_lease: u64,
    transition: RemoteLeaseTransition,
) -> Result<(), String> {
    let snapshot = remote_snapshot(environment)?;
    let retired = !snapshot
        .active_leases
        .iter()
        .any(|lease| lease.lease_id == retired_lease);
    let retained = snapshot
        .active_leases
        .iter()
        .any(|lease| lease.lease_id == retained_lease);
    let audited = snapshot
        .lease_audit
        .iter()
        .any(|event| event.lease_id == retired_lease && event.transition == transition);
    if retired && retained && audited {
        Ok(())
    } else {
        Err(format!(
            "held-input lease retirement incomplete: retired={retired} retained={retained} audited={audited}"
        ))
    }
}

struct Movement {
    primary: nickel_session_protocol::RemoteResourceId,
    secondary: nickel_session_protocol::RemoteResourceId,
    original_workspace: nickel_session_protocol::WorkspaceId,
    second_workspace: nickel_session_protocol::WorkspaceId,
    counter_application: String,
}

impl Movement {
    fn prepare(
        environment: &SessionEnvironment,
        address: SocketAddr,
        identity: &Identity,
        lease: u64,
        first: &Value,
        other: &Value,
    ) -> Result<Self, String> {
        let outputs = scope_call(
            address,
            identity,
            "list_outputs",
            json!({"lease_id": lease}),
        )?;
        let primary = outputs["outputs"]
            .as_array()
            .and_then(|outputs| outputs.iter().find(|output| output["primary"] == true))
            .ok_or("native movement requires an identified primary output")?;
        let primary = native_resource(primary, "name")?;
        local_command(
            environment,
            Command::TestOutput {
                output: nickel_session_protocol::TestOutput::Connect {
                    name: MOVEMENT_OUTPUT.into(),
                    logical_width: 1280,
                    logical_height: 720,
                    scale_120: 180,
                    transform: nickel_session_protocol::OutputTransform::Rotate90,
                },
            },
        )?;
        let ServerMessage::Outputs(native_outputs) =
            session_message(environment, Request::Query(Query::Outputs))?
        else {
            return Err("native transformed output observation unavailable".into());
        };
        let native_secondary = native_outputs
            .iter()
            .find(|output| output.name == MOVEMENT_OUTPUT)
            .ok_or("nested transformed output was not mapped")?;
        if native_secondary.scale_120 != 180
            || native_secondary.transform != nickel_session_protocol::OutputTransform::Rotate90
            || native_secondary.geometry.width != 1280
            || native_secondary.geometry.height != 720
        {
            return Err("nested output lost its fractional scale or transform geometry".into());
        }
        let outputs = scope_call(
            address,
            identity,
            "list_outputs",
            json!({"lease_id": lease}),
        )?;
        let secondary = outputs["outputs"]
            .as_array()
            .and_then(|outputs| {
                outputs
                    .iter()
                    .find(|output| output["name"] == MOVEMENT_OUTPUT)
            })
            .ok_or("nested output was not published through the production owner")?;
        let secondary = native_resource(secondary, "name")?;
        let ServerMessage::Workspaces(before) =
            session_message(environment, Request::Query(Query::Workspaces))?
        else {
            return Err("native workspace inventory unavailable".into());
        };
        let ServerMessage::Workspaces(after) =
            session_message(environment, Request::Command(Command::CreateWorkspace))?
        else {
            return Err("new native workspace inventory unavailable".into());
        };
        let mut added = after
            .ordered
            .iter()
            .filter(|workspace| !before.ordered.iter().any(|old| old.id == workspace.id));
        let second_workspace = added.next().ok_or("new workspace missing")?.id;
        if added.next().is_some() {
            return Err("ambiguous new workspace".into());
        }
        let original_workspace = nickel_session_protocol::WorkspaceId(
            first["workspace"]
                .as_u64()
                .ok_or("window workspace missing")?,
        );
        let catalog = scope_call(
            address,
            identity,
            "list_installed_applications",
            json!({"lease_id": lease}),
        )?;
        let counter = catalog["applications"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["id"] == COUNTER_CATALOG_ID)
            })
            .ok_or("real counter fixture missing from installed catalog")?;
        if counter["verified_application"] != other["verified_application"] {
            return Err("catalog counter identity does not match its live native process".into());
        }
        Ok(Self {
            primary,
            secondary,
            original_workspace,
            second_workspace,
            counter_application: counter["verified_application"]
                .as_str()
                .ok_or("catalog counter identity is unavailable")?
                .to_owned(),
        })
    }

    fn follow(
        &self,
        test: &OrdinaryScope<'_>,
        window: &Value,
        client: &mut OrdinaryClient,
    ) -> Result<(), String> {
        let id = protocol_window_id(window)?;
        local_command(
            test.environment,
            Command::MoveWindowToOutput {
                window: id,
                output: MOVEMENT_OUTPUT.into(),
            },
        )?;
        verify_placement(
            test.environment,
            id,
            MOVEMENT_OUTPUT,
            self.original_workspace,
        )?;
        test.input(window, client, "j")?;
        local_command(
            test.environment,
            Command::MoveWindowToWorkspace {
                window: id,
                workspace: self.second_workspace,
            },
        )?;
        local_command(
            test.environment,
            Command::SwitchWorkspace {
                workspace: self.second_workspace,
                output: Some(MOVEMENT_OUTPUT.into()),
            },
        )?;
        verify_placement(test.environment, id, MOVEMENT_OUTPUT, self.second_workspace)?;
        test.input(window, client, "k")?;
        local_command(
            test.environment,
            Command::MoveWindowToWorkspace {
                window: id,
                workspace: self.original_workspace,
            },
        )?;
        local_command(
            test.environment,
            Command::SwitchWorkspace {
                workspace: self.original_workspace,
                output: Some(MOVEMENT_OUTPUT.into()),
            },
        )?;
        local_command(
            test.environment,
            Command::MoveWindowToOutput {
                window: id,
                output: self.primary.id.clone(),
            },
        )?;
        verify_placement(
            test.environment,
            id,
            &self.primary.id,
            self.original_workspace,
        )?;
        test.input(window, client, "l")?;
        println!(
            "PASS: {:?} lease follows owner-verified output/workspace movement and return with native client-confirmed keys and no new approval",
            test.scope
        );
        Ok(())
    }

    fn deny_counter_launch(&self, test: &OrdinaryScope<'_>) -> Result<(), String> {
        let catalog = scope_call(
            test.address,
            test.identity,
            "list_installed_applications",
            json!({"lease_id": test.lease}),
        )?;
        if catalog["applications"].as_array().is_none_or(|entries| {
            entries
                .iter()
                .any(|entry| entry["id"] == COUNTER_CATALOG_ID)
        }) {
            return Err("application catalog exposed an unrelated executable".into());
        }
        let response = mcp_call(
            test.address,
            test.identity,
            "launch_installed_application",
            json!({
                "lease_id": test.lease, "application_id": COUNTER_CATALOG_ID, "catalog_generation": catalog["catalog_generation"]
            }),
        )?;
        require_tool_error("different-application launch", &response)?;
        if !response
            .to_string()
            .contains("outside the application lease")
        {
            return Err(format!(
                "different-application launch failed for an unrelated reason: {response}"
            ));
        }
        require_unchanged_scope_approval(
            test.approved,
            &remote_snapshot(test.environment)?,
            test.lease,
            test.scope,
        )?;
        println!(
            "PASS: application lease denies launch of real catalog executable with distinct verified identity after movement"
        );
        Ok(())
    }

    fn output_boundary(
        &self,
        environment: &SessionEnvironment,
        address: SocketAddr,
        identity: &Identity,
        window: &Value,
        client: &mut OrdinaryClient,
    ) -> Result<(), String> {
        let scope = RemoteResourceScope::Output(self.primary.clone());
        let lease = approve_scope(environment, address, identity, scope.clone(), false)?;
        let approved = remote_snapshot(environment)?;
        let test = OrdinaryScope {
            environment,
            address,
            identity,
            lease,
            scope: &scope,
            approved: &approved,
        };
        test.input(window, client, "m")?;
        let id = protocol_window_id(window)?;
        local_command(
            environment,
            Command::MoveWindowToOutput {
                window: id,
                output: MOVEMENT_OUTPUT.into(),
            },
        )?;
        verify_placement(environment, id, MOVEMENT_OUTPUT, self.original_workspace)?;
        let inventory = scope_call(
            address,
            identity,
            "list_windows",
            json!({"lease_id": lease}),
        )?;
        if inventory
            .as_array()
            .is_none_or(|windows| windows.iter().any(|current| current["id"] == window["id"]))
        {
            return Err("output lease still exposes a window that moved outside its output".into());
        }
        // This also focuses the moved window locally, so keyboard denial cannot
        // be explained by lack of focus. All errors must be scope denials.
        deny_unrelated(environment, address, identity, lease, window)?;
        require_unchanged_scope_approval(&approved, &remote_snapshot(environment)?, lease, &scope)?;
        local_command(
            environment,
            Command::MoveWindowToOutput {
                window: id,
                output: self.primary.id.clone(),
            },
        )?;
        verify_placement(environment, id, &self.primary.id, self.original_workspace)?;
        test.input(window, client, "n")?;
        revoke_scope(environment, lease)?;
        println!(
            "PASS: same output lease loses inventory/focus/capture/key authority after movement and regains native input on return without reapproval"
        );
        Ok(())
    }

    fn authorized_output_launch(
        &self,
        environment: &SessionEnvironment,
        address: SocketAddr,
        identity: &Identity,
    ) -> Result<(), String> {
        let scope = RemoteResourceScope::Output(self.secondary.clone());
        let lease = approve_scope(environment, address, identity, scope.clone(), false)?;
        let approved = remote_snapshot(environment)?;
        let before = scope_call(
            address,
            identity,
            "list_windows",
            json!({"lease_id": lease}),
        )?;
        let existing = before
            .as_array()
            .ok_or("prelaunch output inventory is not an array")?
            .iter()
            .filter_map(|window| window["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let catalog = scope_call(
            address,
            identity,
            "list_installed_applications",
            json!({"lease_id": lease}),
        )?;
        let entry = catalog["applications"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["id"] == COUNTER_CATALOG_ID)
            })
            .ok_or("counter fixture is unavailable to the output lease")?;
        if entry["verified_application"] != self.counter_application {
            return Err("launch catalog identity changed before output-scoped launch".into());
        }
        let outcome = scope_call(
            address,
            identity,
            "launch_installed_application",
            json!({
                "lease_id": lease,
                "application_id": COUNTER_CATALOG_ID,
                "catalog_generation": catalog["catalog_generation"],
            }),
        )?;
        if outcome["application_id"] != COUNTER_CATALOG_ID
            || outcome["requested"] != true
            || outcome["process_spawn_confirmed"] != true
            || outcome["process_id"].as_u64().is_none_or(|pid| pid == 0)
            || outcome["output_requested"]["id"] != self.secondary.id
            || outcome["output_requested"]["generation"] != self.secondary.generation
            || outcome["output_confirmed"] != false
        {
            return Err(format!(
                "output-scoped launch returned an incoherent requested/confirmed outcome: {outcome}"
            ));
        }

        let window = wait_for_scoped_window(address, identity, lease, &existing)?;
        if window["title"] != "Nickel UI Counter"
            || window["verified_application"] != self.counter_application
        {
            return Err("output-scoped launch mapped an unexpected native application".into());
        }
        verify_placement(
            environment,
            protocol_window_id(&window)?,
            MOVEMENT_OUTPUT,
            self.original_workspace,
        )?;
        scope_call(
            address,
            identity,
            "focus_window",
            window_arguments(lease, &window),
        )?;
        scope_call(
            address,
            identity,
            "capture_window",
            window_arguments(lease, &window),
        )?;
        let global_x = i32::try_from(window["x"].as_i64().ok_or("launched window x missing")?)
            .map_err(|_| "launched window x exceeds i32")?
            .checked_add(1)
            .ok_or("launched window x overflow")?;
        let global_y = i32::try_from(window["y"].as_i64().ok_or("launched window y missing")?)
            .map_err(|_| "launched window y exceeds i32")?
            .checked_add(1)
            .ok_or("launched window y overflow")?;
        scope_call(
            address,
            identity,
            "pointer_action",
            json!({
                "lease_id": lease,
                "target": {
                    "kind": "output", "output_id": self.secondary.id,
                    "generation": self.secondary.generation
                },
                "x": global_x, "y": global_y,
                "action": {"kind": "move"}
            }),
        )?;
        let inventory = scope_call(
            address,
            identity,
            "list_windows",
            json!({"lease_id": lease}),
        )?;
        if !inventory.as_array().is_some_and(|windows| {
            windows
                .iter()
                .any(|candidate| candidate["id"] == window["id"] && candidate["active"] == true)
        }) {
            return Err(
                "output lease did not retain focus authority over its launched window".into(),
            );
        }
        require_unchanged_scope_approval(&approved, &remote_snapshot(environment)?, lease, &scope)?;
        revoke_scope(environment, lease)?;
        println!(
            "PASS: output lease launched the real catalog application, production placement and final global pointer hit stayed confined on the fractional transformed output, and focus/capture required no new approval"
        );
        Ok(())
    }

    fn cleanup(&self, environment: &SessionEnvironment) -> Result<(), String> {
        local_command(
            environment,
            Command::RemoveWorkspace {
                workspace: self.second_workspace,
            },
        )?;
        local_command(
            environment,
            Command::TestOutput {
                output: nickel_session_protocol::TestOutput::Disconnect {
                    name: MOVEMENT_OUTPUT.into(),
                },
            },
        )
    }

    fn held_input(
        &self,
        environment: &SessionEnvironment,
        address: SocketAddr,
        identity: &Identity,
        window: &Value,
        client: &mut OrdinaryClient,
    ) -> Result<(), String> {
        let contender = connect_identity(address, "native-held-contender")?;
        let watch = ConnectionWatch::start(address, &contender)?;
        let result = (|| {
            for kind in [HeldKind::Key, HeldKind::Drag] {
                let owner_lease = approve_scope(
                    environment,
                    address,
                    identity,
                    RemoteResourceScope::Output(self.primary.clone()),
                    false,
                )?;
                let contender_lease = approve_overlapping_window(
                    environment,
                    address,
                    &contender,
                    owner_lease,
                    native_resource(window, "id")?,
                )?;
                let approved = remote_snapshot(environment)?;
                scope_call(
                    address,
                    identity,
                    "focus_window",
                    window_arguments(owner_lease, window),
                )?;
                let baseline = client.hold_receipts(kind)?;
                if baseline.0 != baseline.1 {
                    return Err("native fixture already has unbalanced held input".into());
                }
                let (method, start) = kind.request(owner_lease, window, HoldStep::Start)?;
                scope_call(address, identity, method, start)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1))?;

                // Both leases cover the current window. Observation succeeds;
                // only shared input arbitration can explain these denials.
                require_contender_denied(address, &contender, contender_lease, window, kind)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1))?;

                local_command(
                    environment,
                    Command::MoveWindowToOutput {
                        window: protocol_window_id(window)?,
                        output: MOVEMENT_OUTPUT.into(),
                    },
                )?;
                verify_placement(
                    environment,
                    protocol_window_id(window)?,
                    MOVEMENT_OUTPUT,
                    self.original_workspace,
                )?;
                // Require the client's actual non-synthetic key/button release,
                // not just an owner bookkeeping transition or a failed RPC.
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1 + 1))?;
                let (method, continuation) =
                    kind.request(owner_lease, window, HoldStep::Continue)?;
                require_tool_error(
                    "old output owner continuation",
                    &mcp_call(address, identity, method, continuation)?,
                )?;
                let inventory = scope_call(
                    address,
                    identity,
                    "list_windows",
                    json!({"lease_id": owner_lease}),
                )?;
                if inventory.as_array().is_none_or(|windows| {
                    windows
                        .iter()
                        .any(|candidate| candidate["id"] == window["id"])
                }) {
                    return Err("held target escaped its output lease through inventory".into());
                }

                scope_call(
                    address,
                    &contender,
                    "focus_window",
                    window_arguments(contender_lease, window),
                )?;
                let (method, start) = kind.request(contender_lease, window, HoldStep::Start)?;
                scope_call(address, &contender, method, start)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 1))?;
                let (method, cancel) = kind.request(owner_lease, window, HoldStep::Cancel)?;
                require_tool_error(
                    "old owner cancelling new owner",
                    &mcp_call(address, identity, method, cancel)?,
                )?;
                let (method, continuation) =
                    kind.request(contender_lease, window, HoldStep::Continue)?;
                scope_call(address, &contender, method, continuation)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 1))?;
                let (method, end) = kind.request(contender_lease, window, HoldStep::End)?;
                scope_call(address, &contender, method, end)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 2))?;

                local_command(
                    environment,
                    Command::MoveWindowToOutput {
                        window: protocol_window_id(window)?,
                        output: self.primary.id.clone(),
                    },
                )?;
                let (method, stale) = kind.request(owner_lease, window, HoldStep::Continue)?;
                require_tool_error(
                    "cancelled hold after window return",
                    &mcp_call(address, identity, method, stale)?,
                )?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 2))?;
                let current = remote_snapshot(environment)?;
                if !current.pending_leases.is_empty()
                    || current.active_leases.len() != 2
                    || current.permission_audit != approved.permission_audit
                    || current.lease_audit != approved.lease_audit
                {
                    return Err(
                        "held input required another approval or changed lease authority".into(),
                    );
                }
                session_message(
                    environment,
                    Request::Command(Command::ManageRemoteLease {
                        lease_id: owner_lease,
                        action: RemoteLeaseAction::Revoke,
                    }),
                )?;
                revoke_scope(environment, contender_lease)?;
                println!(
                    "PASS: native {} hold excludes competing key/drag/focus, releases on output exit, rejects stale continuation/cancel and preserves the next owner's hold",
                    kind.marker()
                );
            }
            Ok(())
        })();
        let watch_result = watch.finish();
        result.and(watch_result)
    }

    #[allow(clippy::too_many_arguments)]
    fn application_held_input(
        &self,
        environment: &SessionEnvironment,
        address: SocketAddr,
        identity: &Identity,
        application_lease: u64,
        application: &str,
        window: &Value,
        unrelated: &Value,
        client: &mut OrdinaryClient,
    ) -> Result<(), String> {
        let contender = connect_identity(address, "native-application-held-contender")?;
        let watch = ConnectionWatch::start(address, &contender)?;
        let result = (|| {
            let contender_lease = approve_overlapping_window(
                environment,
                address,
                &contender,
                application_lease,
                native_resource(window, "id")?,
            )?;
            let approved = remote_snapshot(environment)?;
            let application_scope = RemoteResourceScope::Application(application.into());
            for kind in [HeldKind::Key, HeldKind::Drag] {
                scope_call(
                    address,
                    identity,
                    "focus_window",
                    window_arguments(application_lease, window),
                )?;
                let baseline = client.hold_receipts(kind)?;
                if baseline.0 != baseline.1 {
                    return Err(
                        "native fixture already has unbalanced application-held input".into(),
                    );
                }
                let (method, start) = kind.request(application_lease, window, HoldStep::Start)?;
                scope_call(address, identity, method, start)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1))?;
                require_contender_denied(address, &contender, contender_lease, window, kind)?;

                local_command(
                    environment,
                    Command::MoveWindowToOutput {
                        window: protocol_window_id(window)?,
                        output: MOVEMENT_OUTPUT.into(),
                    },
                )?;
                verify_placement(
                    environment,
                    protocol_window_id(window)?,
                    MOVEMENT_OUTPUT,
                    self.original_workspace,
                )?;
                let (method, continuation) =
                    kind.request(application_lease, window, HoldStep::Continue)?;
                scope_call(address, identity, method, continuation)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1))?;

                local_command(
                    environment,
                    Command::MoveWindowToWorkspaceAndSwitch {
                        window: protocol_window_id(window)?,
                        workspace: self.second_workspace,
                        output: Some(MOVEMENT_OUTPUT.into()),
                    },
                )?;
                verify_placement(
                    environment,
                    protocol_window_id(window)?,
                    MOVEMENT_OUTPUT,
                    self.second_workspace,
                )?;
                let (method, continuation) =
                    kind.request(application_lease, window, HoldStep::Continue)?;
                scope_call(address, identity, method, continuation)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1))?;
                require_unchanged_overlapping_approval(
                    &approved,
                    &remote_snapshot(environment)?,
                    application_lease,
                    &application_scope,
                    contender_lease,
                    &RemoteResourceScope::Window(native_resource(window, "id")?),
                )?;

                // Activating the unrelated production window changes the
                // exact recipient and must release the real client input. The
                // application lease remains unable to use that unrelated app.
                deny_unrelated(environment, address, identity, application_lease, unrelated)?;
                client.wait_hold_receipts(kind, (baseline.0 + 1, baseline.1 + 1))?;
                let (method, stale) =
                    kind.request(application_lease, window, HoldStep::Continue)?;
                require_tool_error(
                    "application owner continuation after recipient change",
                    &mcp_call(address, identity, method, stale)?,
                )?;

                scope_call(
                    address,
                    &contender,
                    "focus_window",
                    window_arguments(contender_lease, window),
                )?;
                let (method, start) = kind.request(contender_lease, window, HoldStep::Start)?;
                scope_call(address, &contender, method, start)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 1))?;
                let (method, stale_cancel) =
                    kind.request(application_lease, window, HoldStep::Cancel)?;
                require_tool_error(
                    "stale application owner cancelling contender input",
                    &mcp_call(address, identity, method, stale_cancel)?,
                )?;
                let (method, continuation) =
                    kind.request(contender_lease, window, HoldStep::Continue)?;
                scope_call(address, &contender, method, continuation)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 1))?;
                let (method, end) = kind.request(contender_lease, window, HoldStep::End)?;
                scope_call(address, &contender, method, end)?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 2))?;

                local_command(
                    environment,
                    Command::MoveWindowToWorkspaceAndSwitch {
                        window: protocol_window_id(window)?,
                        workspace: self.original_workspace,
                        output: Some(MOVEMENT_OUTPUT.into()),
                    },
                )?;
                local_command(
                    environment,
                    Command::MoveWindowToOutput {
                        window: protocol_window_id(window)?,
                        output: self.primary.id.clone(),
                    },
                )?;
                verify_placement(
                    environment,
                    protocol_window_id(window)?,
                    &self.primary.id,
                    self.original_workspace,
                )?;
                let (method, stale) =
                    kind.request(application_lease, window, HoldStep::Continue)?;
                require_tool_error(
                    "retired application hold after window return",
                    &mcp_call(address, identity, method, stale)?,
                )?;
                client.wait_hold_receipts(kind, (baseline.0 + 2, baseline.1 + 2))?;
                require_unchanged_overlapping_approval(
                    &approved,
                    &remote_snapshot(environment)?,
                    application_lease,
                    &application_scope,
                    contender_lease,
                    &RemoteResourceScope::Window(native_resource(window, "id")?),
                )?;
                println!(
                    "PASS: native application-scoped {} hold continues across owner-verified output/workspace movement, denies shared-input contenders and an unrelated application, releases on recipient change, and isolates stale owners without another approval",
                    kind.marker()
                );
            }
            session_message(
                environment,
                Request::Command(Command::ManageRemoteLease {
                    lease_id: contender_lease,
                    action: RemoteLeaseAction::Revoke,
                }),
            )?;
            let remaining = remote_snapshot(environment)?;
            if !remaining.pending_leases.is_empty()
                || remaining.active_leases.len() != 1
                || remaining.active_leases[0].lease_id != application_lease
                || remaining.active_leases[0].scope != application_scope
            {
                return Err(
                    "contender revocation did not preserve only application authority".into(),
                );
            }
            Ok(())
        })();
        let watch_result = watch.finish();
        result.and(watch_result)
    }
}

#[derive(Clone, Copy)]
enum HeldKind {
    Key,
    Drag,
}
#[derive(Clone, Copy)]
enum HoldStep {
    Start,
    Continue,
    End,
    Cancel,
}
impl HeldKind {
    fn marker(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Drag => "button",
        }
    }
    fn request(
        self,
        lease: u64,
        window: &Value,
        step: HoldStep,
    ) -> Result<(&'static str, Value), String> {
        let mut request = window_arguments(lease, window);
        let method = match self {
            Self::Key => {
                request["action"] = match step {
                    HoldStep::Start => {
                        json!({"kind": "hold_start", "keysym": 65505, "modifiers": []})
                    }
                    HoldStep::Continue => json!({"kind": "hold_keep_alive"}),
                    HoldStep::End => json!({"kind": "hold_end"}),
                    HoldStep::Cancel => json!({"kind": "hold_cancel"}),
                };
                "keyboard_action"
            }
            Self::Drag => {
                request["x"] = json!(
                    window["width"]
                        .as_u64()
                        .ok_or("held target width missing")?
                        / 2
                );
                request["y"] = json!(
                    window["height"]
                        .as_u64()
                        .ok_or("held target height missing")?
                        / 2
                );
                request["action"] = match step {
                    HoldStep::Start => json!({"kind": "drag_start", "button": "left"}),
                    HoldStep::Continue => json!({"kind": "drag_move"}),
                    HoldStep::End => json!({"kind": "drag_end"}),
                    HoldStep::Cancel => json!({"kind": "drag_cancel"}),
                };
                "pointer_action"
            }
        };
        Ok((method, request))
    }
}

fn require_contender_denied(
    address: SocketAddr,
    contender: &Identity,
    lease: u64,
    window: &Value,
    held: HeldKind,
) -> Result<(), String> {
    let inventory = scope_call(
        address,
        contender,
        "list_windows",
        json!({"lease_id": lease}),
    )?;
    if !inventory.as_array().is_some_and(|windows| {
        windows
            .iter()
            .any(|candidate| candidate["id"] == window["id"])
    }) {
        return Err("contender cannot observe its authorized held target".into());
    }
    for candidate in [HeldKind::Key, HeldKind::Drag] {
        let (method, arguments) = candidate.request(lease, window, HoldStep::Start)?;
        let response = mcp_call(address, contender, method, arguments)?;
        require_tool_error("contending native input", &response)?;
        let diagnostic = response.to_string();
        // The pointer adapter checks native pressed keys before lease
        // arbitration and currently labels any such key local.
        let native_key_busy = matches!((held, candidate), (HeldKind::Key, HeldKind::Drag))
            && diagnostic.contains("local keyboard input is held");
        let native_pointer_busy = matches!((held, candidate), (HeldKind::Drag, HeldKind::Key))
            && diagnostic.contains("pointer input is held or grabbed");
        if !diagnostic.contains("shared input") && !native_key_busy && !native_pointer_busy {
            return Err(format!(
                "input contention rejected for the wrong reason: {response}"
            ));
        }
    }
    let focus = mcp_call(
        address,
        contender,
        "focus_window",
        window_arguments(lease, window),
    )?;
    require_tool_error("contending focus", &focus)?;
    if !focus.to_string().contains("shared input") {
        return Err(format!(
            "focus contention rejected for the wrong reason: {focus}"
        ));
    }
    Ok(())
}

fn require_unchanged_overlapping_approval(
    approved: &RemoteControlSnapshot,
    current: &RemoteControlSnapshot,
    owner_lease: u64,
    owner_scope: &RemoteResourceScope,
    contender_lease: u64,
    contender_scope: &RemoteResourceScope,
) -> Result<(), String> {
    let has_lease = |lease_id, scope: &RemoteResourceScope| {
        current.active_leases.iter().any(|lease| {
            lease.lease_id == lease_id
                && &lease.scope == scope
                && !lease.suspended
                && !lease.full_debug
        })
    };
    if !current.pending_leases.is_empty()
        || current.effective != RemoteControlEffectiveState::Enabled
        || current.active_leases.len() != 2
        || !has_lease(owner_lease, owner_scope)
        || !has_lease(contender_lease, contender_scope)
        || current.permission_audit != approved.permission_audit
        || current.permission_audit_evicted != approved.permission_audit_evicted
        || current.lease_audit != approved.lease_audit
        || current.lease_audit_evicted != approved.lease_audit_evicted
    {
        return Err(
            "application-held movement changed authority or requested another approval".into(),
        );
    }
    Ok(())
}
fn window_arguments(lease: u64, window: &Value) -> Value {
    json!({"lease_id": lease, "window_id": window["id"], "generation": window["generation"]})
}
fn approve_overlapping_window(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    existing: u64,
    resource: nickel_session_protocol::RemoteResourceId,
) -> Result<u64, String> {
    approve_overlapping_window_for(environment, address, identity, existing, resource, 1200)
}

fn approve_overlapping_window_for(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    existing: u64,
    resource: nickel_session_protocol::RemoteResourceId,
    duration_seconds: u64,
) -> Result<u64, String> {
    let scope = RemoteResourceScope::Window(resource);
    let requested = mcp_call(
        address,
        identity,
        "request_control_lease",
        json!({
            "scope": scope, "duration_seconds": duration_seconds, "allow_resumption": false, "full_debug": false
        }),
    )?;
    require_tool_success("overlapping client lease request", &requested)?;
    let pending = remote_snapshot(environment)?;
    if pending.active_leases.len() != 1
        || pending.active_leases[0].lease_id != existing
        || pending.pending_leases.len() != 1
    {
        return Err("overlap approval has unexpected authority".into());
    }
    let pending = &pending.pending_leases[0];
    if pending.client_id != identity.client_id || pending.request.scope != scope {
        return Err("overlap approval target changed".into());
    }
    let response = session_message(
        environment,
        Request::Command(Command::DecideRemoteLease {
            client_id: pending.client_id.clone(),
            pending_generation: pending.pending_generation,
            request: pending.request.clone(),
            allow: true,
        }),
    )?;
    let ServerMessage::RemoteControl(approved) = response else {
        return Err("overlap approval missing snapshot".into());
    };
    if approved.active_leases.len() != 2 || !approved.pending_leases.is_empty() {
        return Err("overlap approval did not create exactly two leases".into());
    }
    approved
        .active_leases
        .iter()
        .find(|lease| lease.lease_id != existing && lease.scope == scope)
        .map(|lease| lease.lease_id)
        .ok_or_else(|| "new overlap lease missing".into())
}

fn protocol_window_id(window: &Value) -> Result<nickel_session_protocol::WindowId, String> {
    window["id"]
        .as_str()
        .and_then(|id| id.parse::<u64>().ok())
        .map(nickel_session_protocol::WindowId)
        .ok_or_else(|| "invalid native window identity".into())
}

fn local_command(environment: &SessionEnvironment, command: Command) -> Result<(), String> {
    let response = session_message(environment, Request::Command(command))?;
    if response != ServerMessage::Ack {
        return Err(format!(
            "native movement command was not acknowledged: {response:?}"
        ));
    }
    Ok(())
}

fn verify_placement(
    environment: &SessionEnvironment,
    id: nickel_session_protocol::WindowId,
    output: &str,
    workspace: nickel_session_protocol::WorkspaceId,
) -> Result<(), String> {
    let ServerMessage::Windows(windows) =
        session_message(environment, Request::Query(Query::Windows))?
    else {
        return Err("native movement window observation unavailable".into());
    };
    let window = windows
        .iter()
        .find(|window| window.id == id)
        .ok_or("moved window missing")?;
    let geometry = window.geometry.ok_or("moved window geometry missing")?;
    let ServerMessage::Outputs(outputs) =
        session_message(environment, Request::Query(Query::Outputs))?
    else {
        return Err("native movement output observation unavailable".into());
    };
    let output = outputs
        .iter()
        .find(|candidate| candidate.name == output)
        .ok_or("movement output missing")?;
    let bounds = output.geometry;
    if window.workspace != workspace
        || geometry.x < bounds.x
        || geometry.y < bounds.y
        || i64::from(geometry.x) + i64::from(geometry.width)
            > i64::from(bounds.x) + i64::from(bounds.width)
        || i64::from(geometry.y) + i64::from(geometry.height)
            > i64::from(bounds.y) + i64::from(bounds.height)
    {
        return Err(
            "production owner did not confirm the requested output/workspace placement".into(),
        );
    }
    Ok(())
}

struct OrdinaryScope<'a> {
    environment: &'a SessionEnvironment,
    address: SocketAddr,
    identity: &'a Identity,
    lease: u64,
    scope: &'a RemoteResourceScope,
    approved: &'a RemoteControlSnapshot,
}
impl OrdinaryScope<'_> {
    fn pointer_and_capture(
        &self,
        window: &Value,
        client: &mut OrdinaryClient,
    ) -> Result<(), String> {
        scope_call(
            self.address,
            self.identity,
            "focus_window",
            window_arguments(self.lease, window),
        )?;
        let capture = mcp_call(
            self.address,
            self.identity,
            "capture_window",
            window_arguments(self.lease, window),
        )?;
        require_capture(
            "capture_window",
            &capture,
            "window_id",
            &native_resource(window, "id")?,
            Some((
                u32::try_from(window["width"].as_u64().ok_or("window width missing")?)
                    .map_err(|_| "window width exceeds u32")?,
                u32::try_from(window["height"].as_u64().ok_or("window height missing")?)
                    .map_err(|_| "window height exceeds u32")?,
            )),
        )?;
        let expected_actions = client.actions.saturating_add(1);
        let clicked = scope_call(
            self.address,
            self.identity,
            "pointer_action",
            json!({
                "lease_id": self.lease,
                "target": {
                    "kind": "window", "window_id": window["id"],
                    "generation": window["generation"]
                },
                "x": 360, "y": 136,
                "action": {"kind": "click", "button": "left"}
            }),
        )?;
        if clicked != true {
            return Err("ordinary pointer dispatch was not acknowledged".into());
        }
        client.wait_for_actions(expected_actions)?;
        require_unchanged_scope_approval(
            self.approved,
            &remote_snapshot(self.environment)?,
            self.lease,
            self.scope,
        )
    }

    fn input(
        &self,
        window: &Value,
        client: &mut OrdinaryClient,
        characters: &str,
    ) -> Result<(), String> {
        scope_call(
            self.address,
            self.identity,
            "focus_window",
            window_arguments(self.lease, window),
        )?;
        scope_call(
            self.address,
            self.identity,
            "pointer_action",
            json!({
                "lease_id": self.lease,
                "target": {
                    "kind": "window", "window_id": window["id"],
                    "generation": window["generation"]
                },
                "x": 360, "y": 76,
                "action": {"kind": "click", "button": "left"}
            }),
        )?;
        for character in characters.chars() {
            let target = json!({"lease_id": self.lease, "window_id": window["id"], "generation": window["generation"]});
            scope_call(self.address, self.identity, "focus_window", target.clone())?;
            let windows = scope_call(
                self.address,
                self.identity,
                "list_windows",
                json!({"lease_id": self.lease}),
            )?;
            let windows = windows
                .as_array()
                .ok_or("ordinary inventory is not an array")?;
            if !windows.iter().any(|current| {
                current["id"] == window["id"]
                    && current["generation"] == window["generation"]
                    && current["active"] == true
            }) {
                return Err("native focus did not reach the authorized ordinary window".into());
            }
            if matches!(self.scope, RemoteResourceScope::Window(_)) && windows.len() != 1 {
                return Err("window lease exposed additional ordinary windows".into());
            }
            let mut key = target;
            key["action"] = json!({"kind": "key", "keysym": u32::from(character), "modifiers": []});
            scope_call(self.address, self.identity, "keyboard_action", key)?;
            client.expected_text.push(character);
            client.wait_for_text()?;
            require_unchanged_scope_approval(
                self.approved,
                &remote_snapshot(self.environment)?,
                self.lease,
                self.scope,
            )?;
        }
        Ok(())
    }
}

fn deny_unrelated(
    environment: &SessionEnvironment,
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    window: &Value,
) -> Result<(), String> {
    // Keep the unrelated client live, focused and topmost so pointer and keyboard
    // denials cannot be explained by occlusion or focus preconditions.
    let numeric = window["id"]
        .as_str()
        .ok_or("missing unrelated window")?
        .parse::<u64>()
        .map_err(|_| "invalid unrelated window")?;
    session_message(
        environment,
        Request::Command(Command::WindowAction {
            window: nickel_session_protocol::WindowId(numeric),
            action: nickel_session_protocol::WindowAction::Activate,
        }),
    )?;
    let ServerMessage::Windows(windows) =
        session_message(environment, Request::Query(Query::Windows))?
    else {
        return Err("local window observation unavailable".into());
    };
    if !windows
        .iter()
        .any(|window| window.id.0 == numeric && window.active)
    {
        return Err("unrelated live client was not focused for the scope-denial test".into());
    }
    for operation in [
        "focus_window",
        "capture_window",
        "pointer_action",
        "keyboard_action",
    ] {
        thread::sleep(MATRIX_PACING);
        let mut arguments = json!({"lease_id": lease, "window_id": window["id"], "generation": window["generation"]});
        if operation == "keyboard_action" {
            arguments["action"] = json!({"kind": "key", "keysym": 120, "modifiers": []});
        } else if operation == "pointer_action" {
            arguments = json!({
                "lease_id": lease,
                "target": {
                    "kind": "window", "window_id": window["id"],
                    "generation": window["generation"]
                },
                "x": 1, "y": 1, "action": {"kind": "move"}
            });
        }
        let response = mcp_call(address, identity, operation, arguments)?;
        require_resource_boundary_error(operation, &response)?;
    }
    Ok(())
}

fn wait_for_window(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    existing: &[String],
    client: &mut OrdinaryClient,
) -> Result<Value, String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        client.require_running()?;
        let windows = scope_call(
            address,
            identity,
            "list_windows",
            json!({"lease_id": lease}),
        )?;
        let windows = windows
            .as_array()
            .ok_or("native window inventory is not an array")?;
        let mut found = windows.iter().filter(|window| {
            window["id"]
                .as_str()
                .is_some_and(|id| !existing.iter().any(|previous| previous == id))
                && window["verified_application"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        });
        if let Some(window) = found.next() {
            if found.next().is_some() {
                return Err("ordinary fixture window attribution is ambiguous".into());
            }
            native_resource(window, "id")?;
            return Ok(window.clone());
        }
        if Instant::now() >= deadline {
            return Err("owned ordinary fixture did not obtain native application evidence".into());
        }
        thread::sleep(POLL);
    }
}

fn wait_for_scoped_window(
    address: SocketAddr,
    identity: &Identity,
    lease: u64,
    existing: &[String],
) -> Result<Value, String> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let windows = scope_call(
            address,
            identity,
            "list_windows",
            json!({"lease_id": lease}),
        )?;
        let windows = windows
            .as_array()
            .ok_or("native window inventory is not an array")?;
        let mut found = windows.iter().filter(|window| {
            window["id"]
                .as_str()
                .is_some_and(|id| !existing.iter().any(|previous| previous == id))
                && window["verified_application"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        });
        if let Some(window) = found.next() {
            if found.next().is_some() {
                return Err("output-scoped launch attribution is ambiguous".into());
            }
            native_resource(window, "id")?;
            return Ok(window.clone());
        }
        if Instant::now() >= deadline {
            return Err("launched application did not map inside its authorized output".into());
        }
        thread::sleep(POLL);
    }
}

struct OrdinaryClient {
    child: Child,
    log: PathBuf,
    expected_text: String,
    actions: usize,
}

#[derive(Clone, Copy)]
enum FixtureBackend {
    Wayland,
    Xwayland,
}

impl FixtureBackend {
    fn require_x11_class(self, window: &Value) -> Result<(), String> {
        if matches!(self, Self::Xwayland)
            && window["application_id"].as_str().is_none_or(str::is_empty)
        {
            return Err("Xwayland fixture did not publish a WM_CLASS application identity".into());
        }
        Ok(())
    }
}

impl OrdinaryClient {
    fn spawn(
        environment: &SessionEnvironment,
        backend: FixtureBackend,
        example: &str,
        label: &str,
    ) -> Result<Self, String> {
        let harness = env::current_exe().map_err(|error| error.to_string())?;
        let directory = harness
            .parent()
            .ok_or("harness has no executable directory")?;
        let executable = directory.join("examples").join(example);
        if !executable.is_file() {
            return Err(format!(
                "missing repository example {example}; build nickel-ui --example {example}"
            ));
        }
        let log = environment.runtime.join(format!("ordinary-{label}.log"));
        let output = fs::File::create(&log).map_err(|error| error.to_string())?;
        let mut command = ProcessCommand::new(executable);
        match backend {
            FixtureBackend::Wayland => {
                command
                    .env("WINIT_UNIX_BACKEND", "wayland")
                    .env("WAYLAND_DISPLAY", &environment.wayland)
                    .env_remove("DISPLAY");
            }
            FixtureBackend::Xwayland => {
                let display = environment
                    .display
                    .as_deref()
                    .ok_or("nested Xwayland did not publish DISPLAY")?;
                command
                    .env("WINIT_UNIX_BACKEND", "x11")
                    .env("DISPLAY", display)
                    .env_remove("WAYLAND_DISPLAY");
            }
        }
        let child = command
            .env(
                "NICKEL_NATIVE_HOLD_RECEIPTS",
                if hold_receipts_enabled() { "1" } else { "0" },
            )
            .env("XDG_RUNTIME_DIR", &environment.runtime)
            .env("XDG_CONFIG_HOME", environment.runtime.join("config"))
            .env("XDG_STATE_HOME", environment.runtime.join("state"))
            .env("NICKEL_SESSION_CONTROL", &environment.control)
            .env("NICKEL_SESSION_TOKEN", &environment.token)
            .stdin(Stdio::null())
            .stdout(output.try_clone().map_err(|error| error.to_string())?)
            .stderr(output)
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            child,
            log,
            expected_text: String::new(),
            actions: 0,
        })
    }
    fn require_running(&mut self) -> Result<(), String> {
        if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("owned ordinary fixture exited with {status}"));
        }
        Ok(())
    }
    fn wait_for_text(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.require_running()?;
            let file = fs::File::open(&self.log).map_err(|error| error.to_string())?;
            let mut text = String::new();
            file.take(65537)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            if text.len() > 65536 {
                return Err("owned fixture output exceeded 64 KiB".into());
            }
            if text
                .lines()
                .any(|line| line == format!("recipient-text={}", self.expected_text))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "native key dispatch did not reach the recipient text field: {text}"
                ));
            }
            thread::sleep(POLL);
        }
    }

    fn wait_for_actions(&mut self, expected: usize) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.require_running()?;
            let mut text = String::new();
            fs::File::open(&self.log)
                .map_err(|error| error.to_string())?
                .take(65537)
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            if text.len() > 65536 {
                return Err("owned fixture output exceeded 64 KiB".into());
            }
            if text
                .lines()
                .any(|line| line == format!("recipient-actions={expected}"))
            {
                self.actions = expected;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "native pointer click did not activate the recipient button: {text}"
                ));
            }
            thread::sleep(POLL);
        }
    }

    fn hold_receipts(&mut self, kind: HeldKind) -> Result<(usize, usize), String> {
        self.require_running()?;
        let mut text = String::new();
        fs::File::open(&self.log)
            .map_err(|error| error.to_string())?
            .take(65537)
            .read_to_string(&mut text)
            .map_err(|error| error.to_string())?;
        if text.len() > 65536 {
            return Err("native hold receipt exceeded 64 KiB".into());
        }
        let pressed = format!("recipient-hold-{}=pressed", kind.marker());
        let released = format!("recipient-hold-{}=released", kind.marker());
        Ok((
            text.lines().filter(|line| *line == pressed).count(),
            text.lines().filter(|line| *line == released).count(),
        ))
    }

    fn wait_hold_receipts(
        &mut self,
        kind: HeldKind,
        expected: (usize, usize),
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let observed = self.hold_receipts(kind)?;
            if observed == expected {
                return Ok(());
            }
            if observed.0 > expected.0 || observed.1 > expected.1 || Instant::now() >= deadline {
                return Err(format!(
                    "native {} receipts {observed:?}, expected {expected:?}",
                    kind.marker()
                ));
            }
            thread::sleep(POLL);
        }
    }
}
impl Drop for OrdinaryClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
