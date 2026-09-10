use super::{Observation, Proof};
use nickel_platform::bounded_dbus::{Limits, bounded_socket};
use nickel_remote_control::{
    DesktopPermit,
    native_semantics::{NativeSemanticNode, NativeSemanticScope, NativeSemanticSnapshot},
};
use std::{
    collections::{HashSet, VecDeque},
    future::Future,
    time::{Duration, Instant},
};
use zbus::{
    Connection, Message,
    address::{Address, Transport, transport::UnixSocket},
    zvariant::{OwnedObjectPath, OwnedValue},
};

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;
const MAX_CALLS: usize = 512;
const APPLICATION_ROOT: &str = "/org/a11y/atspi/accessible/root";
// Stable AT-SPI wire values, not toolkit-specific GTK role enum values.
const PASSWORD: u32 = 40;
const DIALOG: u32 = 16;
const FRAME: u32 = 23;
const WINDOW: u32 = 69;
const DEFUNCT: u32 = 6;
const SHOWING: u32 = 25;
const VISIBLE: u32 = 30;

async fn bounded<T>(
    future: impl Future<Output = Result<T, String>>,
    deadline: Instant,
) -> Result<T, String> {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(100));
    futures_lite::future::race(future, async {
        async_io::Timer::after(remaining).await;
        Err("native accessibility query timed out".into())
    })
    .await
}

struct Query<'a, F> {
    proof: &'a Proof,
    peer: String,
    permit: &'a DesktopPermit,
    deadline: Instant,
    validate: F,
    calls: usize,
}
impl<F: Fn() -> Result<(), String>> Query<'_, F> {
    fn check(&mut self) -> Result<(), String> {
        if self.calls >= MAX_CALLS || Instant::now() >= self.deadline {
            return Err("native accessibility observation exceeded bounds".into());
        }
        (self.validate)()?;
        self.proof.check_live(self.permit)?;
        if Instant::now() >= self.deadline {
            return Err("native accessibility deadline elapsed".into());
        }
        self.calls += 1;
        Ok(())
    }
    async fn connect(&mut self, address: Address) -> Result<Connection, String> {
        self.check()?;
        // Never execute autolaunch/exec transports or connect to arbitrary TCP hosts.
        let Transport::Unix(unix) = address.transport() else {
            return Err("accessibility requires a local Unix bus".into());
        };
        let UnixSocket::File(path) = unix.path() else {
            return Err("accessibility bus address is unsupported".into());
        };
        bounded(
            async {
                let stream = async_io::Async::<std::os::unix::net::UnixStream>::connect(path)
                    .await
                    .map_err(|_| "accessibility bus connection failed")?;
                zbus::connection::Builder::socket(bounded_socket(stream, Limits::ACCESSIBILITY))
                    .max_queued(8)
                    .method_timeout(Duration::from_millis(100))
                    .build()
                    .await
                    .map_err(|_| "accessibility bus authentication failed".into())
            },
            self.deadline,
        )
        .await
    }
    async fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(
        &mut self,
        connection: &Connection,
        destination: &str,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<Message, String> {
        self.check()?;
        bounded(
            async {
                connection
                    .call_method(Some(destination), path, Some(interface), method, body)
                    .await
                    .map_err(|_| "native accessibility provider query failed".into())
            },
            self.deadline,
        )
        .await
    }
    async fn provider<B: serde::Serialize + zbus::zvariant::DynamicType>(
        &mut self,
        connection: &Connection,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<Message, String> {
        let peer = self.peer.clone();
        self.call(connection, &peer, path, interface, method, body)
            .await
    }
    async fn authenticate_peer(&mut self, bus: &Connection, peer: &str) -> Result<bool, String> {
        if peer.len() > 255 || zbus::names::UniqueName::try_from(peer).is_err() {
            return Ok(false);
        }
        for (method, expected) in [
            ("GetConnectionUnixProcessID", self.proof.process.pid()),
            ("GetConnectionUnixUser", self.proof.uid),
        ] {
            let actual: u32 = self
                .call(
                    bus,
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    method,
                    &(peer,),
                )
                .await?
                .body()
                .deserialize()
                .map_err(|_| "accessibility bus peer credentials unavailable")?;
            if actual != expected {
                return Ok(false);
            }
        }
        Ok(true)
    }
    async fn property(
        &mut self,
        connection: &Connection,
        path: &str,
        property: &str,
    ) -> Result<OwnedValue, String> {
        // Intentionally no proxy cache/GetAll: that would collect protected string/value properties.
        self.provider(
            connection,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(ACCESSIBLE, property),
        )
        .await?
        .body()
        .deserialize()
        .map_err(|_| "invalid accessibility property".into())
    }
    async fn role(&mut self, connection: &Connection, path: &str) -> Result<u32, String> {
        self.provider(connection, path, ACCESSIBLE, "GetRole", &())
            .await?
            .body()
            .deserialize()
            .map_err(|_| "invalid accessibility role".into())
    }
    async fn states(&mut self, connection: &Connection, path: &str) -> Result<[u32; 2], String> {
        let states: Vec<u32> = self
            .provider(connection, path, ACCESSIBLE, "GetState", &())
            .await?
            .body()
            .deserialize()
            .map_err(|_| "invalid accessibility state")?;
        states
            .try_into()
            .map_err(|_| "invalid accessibility state length".into())
    }
}

fn state(states: &[u32; 2], bit: u32) -> bool {
    states[(bit / 32) as usize] & (1 << (bit % 32)) != 0
}
fn visible(states: &[u32; 2]) -> bool {
    !state(states, DEFUNCT) && state(states, SHOWING) && state(states, VISIBLE)
}
fn reference(value: OwnedValue) -> Result<(String, OwnedObjectPath), String> {
    <(String, OwnedObjectPath)>::try_from(value)
        .map_err(|_| "invalid accessibility object reference".into())
}

pub(in crate::session) fn observe<F: Fn() -> Result<(), String>>(
    proof: &Proof,
    permit: &DesktopPermit,
    deadline: Instant,
    validate: F,
) -> Result<Observation, String> {
    let started = Instant::now();
    async_io::block_on(async {
        let mut query = Query {
            proof,
            peer: proof
                .binding
                .as_ref()
                .map(|binding| binding.peer.clone())
                .unwrap_or_default(),
            permit,
            deadline,
            validate,
            calls: 0,
        };
        let session = query
            .connect(Address::session().map_err(|_| "session bus unavailable")?)
            .await?;
        let address: String = query
            .call(
                &session,
                "org.a11y.Bus",
                "/org/a11y/bus",
                "org.a11y.Bus",
                "GetAddress",
                &(),
            )
            .await?
            .body()
            .deserialize()
            .map_err(|_| "accessibility bus unavailable")?;
        if address.len() > 4096 {
            return Err("accessibility bus address exceeds bounds".into());
        }
        let bus = query
            .connect(
                Address::try_from(address.as_str())
                    .map_err(|_| "invalid accessibility bus address")?,
            )
            .await?;
        let application_root = proof.binding.is_none();
        let root = if let Some(binding) = &proof.binding {
            if !query.authenticate_peer(&bus, &binding.peer).await? {
                return Err("accessibility bus peer does not match native surface owner".into());
            }
            binding.root.clone()
        } else {
            // Only broker-owned identity metadata is read before a unique peer's
            // OS incarnation is matched to the owner-authorized application.
            let names: Vec<String> = query
                .call(
                    &bus,
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "ListNames",
                    &(),
                )
                .await?
                .body()
                .deserialize()
                .map_err(|_| "accessibility peer inventory unavailable")?;
            let mut found = false;
            for peer in names
                .into_iter()
                .filter(|name| name.starts_with(':'))
                .take(128)
            {
                if !query.authenticate_peer(&bus, &peer).await? {
                    continue;
                }
                query.peer = peer;
                if query.role(&bus, APPLICATION_ROOT).await.ok() == Some(75) {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err("authenticated application accessibility root unavailable".into());
            }
            APPLICATION_ROOT.to_owned()
        };
        let mut queue = VecDeque::from([(root, None, APPLICATION_ROOT.to_owned(), 0usize)]);
        let mut seen = HashSet::new();
        let mut nodes = Vec::new();
        let mut evidence = Vec::new();
        let mut truncated = application_root; // Other connections of the process are not aggregated.
        while let Some((path, parent, parent_path, depth)) = queue.pop_front() {
            // Reserve calls for the final classification recheck as well as this node.
            if nodes.len() >= MAX_NODES || query.calls + evidence.len() * 3 + 10 >= MAX_CALLS {
                truncated = true;
                break;
            }
            if !seen.insert(path.clone()) {
                return Err("accessibility tree contains a cycle".into());
            }
            let role = query.role(&bus, &path).await?;
            // Legacy GTK labels password entries as generic TEXT. Conservatively
            // omit every text-entry/terminal subtree, even when currently plain.
            if matches!(role, PASSWORD | 60 | 61 | 77 | 79) {
                if parent.is_none() {
                    return Err("native accessibility root is protected".into());
                }
                continue;
            }
            if parent.is_some()
                && (matches!(role, DIALOG | 2)
                    || (matches!(role, FRAME | WINDOW) && !(application_root && depth == 1)))
            {
                // Never walk through an embedded reference into another top-level window.
                continue;
            }
            let states = query.states(&bus, &path).await?;
            if state(&states, 7) {
                continue;
            } // Editable metadata is not protection-stable.
            if state(&states, DEFUNCT)
                || !(visible(&states) || application_root && parent.is_none())
            {
                if parent.is_none() {
                    return Err("native accessibility root is hidden or retired".into());
                }
                continue;
            }
            if !(application_root && parent.is_none()) {
                let (peer, actual_parent) =
                    reference(query.property(&bus, &path, "Parent").await?)?;
                if peer != query.peer || actual_parent.as_str() != parent_path {
                    return Err("accessibility object is outside associated window tree".into());
                }
            }
            let bounds = if application_root {
                None
            } else {
                let bounds = query
                    .provider(
                        &bus,
                        &path,
                        "org.a11y.atspi.Component",
                        "GetExtents",
                        &(1u32,),
                    )
                    .await;
                match bounds {
                    Ok(message) => {
                        let (x, y, w, h): (i32, i32, i32, i32) = message
                            .body()
                            .deserialize()
                            .map_err(|_| "invalid accessibility bounds")?;
                        (w >= 0
                            && h >= 0
                            && x >= 0
                            && y >= 0
                            && i64::from(x) + i64::from(w) <= i64::from(proof.geometry[2])
                            && i64::from(y) + i64::from(h) <= i64::from(proof.geometry[3]))
                        .then_some([x, y, w, h])
                    }
                    Err(_) => {
                        proof.check_live(permit)?;
                        None
                    }
                }
            };
            let id = nodes.len() as u32;
            nodes.push(NativeSemanticNode {
                id,
                parent,
                role,
                bounds,
                enabled: state(&states, 8),
                focused: state(&states, 12),
            });
            evidence.push((path.clone(), role, parent_path));
            let count = i32::try_from(query.property(&bus, &path, "ChildCount").await?)
                .map_err(|_| "invalid accessibility child count")?;
            if count < 0 {
                return Err("invalid accessibility child count".into());
            }
            if depth >= MAX_DEPTH {
                truncated |= count > 0;
                continue;
            }
            let room = MAX_NODES.saturating_sub(nodes.len() + queue.len());
            let take = (count as usize)
                .min(room)
                .min(MAX_CALLS.saturating_sub(query.calls + evidence.len() * 3 + 10));
            truncated |= take < count as usize;
            for index in 0..take {
                let (peer, child): (String, OwnedObjectPath) = query
                    .provider(&bus, &path, ACCESSIBLE, "GetChildAtIndex", &(index as i32,))
                    .await?
                    .body()
                    .deserialize()
                    .map_err(|_| "invalid accessibility child")?;
                if peer != query.peer
                    || child.as_str().len() > 512
                    || child.as_str() == APPLICATION_ROOT
                {
                    return Err("accessibility child is outside associated native peer".into());
                }
                queue.push_back((child.to_string(), Some(id), path.clone(), depth + 1));
            }
        }
        // Defensive lifetime/classification check, NOT atomic text-safety evidence.
        // No names, descriptions, editable values or text were queried at any point.
        for (path, role, parent_path) in evidence {
            if !(application_root && path == APPLICATION_ROOT) {
                let (peer, actual_parent) =
                    reference(query.property(&bus, &path, "Parent").await?)?;
                if peer != query.peer || actual_parent.as_str() != parent_path {
                    return Err("native accessibility ancestry changed during observation".into());
                }
            }
            let states = query.states(&bus, &path).await?;
            if query.role(&bus, &path).await? != role
                || state(&states, DEFUNCT)
                || !(visible(&states) || application_root && path == APPLICATION_ROOT)
                || state(&states, 7)
            {
                return Err("native accessibility tree changed during observation".into());
            }
        }
        let completed = Instant::now();
        (query.validate)()?;
        proof.check_live(permit)?;
        let mut result = NativeSemanticSnapshot {
            scope: if application_root {
                NativeSemanticScope::ApplicationConnection
            } else {
                NativeSemanticScope::Window
            },
            window: proof.id.to_string(),
            window_generation: proof.id,
            association_generation: proof.binding.as_ref().map(|binding| binding.generation),
            observation_generation: 0,
            observed_at_us: 0,
            observation_started_at_us: 0,
            owner_validated_at_us: 0,
            atomic: false,
            unavailable_fields: [
                "name",
                "description",
                "text",
                "value",
                "actions",
                "editable_subtrees",
            ]
            .map(str::to_owned)
            .to_vec(),
            nodes,
            truncated,
        };
        if application_root {
            result.unavailable_fields.push("geometry".into());
        }
        if serde_json::to_vec(&result)
            .map_err(|_| "native accessibility encoding failed")?
            .len()
            > 64 * 1024
        {
            return Err("native accessibility result exceeds bounds".into());
        }
        Ok(Observation {
            snapshot: result,
            started,
            completed,
        })
    })
}
