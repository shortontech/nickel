//! StatusNotifierWatcher for standalone Nickel sessions.
use std::sync::{Arc, Mutex};

use super::{STATUS_NOTIFIER_INTERFACE, STATUS_NOTIFIER_PATH, STATUS_NOTIFIER_WATCHER};

#[derive(Default)]
pub(super) struct WatcherState {
    items: Mutex<Vec<(String, String)>>,
}

struct Watcher {
    state: Arc<WatcherState>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Missing sender".into()))?
            .to_string();
        let (name, path) = if service.starts_with('/') {
            zbus::zvariant::ObjectPath::try_from(service)
                .map_err(|_| zbus::fdo::Error::InvalidArgs("Invalid item path".into()))?;
            (sender.as_str(), service)
        } else {
            (service, "/StatusNotifierItem")
        };
        let name = zbus::names::BusName::try_from(name)
            .map_err(|_| zbus::fdo::Error::InvalidArgs("Invalid item service".into()))?;
        let dbus = zbus::fdo::DBusProxy::new(connection).await?;
        let owner = dbus.get_name_owner(name).await?.to_string();
        if owner != sender {
            return Err(zbus::fdo::Error::AccessDenied(
                "Item service belongs to another connection".into(),
            ));
        }
        let id = format!(
            "{service_name}{path}",
            service_name = if service.starts_with('/') {
                sender.as_str()
            } else {
                service
            }
        );
        let added = {
            let mut items = self
                .state
                .items
                .lock()
                .map_err(|_| zbus::fdo::Error::Failed("Watcher state unavailable".into()))?;
            if items.iter().any(|(existing, _)| existing == &id) {
                false
            } else {
                if items.len() >= 128 {
                    return Err(zbus::fdo::Error::LimitsExceeded(
                        "Tray registration limit reached".into(),
                    ));
                }
                items.push((id.clone(), owner));
                true
            }
        };
        if added {
            Self::status_notifier_item_registered(&emitter, &id).await?;
            self.registered_status_notifier_items_changed(&emitter)
                .await?;
        }
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        // Nickel itself is always the host while this watcher owns its name.
        Self::status_notifier_host_registered(&emitter).await?;
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state
            .items
            .lock()
            .map(|items| items.iter().map(|(id, _)| id.clone()).collect())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_host_registered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;
}

pub(super) fn install(
    connection: &zbus::blocking::Connection,
    state: Arc<WatcherState>,
) -> zbus::Result<()> {
    connection
        .object_server()
        .at(STATUS_NOTIFIER_PATH, Watcher { state })?;
    Ok(())
}

pub(super) fn claim(connection: &zbus::blocking::Connection) -> bool {
    matches!(
        connection.request_name_with_flags(
            STATUS_NOTIFIER_WATCHER,
            zbus::fdo::RequestNameFlags::DoNotQueue.into()
        ),
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner)
    )
}

pub(super) fn prune(
    connection: &zbus::blocking::Connection,
    state: &WatcherState,
    dbus: &zbus::blocking::fdo::DBusProxy<'_>,
) {
    let snapshot = state
        .items
        .lock()
        .map(|items| items.clone())
        .unwrap_or_default();
    for (id, owner) in snapshot {
        let (service, _) = super::item_address(&id);
        let alive = zbus::names::BusName::try_from(service)
            .ok()
            .and_then(|name| dbus.get_name_owner(name).ok())
            .is_some_and(|current| current.as_str() == owner);
        if !alive {
            let removed = state
                .items
                .lock()
                .map(|mut items| {
                    let before = items.len();
                    items.retain(|item| item != &(id.clone(), owner.clone()));
                    items.len() != before
                })
                .unwrap_or(false);
            if removed {
                let _ = connection.emit_signal(
                    None::<&str>,
                    STATUS_NOTIFIER_PATH,
                    STATUS_NOTIFIER_INTERFACE,
                    "StatusNotifierItemUnregistered",
                    &(id.as_str(),),
                );
                let values = state
                    .items
                    .lock()
                    .map(|items| items.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>())
                    .unwrap_or_default();
                let changed = std::collections::HashMap::from([(
                    "RegisteredStatusNotifierItems",
                    zbus::zvariant::Value::from(values),
                )]);
                let _ = connection.emit_signal(
                    None::<&str>,
                    STATUS_NOTIFIER_PATH,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                    &(STATUS_NOTIFIER_INTERFACE, changed, Vec::<String>::new()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn tray_watcher_registers_validates_deduplicates_and_removes_departed_items() {
        struct Bus(std::process::Child);
        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut bus = Bus(std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap());
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let connect = || {
            zbus::blocking::connection::Builder::address(address.trim())
                .unwrap()
                .build()
                .unwrap()
        };
        let external = connect();
        external.request_name(STATUS_NOTIFIER_WATCHER).unwrap();
        let host = connect();
        let state = Arc::new(WatcherState::default());
        install(&host, state.clone()).unwrap();
        assert!(!claim(&host), "must respect an existing watcher");
        external.release_name(STATUS_NOTIFIER_WATCHER).unwrap();
        assert!(
            claim(&host),
            "must take over when the external watcher leaves"
        );
        let client = connect();
        client.request_name("org.nickel.TestTray").unwrap();
        let proxy = zbus::blocking::Proxy::new(
            &client,
            STATUS_NOTIFIER_WATCHER,
            STATUS_NOTIFIER_PATH,
            STATUS_NOTIFIER_INTERFACE,
        )
        .unwrap();
        proxy
            .call_method("RegisterStatusNotifierItem", &("org.nickel.TestTray",))
            .unwrap();
        proxy
            .call_method("RegisterStatusNotifierItem", &("org.nickel.TestTray",))
            .unwrap();
        proxy
            .call_method("RegisterStatusNotifierItem", &("/CustomItem",))
            .unwrap();
        let items = proxy
            .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "org.nickel.TestTray/StatusNotifierItem");
        assert_eq!(
            items[1],
            format!("{}/CustomItem", client.unique_name().unwrap())
        );
        assert!(
            proxy
                .get_property::<bool>("IsStatusNotifierHostRegistered")
                .unwrap()
        );
        assert_eq!(proxy.get_property::<i32>("ProtocolVersion").unwrap(), 0);
        let impostor = connect();
        let impostor_proxy = zbus::blocking::Proxy::new(
            &impostor,
            STATUS_NOTIFIER_WATCHER,
            STATUS_NOTIFIER_PATH,
            STATUS_NOTIFIER_INTERFACE,
        )
        .unwrap();
        assert!(
            impostor_proxy
                .call_method("RegisterStatusNotifierItem", &("org.nickel.TestTray",))
                .is_err()
        );
        assert!(
            proxy
                .call_method("RegisterStatusNotifierItem", &("/invalid path",))
                .is_err()
        );
        client.release_name("org.nickel.TestTray").unwrap();
        let dbus = zbus::blocking::fdo::DBusProxy::new(&host).unwrap();
        prune(&host, &state, &dbus);
        assert_eq!(state.items.lock().unwrap().len(), 1);
        drop(proxy);
        drop(client);
        for _ in 0..100 {
            prune(&host, &state, &dbus);
            if state.items.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.items.lock().unwrap().is_empty());
    }
}
