//! Opt-in native ordinary-client acceptance using existing repository examples.
use super::*;

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
    revoke_scope(environment, bootstrap)?;

    let scope = RemoteResourceScope::Window(first_resource.clone());
    let lease = approve_scope(environment, address, identity, scope.clone(), false)?;
    let approved = remote_snapshot(environment)?;
    OrdinaryScope {
        environment,
        address,
        identity,
        lease,
        scope: &scope,
        approved: &approved,
    }
    .input(&first, &mut recipient, "abc")?;
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
