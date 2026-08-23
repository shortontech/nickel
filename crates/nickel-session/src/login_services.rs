use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use zbus::{
    blocking::{Connection, Proxy, connection::Builder, fdo::DBusProxy},
    names::WellKnownName,
    zvariant::{OwnedObjectPath, OwnedValue},
};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const READY_CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SecureStorageState {
    Starting,
    Locked,
    PromptRequired,
    Ready,
    Unavailable,
}

impl SecureStorageState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Locked => "locked",
            Self::PromptRequired => "prompt-required",
            Self::Ready => "ready",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecureStorageError {
    #[error("could not connect to the user D-Bus session: {0}")]
    Connect(#[source] zbus::Error),
    #[error("Secret Service operation failed: {0}")]
    Protocol(#[from] zbus::Error),
    #[error("D-Bus service operation failed: {0}")]
    Bus(#[from] zbus::fdo::Error),
    #[error("Secret Service has no default collection; refusing to create a replacement")]
    MissingDefaultCollection,
    #[error("the Secret Service unlock prompt was dismissed")]
    PromptDismissed,
    #[error("the Secret Service unlock prompt timed out")]
    PromptTimedOut,
    #[error("the Secret Service disappeared")]
    ProviderDisappeared,
    #[error("Secret Service reported that the default collection remained locked")]
    RemainedLocked,
    #[error("could not read Secret Service provider configuration: {0}")]
    ProviderConfiguration(#[source] std::io::Error),
    #[error("configured Secret Service provider {expected} does not match bus owner {actual}")]
    UnexpectedProvider { expected: String, actual: String },
}

pub fn monitor_secure_storage(
    retry_requested: Arc<AtomicBool>,
    mut publish: impl FnMut(SecureStorageState),
) -> ! {
    loop {
        retry_requested.store(false, Ordering::Release);
        publish(SecureStorageState::Starting);
        match connect() {
            Ok(connection) => match prepare_secure_storage(&connection, &mut publish) {
                Ok(()) => monitor_ready_provider(&connection, &mut publish),
                Err(SecureStorageError::PromptDismissed) => {
                    publish(SecureStorageState::Locked);
                    tracing::info!("Secret Service prompt dismissed; waiting for retry");
                }
                Err(error) => {
                    publish(SecureStorageState::Unavailable);
                    tracing::error!(%error, "secure storage unavailable");
                }
            },
            Err(error) => {
                publish(SecureStorageState::Unavailable);
                tracing::error!(%error, "secure storage unavailable");
            }
        }
        wait_for_retry(&retry_requested);
    }
}

fn wait_for_retry(retry_requested: &AtomicBool) {
    while !retry_requested.swap(false, Ordering::AcqRel) {
        thread::sleep(RETRY_DELAY);
    }
}

fn connect() -> Result<Connection, SecureStorageError> {
    Builder::session()
        .map_err(SecureStorageError::Connect)?
        .method_timeout(METHOD_TIMEOUT)
        .build()
        .map_err(SecureStorageError::Connect)
}

fn prepare_secure_storage(
    connection: &Connection,
    publish: &mut impl FnMut(SecureStorageState),
) -> Result<(), SecureStorageError> {
    activate_provider(connection)?;
    verify_provider_identity(connection, configured_provider()?.as_deref())?;
    let collection = default_collection(connection)?;
    if collection_is_locked(connection, &collection)? {
        publish(SecureStorageState::Locked);
        unlock_collection(connection, &collection, publish)?;
    }
    if collection_is_locked(connection, &collection)? {
        return Err(SecureStorageError::RemainedLocked);
    }
    publish(SecureStorageState::Ready);
    Ok(())
}

fn configured_provider() -> Result<Option<PathBuf>, SecureStorageError> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let Some(path) = config_home.map(|home| home.join("nickel/secret-service-provider")) else {
        return Ok(None);
    };
    let value = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SecureStorageError::ProviderConfiguration(error)),
    };
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(value)))
    }
}

fn verify_provider_identity(
    connection: &Connection,
    expected: Option<&Path>,
) -> Result<(), SecureStorageError> {
    let dbus = DBusProxy::new(connection)?;
    let owner = dbus.get_name_owner(SERVICE.try_into().expect("valid bus name"))?;
    let Some(expected) = expected else {
        tracing::info!(
            owner = owner.as_str(),
            "connected to unpinned Secret Service provider"
        );
        return Ok(());
    };
    let pid = dbus.get_connection_unix_process_id(owner.clone().into())?;
    let actual = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(SecureStorageError::ProviderConfiguration)?;
    let expected =
        std::fs::canonicalize(expected).map_err(SecureStorageError::ProviderConfiguration)?;
    if actual != expected {
        return Err(SecureStorageError::UnexpectedProvider {
            expected: expected.display().to_string(),
            actual: actual.display().to_string(),
        });
    }
    Ok(())
}

fn activate_provider(connection: &Connection) -> Result<(), SecureStorageError> {
    let dbus = DBusProxy::new(connection)?;
    if !dbus.name_has_owner(SERVICE.try_into().expect("valid bus name"))? {
        dbus.start_service_by_name(
            WellKnownName::try_from(SERVICE).expect("valid well-known bus name"),
            0,
        )?;
    }
    if dbus.name_has_owner(SERVICE.try_into().expect("valid bus name"))? {
        Ok(())
    } else {
        Err(SecureStorageError::ProviderDisappeared)
    }
}

fn default_collection(connection: &Connection) -> Result<OwnedObjectPath, SecureStorageError> {
    let proxy = Proxy::new(connection, SERVICE, SERVICE_PATH, SERVICE_INTERFACE)?;
    let collection: OwnedObjectPath = proxy.call("ReadAlias", &("default",))?;
    if collection.as_str() == "/" {
        Err(SecureStorageError::MissingDefaultCollection)
    } else {
        Ok(collection)
    }
}

fn collection_is_locked(
    connection: &Connection,
    collection: &OwnedObjectPath,
) -> Result<bool, SecureStorageError> {
    let proxy = Proxy::new(
        connection,
        SERVICE,
        collection.as_str(),
        COLLECTION_INTERFACE,
    )?;
    proxy.get_property("Locked").map_err(Into::into)
}

fn unlock_collection(
    connection: &Connection,
    collection: &OwnedObjectPath,
    publish: &mut impl FnMut(SecureStorageState),
) -> Result<(), SecureStorageError> {
    let proxy = Proxy::new(connection, SERVICE, SERVICE_PATH, SERVICE_INTERFACE)?;
    let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        proxy.call("Unlock", &(vec![collection.clone()],))?;
    if unlocked.iter().any(|path| path == collection) || prompt.as_str() == "/" {
        return Ok(());
    }

    publish(SecureStorageState::PromptRequired);
    complete_prompt(connection, &prompt)
}

fn complete_prompt(
    connection: &Connection,
    prompt_path: &OwnedObjectPath,
) -> Result<(), SecureStorageError> {
    let connection = connection.clone();
    let prompt_path = prompt_path.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("nickel-secret-prompt".into())
        .spawn(move || {
            let result = (|| {
                let proxy =
                    Proxy::new(&connection, SERVICE, prompt_path.as_str(), PROMPT_INTERFACE)?;
                let mut completed = proxy.receive_signal("Completed")?;
                proxy.call::<_, _, ()>("Prompt", &("",))?;
                let message = completed
                    .next()
                    .ok_or(SecureStorageError::ProviderDisappeared)?;
                let (dismissed, _result): (bool, OwnedValue) = message.body().deserialize()?;
                if dismissed {
                    Err(SecureStorageError::PromptDismissed)
                } else {
                    Ok(())
                }
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| SecureStorageError::Connect(zbus::Error::InputOutput(error.into())))?;

    receiver
        .recv_timeout(PROMPT_TIMEOUT)
        .map_err(|_| SecureStorageError::PromptTimedOut)?
}

fn monitor_ready_provider(connection: &Connection, publish: &mut impl FnMut(SecureStorageState)) {
    let dbus = match DBusProxy::new(connection) {
        Ok(proxy) => proxy,
        Err(error) => {
            publish(SecureStorageState::Unavailable);
            tracing::error!(%error, "could not monitor Secret Service owner");
            return;
        }
    };

    loop {
        thread::sleep(READY_CHECK_INTERVAL);
        match dbus.name_has_owner(SERVICE.try_into().expect("valid bus name")) {
            Ok(true) => {}
            Ok(false) => {
                publish(SecureStorageState::Unavailable);
                tracing::warn!("Secret Service owner disappeared");
                return;
            }
            Err(error) => {
                publish(SecureStorageState::Unavailable);
                tracing::error!(%error, "could not monitor Secret Service owner");
                return;
            }
        }

        match configured_provider().and_then(|expected| {
            if expected.is_some() {
                verify_provider_identity(connection, expected.as_deref())
            } else {
                Ok(())
            }
        }) {
            Ok(()) => {}
            Err(error) => {
                publish(SecureStorageState::Unavailable);
                tracing::error!(%error, "Secret Service provider identity changed");
                return;
            }
        }

        match default_collection(connection)
            .and_then(|collection| collection_is_locked(connection, &collection))
        {
            Ok(false) => {}
            Ok(true) => {
                publish(SecureStorageState::Locked);
                return;
            }
            Err(error) => {
                publish(SecureStorageState::Unavailable);
                tracing::error!(%error, "Secret Service readiness check failed");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use zbus::{
        blocking::{Connection, connection::Builder},
        object_server::SignalEmitter,
        zvariant::{ObjectPath, OwnedObjectPath, OwnedValue},
    };

    use super::{SecureStorageError, SecureStorageState, prepare_secure_storage};

    const COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/login";
    const PROMPT_PATH: &str = "/org/freedesktop/secrets/prompt/unlock";

    #[derive(Clone, Copy)]
    enum UnlockBehavior {
        Immediate,
        PromptSuccess,
        PromptDismissed,
    }

    struct MockService {
        missing_default: bool,
        locked: Arc<AtomicBool>,
        behavior: UnlockBehavior,
    }

    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl MockService {
        fn read_alias(&self, _name: &str) -> ObjectPath<'_> {
            if self.missing_default {
                ObjectPath::from_static_str_unchecked("/")
            } else {
                ObjectPath::from_static_str_unchecked(COLLECTION_PATH)
            }
        }

        fn unlock(&self, _objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, ObjectPath<'_>) {
            match self.behavior {
                UnlockBehavior::Immediate => {
                    self.locked.store(false, Ordering::Release);
                    (
                        vec![OwnedObjectPath::try_from(COLLECTION_PATH).unwrap()],
                        ObjectPath::from_static_str_unchecked("/"),
                    )
                }
                UnlockBehavior::PromptSuccess | UnlockBehavior::PromptDismissed => (
                    Vec::new(),
                    ObjectPath::from_static_str_unchecked(PROMPT_PATH),
                ),
            }
        }
    }

    struct MockCollection {
        locked: Arc<AtomicBool>,
    }

    struct SlowService;

    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl SlowService {
        fn read_alias(&self, _name: &str) -> ObjectPath<'_> {
            std::thread::sleep(Duration::from_millis(200));
            ObjectPath::from_static_str_unchecked(COLLECTION_PATH)
        }
    }

    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl MockCollection {
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.locked.load(Ordering::Acquire)
        }
    }

    struct MockPrompt {
        locked: Arc<AtomicBool>,
        dismissed: bool,
    }

    #[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
    impl MockPrompt {
        async fn prompt(
            &self,
            _window_id: &str,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) {
            if !self.dismissed {
                self.locked.store(false, Ordering::Release);
            }
            Self::completed(&emitter, self.dismissed, OwnedValue::from(0_u32))
                .await
                .unwrap();
        }

        #[zbus(signal)]
        async fn completed(
            emitter: &SignalEmitter<'_>,
            dismissed: bool,
            result: OwnedValue,
        ) -> zbus::Result<()>;
    }

    struct PrivateBus {
        child: Child,
        address: String,
    }

    impl PrivateBus {
        fn start() -> Self {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("start private dbus-daemon");
            let mut address = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut address)
                .unwrap();
            Self {
                child,
                address: address.trim().to_owned(),
            }
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn mock_connections(
        initially_locked: bool,
        missing_default: bool,
        behavior: UnlockBehavior,
    ) -> (PrivateBus, Connection, Connection, Arc<AtomicBool>) {
        let bus = PrivateBus::start();
        let locked = Arc::new(AtomicBool::new(initially_locked));
        let dismissed = matches!(behavior, UnlockBehavior::PromptDismissed);
        let service = Builder::address(bus.address.as_str())
            .unwrap()
            .name(super::SERVICE)
            .unwrap()
            .serve_at(
                super::SERVICE_PATH,
                MockService {
                    missing_default,
                    locked: Arc::clone(&locked),
                    behavior,
                },
            )
            .unwrap()
            .serve_at(
                COLLECTION_PATH,
                MockCollection {
                    locked: Arc::clone(&locked),
                },
            )
            .unwrap()
            .serve_at(
                PROMPT_PATH,
                MockPrompt {
                    locked: Arc::clone(&locked),
                    dismissed,
                },
            )
            .unwrap()
            .build()
            .unwrap();
        let client = Builder::address(bus.address.as_str())
            .unwrap()
            .method_timeout(super::METHOD_TIMEOUT)
            .build()
            .unwrap();
        (bus, service, client, locked)
    }

    #[test]
    fn readiness_states_are_distinct() {
        assert_ne!(SecureStorageState::Starting, SecureStorageState::Ready);
        assert_ne!(
            SecureStorageState::Locked,
            SecureStorageState::PromptRequired
        );
        assert_ne!(SecureStorageState::Ready, SecureStorageState::Unavailable);
    }

    #[test]
    fn unlocked_default_collection_reaches_ready() {
        let (_bus, _service, client, _locked) =
            mock_connections(false, false, UnlockBehavior::Immediate);
        let mut states = Vec::new();
        prepare_secure_storage(&client, &mut |state| states.push(state)).unwrap();
        assert_eq!(states, [SecureStorageState::Ready]);
    }

    #[test]
    fn locked_collection_can_unlock_without_prompt() {
        let (_bus, _service, client, _locked) =
            mock_connections(true, false, UnlockBehavior::Immediate);
        let mut states = Vec::new();
        prepare_secure_storage(&client, &mut |state| states.push(state)).unwrap();
        assert_eq!(
            states,
            [SecureStorageState::Locked, SecureStorageState::Ready]
        );
    }

    #[test]
    fn prompt_completion_is_observed_before_ready() {
        let (_bus, _service, client, _locked) =
            mock_connections(true, false, UnlockBehavior::PromptSuccess);
        let mut states = Vec::new();
        prepare_secure_storage(&client, &mut |state| states.push(state)).unwrap();
        assert_eq!(
            states,
            [
                SecureStorageState::Locked,
                SecureStorageState::PromptRequired,
                SecureStorageState::Ready,
            ]
        );
    }

    #[test]
    fn dismissed_prompt_does_not_reach_ready() {
        let (_bus, _service, client, _locked) =
            mock_connections(true, false, UnlockBehavior::PromptDismissed);
        let mut states = Vec::new();
        let error = prepare_secure_storage(&client, &mut |state| states.push(state)).unwrap_err();
        assert!(matches!(error, SecureStorageError::PromptDismissed));
        assert_eq!(
            states,
            [
                SecureStorageState::Locked,
                SecureStorageState::PromptRequired,
            ]
        );
    }

    #[test]
    fn missing_default_collection_is_not_created() {
        let (_bus, _service, client, _locked) =
            mock_connections(false, true, UnlockBehavior::Immediate);
        let error = prepare_secure_storage(&client, &mut |_| {}).unwrap_err();
        assert!(matches!(
            error,
            SecureStorageError::MissingDefaultCollection
        ));
    }

    #[test]
    fn configured_provider_identity_accepts_matching_executable() {
        let (_bus, _service, client, _locked) =
            mock_connections(false, false, UnlockBehavior::Immediate);
        let executable = std::env::current_exe().unwrap();
        super::verify_provider_identity(&client, Some(&executable)).unwrap();
    }

    #[test]
    fn configured_provider_identity_rejects_replacement_executable() {
        let (_bus, _service, client, _locked) =
            mock_connections(false, false, UnlockBehavior::Immediate);
        let error =
            super::verify_provider_identity(&client, Some(std::path::Path::new("/bin/false")))
                .unwrap_err();
        assert!(matches!(
            error,
            SecureStorageError::UnexpectedProvider { .. }
        ));
    }

    #[test]
    fn provider_exit_revokes_ready_state() {
        let (_bus, service, client, _locked) =
            mock_connections(false, false, UnlockBehavior::Immediate);
        drop(service);
        let mut states = Vec::new();
        super::monitor_ready_provider(&client, &mut |state| states.push(state));
        assert_eq!(states, [SecureStorageState::Unavailable]);
    }

    #[test]
    fn provider_relock_revokes_ready_state() {
        let (_bus, _service, client, locked) =
            mock_connections(false, false, UnlockBehavior::Immediate);
        locked.store(true, Ordering::Release);
        let mut states = Vec::new();
        super::monitor_ready_provider(&client, &mut |state| states.push(state));
        assert_eq!(states, [SecureStorageState::Locked]);
    }

    #[test]
    fn missing_provider_activation_fails_without_substitution() {
        let bus = PrivateBus::start();
        let client = Builder::address(bus.address.as_str())
            .unwrap()
            .method_timeout(super::METHOD_TIMEOUT)
            .build()
            .unwrap();
        let error = prepare_secure_storage(&client, &mut |_| {}).unwrap_err();
        assert!(matches!(
            error,
            SecureStorageError::Bus(_) | SecureStorageError::ProviderDisappeared
        ));
    }

    #[test]
    fn slow_provider_is_bounded_by_method_timeout() {
        let bus = PrivateBus::start();
        let _service = Builder::address(bus.address.as_str())
            .unwrap()
            .name(super::SERVICE)
            .unwrap()
            .serve_at(super::SERVICE_PATH, SlowService)
            .unwrap()
            .build()
            .unwrap();
        let client = Builder::address(bus.address.as_str())
            .unwrap()
            .method_timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        assert!(prepare_secure_storage(&client, &mut |_| {}).is_err());
        assert!(started.elapsed() < Duration::from_millis(175));
    }

    #[test]
    fn retry_wait_consumes_an_explicit_request() {
        let retry = AtomicBool::new(true);
        super::wait_for_retry(&retry);
        assert!(!retry.load(Ordering::Acquire));
    }
}
