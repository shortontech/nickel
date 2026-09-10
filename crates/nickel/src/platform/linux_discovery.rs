//! Sender-owned BlueZ discovery. Closing this dedicated sender only releases
//! Nickel's own session; it never stops another client's discovery session.
use super::linux_control::{BLUEZ, managed_bluez_objects, service_owner};
use super::linux_guarded_control::{GuardedControlOrigin, GuardedControlOutcome as Outcome};
use nickel_platform::bounded_dbus::{self, GuardedSender};
use nickel_remote_control::DesktopPermit;
use std::{
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use zbus::{blocking::Connection, export::futures_core::Stream};

struct MaterialSignals {
    owner: String,
    adapter: String,
    streams: Mutex<Vec<zbus::MessageStream>>,
}
impl MaterialSignals {
    fn subscribe(
        connection: &Connection,
        owner: &str,
        adapter: &str,
        check: impl Fn() -> bool,
    ) -> Option<Self> {
        let rules = [
            format!(
                "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='{owner}'"
            ),
            format!(
                "type='signal',sender='{owner}',path='{adapter}',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',arg0='org.bluez.Adapter1'"
            ),
            format!(
                "type='signal',sender='{owner}',path='/',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesRemoved'"
            ),
        ];
        let mut streams = Vec::with_capacity(3);
        for rule in rules {
            streams.push(
                bounded_future(
                    zbus::MessageStream::for_match_rule(rule.as_str(), connection.inner(), Some(4)),
                    &check,
                )?
                .ok()?,
            );
        }
        Some(Self {
            owner: owner.into(),
            adapter: adapter.into(),
            streams: Mutex::new(streams),
        })
    }
    fn material(&self, message: &zbus::Message) -> bool {
        let header = message.header();
        if message.message_type() != zbus::message::Type::Signal {
            return false;
        }
        let sender = header.sender().map(|value| value.as_str());
        match (
            header.interface().map(|value| value.as_str()),
            header.member().map(|value| value.as_str()),
        ) {
            (Some("org.freedesktop.DBus"), Some("NameOwnerChanged"))
                if sender == Some("org.freedesktop.DBus") =>
            {
                message
                    .body()
                    .deserialize::<(String, String, String)>()
                    .map_or(true, |(name, _, new)| name == self.owner && new.is_empty())
            }
            (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged"))
                if sender == Some(self.owner.as_str())
                    && header
                        .path()
                        .is_some_and(|path| path.as_str() == self.adapter) =>
            {
                type Changes = (
                    String,
                    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
                    Vec<String>,
                );
                message.body().deserialize::<Changes>().map_or(
                    true,
                    |(interface, changes, invalidated)| {
                        interface == "org.bluez.Adapter1"
                            && (["Powered", "Discovering"].iter().any(|key| {
                                changes
                                    .get(*key)
                                    .is_some_and(|value| bool::try_from(value).ok() != Some(true))
                            }) || invalidated
                                .iter()
                                .any(|key| key == "Powered" || key == "Discovering"))
                    },
                )
            }
            (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesRemoved"))
                if sender == Some(self.owner.as_str()) =>
            {
                message
                    .body()
                    .deserialize::<(zbus::zvariant::OwnedObjectPath, Vec<String>)>()
                    .map_or(true, |(path, interfaces)| {
                        path.as_str() == self.adapter
                            && interfaces
                                .iter()
                                .any(|interface| interface == "org.bluez.Adapter1")
                    })
            }
            _ => false,
        }
    }
    fn retire_if_changed(&self, sender: &GuardedSender, origin: &GuardedControlOrigin) -> bool {
        let changed = match self.streams.try_lock() {
            Err(_) => true,
            Ok(mut streams) => streams.iter_mut().any(|stream| {
                for _ in 0..8 {
                    match Pin::new(&mut *stream).poll_next(&mut Context::from_waker(Waker::noop()))
                    {
                        Poll::Ready(Some(Ok(message))) if self.material(&message) => return true,
                        Poll::Ready(Some(Ok(_))) => {}
                        Poll::Ready(_) => return true,
                        Poll::Pending => return false,
                    }
                }
                // A saturated material channel cannot establish continuity.
                true
            }),
        };
        if changed {
            origin.set_standing(false);
            sender.retire();
        }
        changed
    }
}

struct Session {
    connection: Connection,
    sender: GuardedSender,
    owner: String,
    adapter: String,
    permit: DesktopPermit,
    origin: GuardedControlOrigin,
    material: MaterialSignals,
}
impl Session {
    fn retire(&self) {
        self.origin.set_standing(false);
        self.sender.retire();
    }
    fn live(&self) -> bool {
        if self.origin.check().is_err()
            || self.permit.check_standing_live().is_err()
            || self.sender.is_retired()
            || self.connection.inner().is_closed()
        {
            return false;
        }
        !self.material.retire_if_changed(&self.sender, &self.origin)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.retire();
    }
}
const CAPACITY: usize = 16;
type SessionRegistry = Arc<Mutex<Vec<Arc<Session>>>>;
static SESSIONS: OnceLock<Result<SessionRegistry, ()>> = OnceLock::new();
fn sessions() -> Result<&'static SessionRegistry, ()> {
    SESSIONS
        .get_or_init(|| {
            start_monitor(|task| {
                std::thread::Builder::new()
                    .name("nickel-discovery-cleanup".into())
                    .spawn(task)
                    .map(|_| ())
                    .map_err(|_| ())
            })
        })
        .as_ref()
        .map_err(|_| ())
}
fn start_monitor(
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), ()>,
) -> Result<SessionRegistry, ()> {
    let sessions = Arc::new(Mutex::new(Vec::<Arc<Session>>::new()));
    let monitored = Arc::clone(&sessions);
    spawn(Box::new(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            // No native roundtrip or blocking authority lock in this monitor.
            let Ok(mut sessions) = monitored.try_lock() else {
                continue;
            };
            sessions.retain(|session| {
                let live = session.live();
                if !live {
                    session.retire();
                }
                live
            });
        }
    }))?;
    Ok(sessions)
}
fn confirmed_reply(reply: &zbus::Message, serial: std::num::NonZeroU32, owner: &str) -> bool {
    reply.header().reply_serial() == Some(serial)
        && reply
            .header()
            .sender()
            .is_some_and(|sender| sender.as_str() == owner)
        && reply.message_type() == zbus::message::Type::MethodReturn
        && reply.body().is_empty()
        && reply.body().deserialize::<()>().is_ok()
}
fn evidence() -> nickel_remote_control::leases::ResourceEvidence<'static> {
    nickel_remote_control::leases::ResourceEvidence {
        surface: None,
        window: None,
        verified_application: None,
        output: None,
        authorized_surface_ancestors: &[],
        protected: false,
    }
}
fn bounded_future<F: std::future::Future>(
    future: F,
    check: impl Fn() -> bool,
) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + Duration::from_millis(300);
    loop {
        if Instant::now() >= deadline || !check() {
            return None;
        }
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => return Some(value),
            Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
        }
    }
}
fn invoke(
    session: &Session,
    start: bool,
    permit: &DesktopPermit,
    origin: &GuardedControlOrigin,
) -> Outcome {
    let method = if start {
        "StartDiscovery"
    } else {
        "StopDiscovery"
    };
    let Ok(message) = zbus::Message::method_call(session.adapter.as_str(), method)
        .and_then(|builder| builder.destination(session.owner.as_str()))
        .and_then(|builder| builder.interface("org.bluez.Adapter1"))
        .and_then(|builder| builder.build(&()))
    else {
        return Outcome::Unavailable;
    };
    let serial = message.primary_header().serial_num();
    let mut replies = zbus::MessageStream::from(session.connection.inner());
    let mut attempted = false;
    let mut not_accepted = false;
    let result = permit.with_input_boundary(&evidence(), |boundary| {
        permit.check_commit_boundary(boundary)?;
        origin.check()?;
        attempted = true;
        session
            .sender
            .send_guarded(&message, || {
                permit.check_commit_boundary(boundary)?;
                origin.check()
            })
            .map_err(|error| {
                not_accepted =
                    error == nickel_platform::bounded_dbus::GuardedSendError::NotAccepted;
                error.to_string()
            })
    });
    if result.is_err() {
        return if not_accepted {
            Outcome::Unavailable
        } else if attempted {
            Outcome::Uncertain
        } else {
            Outcome::Cancelled
        };
    }
    let deadline = Instant::now() + Duration::from_millis(300);
    for _ in 0..32 {
        loop {
            if Instant::now() >= deadline || permit.check_live().is_err() || origin.check().is_err()
            {
                return Outcome::Uncertain;
            }
            match Pin::new(&mut replies).poll_next(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Some(Ok(reply))) => {
                    if reply.header().reply_serial() == Some(serial)
                        && reply
                            .header()
                            .sender()
                            .is_some_and(|sender| sender.as_str() == session.owner)
                    {
                        return if confirmed_reply(&reply, serial, &session.owner) {
                            Outcome::Confirmed
                        } else if reply.message_type() == zbus::message::Type::Error {
                            Outcome::Unavailable
                        } else {
                            Outcome::Uncertain
                        };
                    }
                    break;
                }
                Poll::Ready(_) => return Outcome::Uncertain,
                Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
            }
        }
    }
    Outcome::Uncertain
}
pub(super) fn execute(start: bool, permit: DesktopPermit, origin: GuardedControlOrigin) -> Outcome {
    if origin.check().is_err() || permit.with_resource(&evidence(), || Ok(())).is_err() {
        return Outcome::Cancelled;
    }
    let Ok(registry) = sessions() else {
        return Outcome::Unavailable;
    };
    if !start {
        let session = {
            let Ok(mut sessions) = registry.lock() else {
                return Outcome::Unavailable;
            };
            let Some(index) = sessions
                .iter()
                .position(|session| session.permit.same_lease_as(&permit))
            else {
                return Outcome::Confirmed; // This lease owns no discovery sender.
            };
            sessions.remove(index)
        };
        if !session.live() {
            session.retire();
            return Outcome::Cancelled;
        }
        let outcome = invoke(&session, false, &permit, &origin);
        session.retire();
        return outcome;
    }
    if registry.lock().map_or(true, |sessions| {
        sessions.len() >= CAPACITY
            && !sessions
                .iter()
                .any(|session| session.permit.same_lease_as(&permit))
    }) {
        return Outcome::Unavailable;
    }
    let Ok(address) = zbus::Address::system() else {
        return Outcome::Unavailable;
    };
    let Ok((connection, sender)) = bounded_dbus::connect_guarded_blocking(
        address,
        bounded_dbus::Limits::ACCESSIBILITY,
        Duration::from_millis(300),
    ) else {
        return Outcome::Unavailable;
    };
    let Some(owner) = service_owner(&connection, BLUEZ) else {
        return Outcome::Unavailable;
    };
    let Ok(objects) = managed_bluez_objects(&connection) else {
        return Outcome::Unavailable;
    };
    if objects.len() > 128 {
        return Outcome::Unavailable;
    }
    let Some(adapter) = objects
        .into_iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path)
    else {
        return Outcome::Unavailable;
    };
    let Some(material) = MaterialSignals::subscribe(&connection, &owner, adapter.as_str(), || {
        permit.check_live().is_ok() && origin.check().is_ok()
    }) else {
        return Outcome::Unavailable;
    };
    if service_owner(&connection, BLUEZ).as_deref() != Some(owner.as_str()) {
        return Outcome::Unavailable;
    }
    let session = Arc::new(Session {
        connection,
        sender,
        owner,
        adapter: adapter.to_string(),
        permit,
        origin,
        material,
    });
    let result = invoke(&session, true, &session.permit, &session.origin);
    if result != Outcome::Confirmed {
        return result;
    }
    // Standing ownership is established only after this sender's MethodReturn.
    if !session.live() {
        return Outcome::Uncertain;
    }
    let Ok(mut sessions) = registry.lock() else {
        return Outcome::Uncertain;
    };
    sessions.retain(|old| {
        if old.permit.same_lease_as(&session.permit) {
            old.retire();
            false
        } else {
            true
        }
    });
    if sessions.len() >= CAPACITY {
        return Outcome::Uncertain;
    }
    session.origin.set_standing(true);
    sessions.push(session);
    Outcome::DiscoveryStarted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn failed_monitor_spawn_admits_no_registry() {
        let mut attempted = false;
        assert!(
            start_monitor(|_task| {
                attempted = true;
                Err(())
            })
            .is_err()
        );
        assert!(attempted);
    }
    #[test]
    fn discovery_reply_requires_pinned_sender_and_empty_body() {
        let call = zbus::Message::method_call("/org/bluez/hci0", "StartDiscovery")
            .unwrap()
            .build(&())
            .unwrap();
        let serial = call.primary_header().serial_num();
        let reply = |sender: &str, body: &str| {
            zbus::Message::method_return(&call.header())
                .unwrap()
                .sender(sender)
                .unwrap()
                .build(&body)
                .unwrap()
        };
        assert!(!confirmed_reply(&reply(":1.99", ""), serial, ":1.10"));
        assert!(!confirmed_reply(
            &reply(":1.10", "unexpected"),
            serial,
            ":1.10"
        ));
        let valid = zbus::Message::method_return(&call.header())
            .unwrap()
            .sender(":1.10")
            .unwrap()
            .build(&())
            .unwrap();
        assert!(confirmed_reply(&valid, serial, ":1.10"));
    }
    struct PrivateBluez(Arc<Mutex<HashSet<String>>>);
    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl PrivateBluez {
        #[zbus(property)]
        fn powered(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn discovering(&self) -> bool {
            !self.0.lock().unwrap().is_empty()
        }
        fn start_discovery(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
            self.0
                .lock()
                .unwrap()
                .insert(header.sender().unwrap().to_string());
        }
        fn stop_discovery(
            &self,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<()> {
            if self
                .0
                .lock()
                .unwrap()
                .remove(header.sender().unwrap().as_str())
            {
                Ok(())
            } else {
                Err(zbus::fdo::Error::Failed("No discovery started".into()))
            }
        }
    }
    /// A bounded reusable service fixture for the compositor/MCP native harness.
    #[test]
    #[ignore = "explicit owned BlueZ service harness mode only"]
    fn private_bluez_fixture_service_for_mcp() {
        if std::env::var("NICKEL_TEST_BLUEZ_FIXTURE_SERVICE").as_deref() != Ok("1") {
            return;
        }
        let address = std::env::var("NICKEL_TEST_GUARDED_DBUS_ADDRESS").unwrap();
        assert!(address.starts_with("unix:path=/tmp/nickel-guarded-standing/"));
        let owners = Arc::new(Mutex::new(HashSet::new()));
        let service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name(BLUEZ)
            .unwrap()
            .serve_at("/", zbus::fdo::ObjectManager)
            .unwrap()
            .serve_at("/org/bluez/hci0", PrivateBluez(Arc::clone(&owners)))
            .unwrap()
            .build()
            .unwrap();
        let mut changes = bounded_future(zbus::MessageStream::for_match_rule(
            "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged'",
            service.inner(), Some(32)), || true).unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            for _ in 0..32 {
                match Pin::new(&mut changes).poll_next(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Some(Ok(message))) => {
                        let (name, _old, new): (String, String, String) =
                            message.body().deserialize().unwrap();
                        if new.is_empty() {
                            owners.lock().unwrap().remove(&name);
                        }
                    }
                    Poll::Ready(_) => return,
                    Poll::Pending => break,
                }
            }
            std::fs::write(
                "/tmp/nickel-guarded-standing/session-count",
                owners.lock().unwrap().len().to_string(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore = "requires explicitly owned private DBus daemon"]
    fn private_bluez_discovery_belongs_to_sender_and_disconnect_releases_only_it() {
        let address = std::env::var("NICKEL_TEST_GUARDED_DBUS_ADDRESS").unwrap();
        assert!(address.starts_with("unix:path=/tmp/nickel-guarded-"));
        let owners = Arc::new(Mutex::new(HashSet::new()));
        let service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name(BLUEZ)
            .unwrap()
            .serve_at("/org/bluez/hci0", PrivateBluez(Arc::clone(&owners)))
            .unwrap()
            .build()
            .unwrap();
        let mut changes = bounded_future(zbus::MessageStream::for_match_rule(
            "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged'",
            service.inner(), Some(32)), || true).unwrap().unwrap();
        let connect = || {
            bounded_dbus::connect_guarded_blocking(
                address.parse().unwrap(),
                bounded_dbus::Limits::ACCESSIBILITY,
                Duration::from_millis(300),
            )
            .unwrap()
        };
        let (first, first_sender) = connect();
        let (second, second_sender) = connect();
        // A different private peer can know the serial, but cannot supply the
        // pinned BlueZ sender identity. Its actual bus-delivered reply is denied.
        let call = zbus::Message::method_call("/org/bluez/hci0", "StartDiscovery")
            .unwrap()
            .sender(first.unique_name().unwrap().as_str())
            .unwrap()
            .build(&())
            .unwrap();
        let serial = call.primary_header().serial_num();
        let mut forged_replies = zbus::MessageStream::from(first.inner());
        let forged = zbus::Message::method_return(&call.header())
            .unwrap()
            .build(&())
            .unwrap();
        second.send(&forged).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            assert!(Instant::now() < deadline);
            match Pin::new(&mut forged_replies).poll_next(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Some(Ok(reply))) if reply.header().reply_serial() == Some(serial) => {
                    assert_eq!(
                        reply.header().sender().unwrap().as_str(),
                        second.unique_name().unwrap().as_str()
                    );
                    assert!(!confirmed_reply(
                        &reply,
                        serial,
                        service.unique_name().unwrap().as_str()
                    ));
                    break;
                }
                Poll::Ready(Some(Ok(_))) => {}
                Poll::Ready(_) => panic!("forged private reply stream closed"),
                Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
            }
        }
        drop(forged_replies);
        let invoke = |connection: &Connection, sender: &GuardedSender, method: &str| {
            let message = zbus::Message::method_call("/org/bluez/hci0", method)
                .unwrap()
                .destination(BLUEZ)
                .unwrap()
                .interface("org.bluez.Adapter1")
                .unwrap()
                .build(&())
                .unwrap();
            let serial = message.primary_header().serial_num();
            let mut replies = zbus::MessageStream::from(connection.inner());
            sender.send_guarded(&message, || Ok(())).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                assert!(Instant::now() < deadline);
                match Pin::new(&mut replies).poll_next(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Some(Ok(reply)))
                        if reply.header().reply_serial() == Some(serial) =>
                    {
                        return reply.message_type();
                    }
                    Poll::Ready(Some(Ok(_))) => {}
                    Poll::Ready(_) => panic!("private reply stream closed"),
                    Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
                }
            }
        };
        assert_eq!(
            invoke(&first, &first_sender, "StartDiscovery"),
            zbus::message::Type::MethodReturn
        );
        assert_eq!(
            invoke(&second, &second_sender, "StopDiscovery"),
            zbus::message::Type::Error
        );
        assert_eq!(owners.lock().unwrap().len(), 1);
        assert_eq!(
            invoke(&second, &second_sender, "StartDiscovery"),
            zbus::message::Type::MethodReturn
        );
        assert_eq!(owners.lock().unwrap().len(), 2);
        assert_eq!(
            invoke(&first, &first_sender, "StopDiscovery"),
            zbus::message::Type::MethodReturn
        );
        assert_eq!(owners.lock().unwrap().len(), 1);
        assert_eq!(
            invoke(&first, &first_sender, "StartDiscovery"),
            zbus::message::Type::MethodReturn
        );
        first_sender.retire();
        let first_name = first.unique_name().unwrap().to_string();
        let deadline = Instant::now() + Duration::from_secs(1);
        // BlueZ's disconnect watcher removes the exact vanished bus sender.
        while owners.lock().unwrap().contains(&first_name) {
            assert!(Instant::now() < deadline);
            match Pin::new(&mut changes).poll_next(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(Some(Ok(message))) => {
                    let (name, _old, new): (String, String, String) =
                        message.body().deserialize().unwrap();
                    if new.is_empty() {
                        owners.lock().unwrap().remove(&name);
                    }
                }
                Poll::Ready(_) => panic!("private disconnect watcher closed"),
                Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
            }
        }
        assert!(
            owners
                .lock()
                .unwrap()
                .contains(second.unique_name().unwrap().as_str())
        );
        for event in ["Powered", "Discovering", "Removed"] {
            let (client, sender) = connect();
            let origin_owner =
                super::super::linux_guarded_control::GuardedControlOriginOwner::new();
            let origin = origin_owner.ticket();
            let material = MaterialSignals::subscribe(
                &client,
                service.unique_name().unwrap().as_str(),
                "/org/bluez/hci0",
                || true,
            )
            .unwrap();
            assert_eq!(
                invoke(&client, &sender, "StartDiscovery"),
                zbus::message::Type::MethodReturn
            );
            origin.set_standing(true);
            assert!(origin_owner.has_standing_session());
            let emit_property = |connection: &Connection, path: &str, property: &str| {
                let values = std::collections::HashMap::from([(
                    property.to_string(),
                    zbus::zvariant::Value::from(false),
                )]);
                connection
                    .emit_signal(
                        None::<&str>,
                        path,
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        &("org.bluez.Adapter1", values, Vec::<String>::new()),
                    )
                    .unwrap();
            };
            // Wrong adapter and wrong sender cannot retire accepted ownership.
            emit_property(&service, "/org/bluez/hci9", "Powered");
            emit_property(&second, "/org/bluez/hci0", "Powered");
            std::thread::sleep(Duration::from_millis(20));
            assert!(!material.retire_if_changed(&sender, &origin));
            assert!(origin_owner.has_standing_session());
            if event == "Removed" {
                let path = zbus::zvariant::OwnedObjectPath::try_from("/org/bluez/hci0").unwrap();
                service
                    .emit_signal(
                        None::<&str>,
                        "/",
                        "org.freedesktop.DBus.ObjectManager",
                        "InterfacesRemoved",
                        &(path, vec!["org.bluez.Adapter1"]),
                    )
                    .unwrap();
            } else {
                emit_property(&service, "/org/bluez/hci0", event);
            }
            let deadline = Instant::now() + Duration::from_secs(1);
            while !material.retire_if_changed(&sender, &origin) {
                assert!(Instant::now() < deadline);
                std::thread::park_timeout(Duration::from_millis(1));
            }
            assert!(!origin_owner.has_standing_session());
            assert!(sender.is_retired());
            let name = client.unique_name().unwrap().to_string();
            while owners.lock().unwrap().contains(&name) {
                assert!(Instant::now() < deadline);
                match Pin::new(&mut changes).poll_next(&mut Context::from_waker(Waker::noop())) {
                    Poll::Ready(Some(Ok(message))) => {
                        let (name, _, new): (String, String, String) =
                            message.body().deserialize().unwrap();
                        if new.is_empty() {
                            owners.lock().unwrap().remove(&name);
                        }
                    }
                    Poll::Ready(_) => panic!("private material watcher closed"),
                    Poll::Pending => std::thread::park_timeout(Duration::from_millis(1)),
                }
            }
            assert!(
                owners
                    .lock()
                    .unwrap()
                    .contains(second.unique_name().unwrap().as_str())
            );
        }
        assert_eq!(
            invoke(&second, &second_sender, "StopDiscovery"),
            zbus::message::Type::MethodReturn
        );
        assert!(owners.lock().unwrap().is_empty());
    }
}
