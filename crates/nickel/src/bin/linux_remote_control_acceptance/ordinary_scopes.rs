//! Opt-in native ordinary-client acceptance using existing repository examples.
use super::*;

pub(super) fn movement_enabled() -> bool {
    env::args().any(|argument| argument == "--movement")
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
    session_message(
        environment,
        Request::Command(Command::SetLauncherVisible { visible: false }),
    )?;
    let mut recipient = OrdinaryClient::spawn(environment, "keyboard_recipient", "first")?;
    let first = wait_for_window(address, identity, bootstrap, &[], &mut recipient)?;
    if first["title"] != "Keyboard recipient acceptance" {
        return Err("owned keyboard fixture published an unexpected window".into());
    }
    // Title identifies the expected fixture only. Authority comes exclusively
    // from the production owner's native process/executable evidence.
    let application = first["verified_application"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("native owner could not verify the keyboard fixture application")?
        .to_owned();
    let first_resource = native_resource(&first, "id")?;
    let mut unrelated = OrdinaryClient::spawn(environment, "standalone", "unrelated")?;
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
    if let Some(movement) = &movement {
        movement.follow(&scope_test, &first, &mut recipient)?;
    }
    deny_unrelated(environment, address, identity, lease, &other)?;
    require_unchanged_scope_approval(&approved, &remote_snapshot(environment)?, lease, &scope)?;
    println!(
        "PASS: native ordinary window scope, one approval, repeated focus/key delivery confirmed by the real client, unrelated window denied"
    );
    revoke_scope(environment, lease)?;

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
    if let Some(movement) = &movement {
        movement.follow(&scope_test, &first, &mut recipient)?;
        movement.deny_counter_launch(&scope_test)?;
    }
    // A later process/window must inherit through verified executable identity;
    // it did not exist when the application lease was approved.
    let mut second = OrdinaryClient::spawn(environment, "keyboard_recipient", "second")?;
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
        "PASS: native application scope, one approval, repeated client-confirmed input, later same-executable window inherited authority, unrelated executable denied"
    );
    revoke_scope(environment, lease)?;
    if let Some(movement) = &movement {
        movement.output_boundary(environment, address, identity, &first, &mut recipient)?;
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

struct Movement {
    primary: nickel_session_protocol::RemoteResourceId,
    original_workspace: nickel_session_protocol::WorkspaceId,
    second_workspace: nickel_session_protocol::WorkspaceId,
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
                    scale_120: 120,
                    transform: nickel_session_protocol::OutputTransform::Normal,
                },
            },
        )?;
        let outputs = scope_call(
            address,
            identity,
            "list_outputs",
            json!({"lease_id": lease}),
        )?;
        if !outputs["outputs"].as_array().is_some_and(|outputs| {
            outputs
                .iter()
                .any(|output| output["name"] == MOVEMENT_OUTPUT)
        }) {
            return Err("nested output was not published through the production owner".into());
        }
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
            original_workspace,
            second_workspace,
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
    fn input(
        &self,
        window: &Value,
        client: &mut OrdinaryClient,
        characters: &str,
    ) -> Result<(), String> {
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
    for operation in ["focus_window", "capture_window", "keyboard_action"] {
        thread::sleep(MATRIX_PACING);
        let mut arguments = json!({"lease_id": lease, "window_id": window["id"], "generation": window["generation"]});
        if operation == "keyboard_action" {
            // Remove the focus precondition as an alternative reason for denial.
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
                return Err(
                    "unrelated live client was not focused for the scope-denial test".into(),
                );
            }
            arguments["action"] = json!({"kind": "key", "keysym": 120, "modifiers": []});
        }
        let response = mcp_call(address, identity, operation, arguments)?;
        require_tool_error(operation, &response)?;
        if !response
            .to_string()
            .contains("outside the resource boundary")
        {
            return Err(format!(
                "{operation} failed for a reason other than scope: {response}"
            ));
        }
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

struct OrdinaryClient {
    child: Child,
    log: PathBuf,
    expected_text: String,
}
impl OrdinaryClient {
    fn spawn(environment: &SessionEnvironment, example: &str, label: &str) -> Result<Self, String> {
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
        let child = ProcessCommand::new(executable)
            .env("WINIT_UNIX_BACKEND", "wayland")
            .env("WAYLAND_DISPLAY", &environment.wayland)
            .env("XDG_RUNTIME_DIR", &environment.runtime)
            .env("XDG_CONFIG_HOME", environment.runtime.join("config"))
            .env("XDG_STATE_HOME", environment.runtime.join("state"))
            .env("NICKEL_SESSION_CONTROL", &environment.control)
            .env("NICKEL_SESSION_TOKEN", &environment.token)
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(output.try_clone().map_err(|error| error.to_string())?)
            .stderr(output)
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            child,
            log,
            expected_text: String::new(),
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
}
impl Drop for OrdinaryClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
