use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};

use pipewire_native::{
    self as pipewire,
    context::Context,
    core::{Core, CoreEvents},
    main_loop::MainLoop,
    properties::Properties,
    proxy::{
        HasProxy, ProxyEvents,
        metadata::{Metadata, MetadataEvents},
        node::{Node, NodeEvents},
        registry::RegistryEvents,
    },
    some_closure, types,
};
use pipewire_native_spa as spa;
use spa::{
    param::{ParamType, props::Prop},
    pod::{
        parser::Parser,
        types::{ObjectType, PropertyFlags},
    },
};

use super::super::{
    AUDIO_DEVICE_LIMIT, AudioDeviceStatus, AudioRefresh, AudioStatus, bound_audio_refresh,
};

#[derive(Debug)]
enum AudioCommand {
    SetVolume(u8),
    AdjustVolume(i8),
    ToggleMute,
    SetMuted(bool),
    SelectOutput(String),
    Refresh(mpsc::SyncSender<Result<AudioRefresh, String>>),
}

struct AudioBackend {
    snapshot: Arc<RwLock<AudioStatus>>,
    commands: mpsc::Sender<AudioCommand>,
    subscribers: Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
}

#[derive(Clone)]
struct Sink {
    node: Node,
    name: String,
    serial: Option<u64>,
    description: String,
    exposed: bool,
    driver_id: Option<u32>,
    channel_volumes: Vec<f32>,
    volume_observed: bool,
    muted: bool,
    mute_observed: bool,
}

#[derive(Default)]
struct Graph {
    sinks: HashMap<u32, Sink>,
    default_name: Option<String>,
    metadata: Option<Metadata>,
    metadata_id: Option<u32>,
    proxy_admissions: usize,
    invalid_inventory: bool,
    cookie: Option<u32>,
}

// Count lifetime admissions, not current map occupancy: the native Core may
// retain removed proxies. Never recycle this budget within a guarded connection.
fn admit_guarded_proxy(graph: &Mutex<Graph>, metadata: bool) -> bool {
    let Ok(mut graph) = graph.lock() else {
        return false;
    };
    if graph.invalid_inventory
        || graph.proxy_admissions >= 129
        || (metadata && graph.metadata_id.is_some())
    {
        graph.invalid_inventory = true;
        return false;
    }
    graph.proxy_admissions += 1;
    true
}

static BACKEND: OnceLock<AudioBackend> = OnceLock::new();

pub fn status() -> AudioStatus {
    backend()
        .snapshot
        .read()
        .map(|status| status.clone())
        .unwrap_or_default()
}

pub fn set_volume(volume: u8) -> bool {
    backend()
        .commands
        .send(AudioCommand::SetVolume(volume.min(100)))
        .is_ok()
}

pub fn adjust_volume(delta: i8) -> bool {
    backend()
        .commands
        .send(AudioCommand::AdjustVolume(delta))
        .is_ok()
}

pub fn toggle_mute() -> bool {
    backend().commands.send(AudioCommand::ToggleMute).is_ok()
}

pub fn select_output(id: &str) -> bool {
    backend()
        .commands
        .send(AudioCommand::SelectOutput(id.to_owned()))
        .is_ok()
}

pub fn refresh() -> Result<AudioRefresh, String> {
    let (reply, response) = mpsc::sync_channel(1);
    backend()
        .commands
        .send(AudioCommand::Refresh(reply))
        .map_err(|_| "audio worker stopped".to_owned())?;
    response
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "audio refresh timed out".to_owned())?
}

pub fn subscribe() -> crate::platform::status_mailbox::StatusReceiver {
    let (_, mut receiver) = crate::platform::status_mailbox::channel();
    subscribe_into(&mut receiver);
    receiver
}

pub fn subscribe_into(receiver: &mut crate::platform::status_mailbox::StatusReceiver) {
    let backend = backend();
    let sender = receiver.sender();
    if let Ok(mut subscribers) = backend.subscribers.lock() {
        subscribers.push(sender.clone());
    }
    if let Ok(status) = backend.snapshot.read() {
        let _ = sender.send(Arc::new(crate::platform::SystemStatusUpdate::Audio(
            status.clone(),
        )));
    }
    let subscribers = Arc::clone(&backend.subscribers);
    receiver.on_drop(move || {
        subscribers
            .lock()
            .unwrap()
            .retain(|entry| !entry.same_channel(&sender))
    });
}

fn backend() -> &'static AudioBackend {
    BACKEND.get_or_init(|| {
        let snapshot = Arc::new(RwLock::new(AudioStatus::default()));
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let (commands, receiver) = mpsc::channel();
        let worker_snapshot = Arc::clone(&snapshot);
        let worker_subscribers = Arc::clone(&subscribers);
        let _ = thread::Builder::new()
            .name("nickel-pipewire".into())
            .spawn(move || audio_worker(worker_snapshot, worker_subscribers, receiver));
        AudioBackend {
            snapshot,
            commands,
            subscribers,
        }
    })
}

fn audio_worker(
    snapshot: Arc<RwLock<AudioStatus>>,
    subscribers: Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    commands: mpsc::Receiver<AudioCommand>,
) {
    pipewire::init();
    retry_audio_connection(
        &commands,
        |pending| run_connection(&snapshot, &subscribers, &commands, pending),
        |error| {
            tracing::warn!(%error, "PipeWire audio connection failed; retrying");
            publish(&snapshot, &subscribers, &Graph::default());
            thread::sleep(Duration::from_millis(500));
        },
    );
}

fn retry_audio_connection(
    commands: &mpsc::Receiver<AudioCommand>,
    mut connect: impl FnMut(&mut Option<AudioCommand>) -> Result<(), String>,
    mut on_failure: impl FnMut(String),
) {
    // Preserve the head command read by a disconnect probe across failed
    // connection setup. It must execute before later queue entries; one slot
    // also lets an empty disconnected worker stop when PipeWire is unavailable.
    let mut pending = None;
    while let Err(error) = connect(&mut pending) {
        on_failure(error);
        if pending.is_none() {
            match commands.try_recv() {
                Ok(command) => pending = Some(command),
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}

fn run_connection(
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    commands: &mpsc::Receiver<AudioCommand>,
    pending: &mut Option<AudioCommand>,
) -> Result<(), String> {
    let connection = create_connection(snapshot, subscribers, false)?;
    let AudioConnection {
        main_loop,
        core,
        completed,
        graph,
        ..
    } = &connection;

    loop {
        main_loop
            .iterate(Some(Duration::from_millis(50)))
            .map_err(|error| error.to_string())?;
        loop {
            match pending
                .take()
                .map(Ok)
                .unwrap_or_else(|| commands.try_recv())
            {
                Ok(command) => {
                    if graph.lock().map_or(true, |graph| graph.invalid_inventory) {
                        return Err("PipeWire inventory is ambiguous or exceeded its bound".into());
                    }
                    let command = match command {
                        AudioCommand::Refresh(reply) => {
                            let nodes = graph
                                .lock()
                                .map_err(|_| "PipeWire graph lock was poisoned")?
                                .sinks
                                .values()
                                .filter(|sink| sink.exposed)
                                .take(AUDIO_DEVICE_LIMIT)
                                .map(|sink| sink.node.clone())
                                .collect::<Vec<_>>();
                            for node in nodes {
                                node.enum_params(0, Some(ParamType::Props), 0, 128, None)
                                    .map_err(|error| error.to_string())?;
                            }
                            audio_roundtrip(core, main_loop, completed)?;
                            let result = graph
                                .lock()
                                .map_err(|_| "PipeWire graph lock was poisoned".to_owned())
                                .map(|graph| bound_audio_refresh(status_from_graph(&graph)));
                            let _ = reply.send(result);
                            continue;
                        }
                        command => command,
                    };
                    apply_command(command, graph)?;
                    audio_roundtrip(core, main_loop, completed)?;
                    // Relative commands must not all read the pre-burst graph.
                    // Refresh properties, then process the matching roundtrip
                    // before deriving the next adjustment. This wait is confined
                    // to the existing audio worker, never the compositor loop.
                    let node = {
                        let current = graph
                            .lock()
                            .map_err(|_| "PipeWire graph lock was poisoned")?;
                        effective_sink(&current)?.node.clone()
                    };
                    node.enum_params(0, Some(ParamType::Props), 0, u32::MAX, None)
                        .map_err(|error| error.to_string())?;
                    audio_roundtrip(core, main_loop, completed)?;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
}

struct AudioConnection {
    core: Core,
    completed: Arc<Mutex<Option<u32>>>,
    graph: Arc<Mutex<Graph>>,
    // Context and main loop outlive the Core and its proxies.
    _context: Context,
    main_loop: MainLoop,
}

impl Drop for AudioConnection {
    fn drop(&mut self) {
        // Core's destroy listener disconnects the transport, removes its loop
        // source and discards queued bytes. It does not flush on disconnect.
        self.core.disconnect();
    }
}

fn create_connection(
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    bounded: bool,
) -> Result<AudioConnection, String> {
    pipewire::init();
    let main_loop = MainLoop::new(&Properties::new())
        .ok_or_else(|| "could not create PipeWire main loop".to_owned())?;
    let context = Context::new(&main_loop, Properties::new()).map_err(|error| error.to_string())?;
    let core = if bounded {
        context.connect_timeout(None, Duration::from_millis(300))
    } else {
        context.connect(None)
    }
    .map_err(|error| error.to_string())?;
    if bounded {
        core.set_dispatch_limits(128, 262_144, 65_536)
            .map_err(|error| error.to_string())?;
    }
    let completed = Arc::new(Mutex::new(None));
    let completion = Arc::clone(&completed);
    let mut events = CoreEvents::default();
    events.done = Some(Box::new(move |id, sequence| {
        if id == 0 {
            *completion.lock().unwrap() = Some(sequence);
        }
    }));
    core.add_listener(events);
    let registry = core.registry().map_err(|error| error.to_string())?;
    let graph = Arc::new(Mutex::new(Graph::default()));
    let info_graph = Arc::clone(&graph);
    let mut info_events = CoreEvents::default();
    info_events.info = Some(Box::new(move |info| {
        if let Ok(mut graph) = info_graph.lock() {
            graph.cookie = Some(info.cookie);
        }
    }));
    core.add_listener(info_events);
    let listener_graph = Arc::clone(&graph);
    let listener_snapshot = Arc::clone(snapshot);
    let listener_subscribers = Arc::clone(subscribers);

    registry.add_listener(RegistryEvents {
        global: some_closure!([registry ^(listener_graph, listener_snapshot, listener_subscribers)] id, _permissions, type_, version, props, {
            if type_ == types::interface::NODE
                && props.get("media.class").is_some_and(|class| class.starts_with("Audio/Sink"))
            {
                if bounded && !admit_guarded_proxy(listener_graph, false) { return; }
                let name = props.get("node.name").unwrap_or("unknown-output").to_owned();
                if bounded && name.len() > 512 { return; }
                let description = props
                    .get("node.description")
                    .or_else(|| props.get("node.nick"))
                    .unwrap_or(&name)
                    .to_owned();
                if bounded && description.len() > 512 { return; }
                let exposed = props.get("media.class") == Some("Audio/Sink");
                let driver_id = props.get("node.driver-id").and_then(|value| value.parse().ok());
                let Ok(object) = registry.bind(id, type_, version.min(3)) else { return; };
                let Some(node) = object.downcast::<Node>() else { return; };
                let param_graph = Arc::clone(listener_graph);
                let param_snapshot = Arc::clone(listener_snapshot);
                let param_subscribers = Arc::clone(listener_subscribers);
                node.add_listener(NodeEvents {
                    param: some_closure!([^(param_graph, param_snapshot, param_subscribers)] _seq, param_id, _index, _next, pod, {
                        if param_id == ParamType::Props {
                            update_props(id, pod.data(), param_graph, param_snapshot, param_subscribers);
                        }
                    }),
                    ..Default::default()
                });
                let remove_graph = Arc::clone(listener_graph);
                let remove_snapshot = Arc::clone(listener_snapshot);
                let remove_subscribers = Arc::clone(listener_subscribers);
                node.proxy().add_listener(ProxyEvents {
                    removed: some_closure!([^(remove_graph, remove_snapshot, remove_subscribers)] {
                        if let Ok(mut graph) = remove_graph.lock() {
                            graph.sinks.remove(&id);
                            publish(remove_snapshot, remove_subscribers, &graph);
                        }
                    }),
                    ..Default::default()
                });
                if let Ok(mut graph) = listener_graph.lock() {
                    graph.sinks.insert(id, Sink {
                        node: node.clone(),
                        name,
                        serial: props.get("object.serial").and_then(|value| value.parse().ok()),
                        description,
                        exposed,
                        driver_id,
                        channel_volumes: vec![0.0],
                        volume_observed: false,
                        muted: false,
                        mute_observed: false,
                    });
                    publish(listener_snapshot, listener_subscribers, &graph);
                }
                let _ = node.subscribe_params(&[ParamType::Props]);
                let _ = node.enum_params(0, Some(ParamType::Props), 0, u32::MAX, None);
            } else if type_ == types::interface::METADATA && props.get("metadata.name") == Some("default") {
                if bounded && !admit_guarded_proxy(listener_graph, true) { return; }
                let Ok(object) = registry.bind(id, type_, version.min(3)) else { return; };
                let Some(metadata) = object.downcast::<Metadata>() else { return; };
                let metadata_graph = Arc::clone(listener_graph);
                let metadata_snapshot = Arc::clone(listener_snapshot);
                let metadata_subscribers = Arc::clone(listener_subscribers);
                metadata.add_listener(MetadataEvents {
                    property: some_closure!([^(metadata_graph, metadata_snapshot, metadata_subscribers)] _subject, key, _type, value, {
                        if key == Some("default.audio.sink")
                            && let Ok(mut graph) = metadata_graph.lock()
                        {
                            graph.default_name = value.and_then(default_sink_name);
                            publish(metadata_snapshot, metadata_subscribers, &graph);
                        }
                    }),
                });
                if let Ok(mut graph) = listener_graph.lock() {
                    graph.metadata = Some(metadata);
                    graph.metadata_id = Some(id);
                }
            }
        }),
        global_remove: some_closure!([^(graph, snapshot, subscribers)] id, {
            if let Ok(mut graph) = graph.lock() {
                graph.sinks.remove(&id);
                if graph.metadata_id == Some(id) {
                    graph.metadata = None;
                    graph.metadata_id = None;
                    if bounded { graph.invalid_inventory = true; }
                }
                publish(snapshot, subscribers, &graph);
            }
        }),
    });
    let _ = core.sync();

    Ok(AudioConnection {
        main_loop,
        _context: context,
        core,
        completed,
        graph,
    })
}

pub(super) fn execute_guarded(
    action: crate::control_view::ControlAction,
    permit: nickel_remote_control::DesktopPermit,
    origin: super::linux_guarded_control::GuardedControlOrigin,
) -> super::linux_guarded_control::GuardedControlOutcome {
    use super::linux_guarded_control::GuardedControlOutcome as Outcome;
    use crate::control_view::ControlAction;
    let command = match &action {
        ControlAction::SetAudioVolume(volume) if *volume <= 100 => AudioCommand::SetVolume(*volume),
        ControlAction::SetAudioMuted(muted) => AudioCommand::SetMuted(*muted),
        ControlAction::SelectAudioDevice { id } if id.len() <= 512 => {
            AudioCommand::SelectOutput(id.clone())
        }
        _ => return Outcome::Unavailable,
    };
    if origin.check().is_err() {
        return Outcome::Cancelled;
    }
    let evidence = nickel_remote_control::leases::ResourceEvidence {
        surface: None,
        window: None,
        verified_application: None,
        output: None,
        authorized_surface_ancestors: &[],
        protected: false,
    };
    if permit.with_resource(&evidence, || Ok(())).is_err() {
        return Outcome::Cancelled;
    }
    // This worker exclusively owns a separate connection. Neither the local
    // audio worker nor another thread can iterate or flush its pending bytes.
    let snapshot = Arc::new(RwLock::new(AudioStatus::default()));
    let subscribers = Arc::new(Mutex::new(Vec::new()));
    let Ok(connection) = create_connection(&snapshot, &subscribers, true) else {
        return Outcome::Unavailable;
    };
    let AudioConnection {
        core,
        main_loop,
        completed,
        graph,
        ..
    } = &connection;
    // Registry discovery, node bindings and initial properties each need their
    // own server barrier. These read-only setup phases never hold authority.
    for _ in 0..3 {
        if permit.check_live().is_err() || origin.check().is_err() {
            return Outcome::Cancelled;
        }
        if audio_roundtrip(core, main_loop, completed).is_err() {
            return Outcome::Unavailable;
        }
    }
    // Drain all setup output before queuing a mutation. No main-loop callbacks
    // execute between the setter and completion of its guarded socket writes.
    if !matches!(core.try_flush_pending_once(65_536), Ok(0)) {
        return Outcome::Unavailable;
    }

    let mut attempted = false;
    let result = origin.with_boundary(&permit, &evidence, |boundary| {
        permit.check_commit_boundary(boundary)?;
        origin.check()?;
        if graph.lock().map_or(true, |graph| graph.invalid_inventory) {
            return Err("PipeWire inventory is ambiguous or exceeded its bound".into());
        }
        if origin.expects_device() {
            origin.validate_observation(&device_observation(graph)?)?;
        }
        apply_command(command, graph)?;
        // Only this bounded message belongs to this connection. Each partial
        // write is reauthorized, with no event-loop iteration in between.
        for _ in 0..16 {
            permit.check_commit_boundary(boundary)?;
            origin.check()?;
            attempted = true;
            match core.try_flush_pending_once(65_536) {
                Ok(0) => return Ok(()),
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err("native audio submission incomplete".into()),
            }
        }
        Err("native audio submission exceeded step bound".into())
    });
    if result.is_err() {
        return if attempted {
            Outcome::Uncertain
        } else if permit.check_live().is_err() || origin.check().is_err() {
            Outcome::Cancelled
        } else {
            Outcome::Unavailable
        };
    }
    // Once the complete native message is accepted it cannot be rolled back.
    // Cancellation suppresses confirmed success, but cannot unsend that message.
    if permit.check_live().is_err()
        || origin.check().is_err()
        || audio_roundtrip(core, main_loop, completed).is_err()
    {
        return Outcome::Uncertain;
    }
    if permit.check_live().is_err() || origin.check().is_err() {
        return Outcome::Uncertain;
    }
    let node = graph
        .lock()
        .ok()
        .and_then(|current| effective_sink(&current).ok().map(|sink| sink.node.clone()));
    if let Some(node) = node
        && (node
            .enum_params(0, Some(ParamType::Props), 0, 128, None)
            .is_err()
            || audio_roundtrip(core, main_loop, completed).is_err())
    {
        return Outcome::Uncertain;
    }
    if permit.check_live().is_err() || origin.check().is_err() {
        return Outcome::Uncertain;
    }
    let confirmed = graph.lock().ok().is_some_and(|current| match action {
        ControlAction::SetAudioMuted(muted) => {
            effective_sink(&current).is_ok_and(|sink| sink.mute_observed && sink.muted == muted)
        }
        ControlAction::SetAudioVolume(volume) => effective_sink(&current).is_ok_and(|sink| {
            sink.volume_observed && average_volume(&sink.channel_volumes) == volume
        }),
        ControlAction::SelectAudioDevice { id } => {
            current.default_name.as_deref() == Some(id.as_str())
        }
        _ => false,
    });
    if confirmed {
        Outcome::Confirmed
    } else {
        Outcome::Requested
    }
}

fn audio_roundtrip(
    core: &Core,
    main_loop: &MainLoop,
    completed: &Mutex<Option<u32>>,
) -> Result<(), String> {
    *completed
        .lock()
        .map_err(|_| "PipeWire completion lock was poisoned")? = None;
    let sequence = core.sync().map_err(|error| error.to_string())?;
    wait_for_audio_ack(
        sequence,
        Duration::from_secs(2),
        || *completed.lock().unwrap(),
        |timeout| {
            main_loop
                .iterate(Some(timeout))
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
    )
}

fn wait_for_audio_ack(
    sequence: u32,
    timeout: Duration,
    mut completed: impl FnMut() -> Option<u32>,
    mut dispatch: impl FnMut(Duration) -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while completed() != Some(sequence) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            // The attempted command may already have reached the server. Fail
            // the connection without replaying it and risking a second toggle.
            return Err("PipeWire command acknowledgment timed out".into());
        }
        dispatch(remaining.min(Duration::from_millis(50)))?;
    }
    Ok(())
}

fn update_props(
    id: u32,
    data: &[u8],
    graph: &Arc<Mutex<Graph>>,
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
) {
    let mut volume = None;
    let mut muted = None;
    let mut parser = Parser::new(data);
    let parsed = parser.pop_object::<Prop, ParamType, _>(|properties, _| {
        for (key, _, value) in properties {
            match key {
                Prop::ChannelVolumes => {
                    if let Ok(values) = value.decode::<Vec<f32>>() {
                        volume = Some(values);
                    }
                }
                Prop::SoftVolumes if volume.is_none() => {
                    if let Ok(values) = value.decode::<Vec<f32>>() {
                        volume = Some(values);
                    }
                }
                Prop::Volume => {
                    if volume.is_none()
                        && let Ok(value) = value.decode::<f32>()
                    {
                        volume = Some(vec![value]);
                    }
                }
                Prop::Mute => {
                    if let Ok(value) = value.decode::<bool>() {
                        muted = Some(value);
                    }
                }
                Prop::SoftMute if muted.is_none() => {
                    if let Ok(value) = value.decode::<bool>() {
                        muted = Some(value);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    });
    if parsed.is_err() {
        return;
    }
    if let Ok(mut graph) = graph.lock()
        && let Some(sink) = graph.sinks.get_mut(&id)
    {
        if let Some(volume) = volume {
            sink.volume_observed = !volume.is_empty();
            sink.channel_volumes = volume;
        }
        if let Some(muted) = muted {
            sink.muted = muted;
            sink.mute_observed = true;
        }
        publish(snapshot, subscribers, &graph);
    }
}

fn default_sink_name(value: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(value)
        .ok()?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

fn publish(
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    graph: &Graph,
) {
    let status = bound_audio_refresh(status_from_graph(graph)).audio;
    let changed = if let Ok(mut current) = snapshot.write() {
        if *current == status {
            false
        } else {
            *current = status.clone();
            true
        }
    } else {
        false
    };
    if changed && let Ok(mut subscribers) = subscribers.lock() {
        let status = Arc::new(crate::platform::SystemStatusUpdate::Audio(status));
        subscribers.retain(|sender| sender.send(status.clone()).is_ok());
    }
}

fn status_from_graph(graph: &Graph) -> AudioStatus {
    let mut sinks = graph
        .sinks
        .values()
        .filter(|sink| sink.exposed)
        .collect::<Vec<_>>();
    sinks.sort_by(|left, right| left.description.cmp(&right.description));
    let effective = graph
        .default_name
        .as_ref()
        .and_then(|name| sinks.iter().find(|sink| &sink.name == name))
        .copied()
        .or_else(|| sinks.first().copied());
    let devices = sinks
        .iter()
        .map(|sink| AudioDeviceStatus {
            id: sink.name.clone(),
            name: sink.description.clone(),
            is_default: effective.is_some_and(|current| current.name == sink.name),
        })
        .collect();
    let effective_control = effective.and_then(|sink| control_sink(graph, sink));
    AudioStatus {
        available: effective_control.is_some(),
        devices,
        volume_percent: effective_control
            .map(|sink| average_volume(&sink.channel_volumes))
            .unwrap_or(0),
        muted: effective_control.is_some_and(|sink| sink.muted),
    }
}

fn average_volume(values: &[f32]) -> u8 {
    if values.is_empty() {
        return 0;
    }
    let average = values.iter().copied().sum::<f32>() / values.len() as f32;
    (average.cbrt() * 100.0).round().clamp(0.0, 100.0) as u8
}

fn apply_command(command: AudioCommand, graph: &Arc<Mutex<Graph>>) -> Result<(), String> {
    let graph = graph
        .lock()
        .map_err(|_| "PipeWire graph lock was poisoned")?;
    match command {
        AudioCommand::SelectOutput(name) => {
            if !graph
                .sinks
                .values()
                .any(|sink| sink.exposed && sink.name == name)
            {
                return Err("selected PipeWire output is stale".into());
            }
            let metadata = graph
                .metadata
                .as_ref()
                .ok_or("PipeWire default metadata is unavailable")?;
            let value = serde_json::json!({ "name": name }).to_string();
            metadata
                .set_property(
                    0,
                    Some("default.audio.sink"),
                    Some("Spa:String:JSON"),
                    Some(&value),
                )
                .map_err(|error| error.to_string())
        }
        AudioCommand::SetVolume(percent) => set_effective_volume(&graph, percent),
        AudioCommand::AdjustVolume(delta) => {
            let sink = effective_sink(&graph)?;
            let current = i16::from(average_volume(&sink.channel_volumes));
            set_effective_volume(&graph, (current + i16::from(delta)).clamp(0, 100) as u8)
        }
        AudioCommand::SetMuted(muted) => effective_sink(&graph)?
            .node
            .set_param(
                ParamType::Props,
                ObjectType::Props,
                0,
                Box::new(move |builder| {
                    builder.push_property(Prop::Mute, PropertyFlags::empty(), muted)
                }),
            )
            .map_err(|error| error.to_string()),
        AudioCommand::ToggleMute => {
            let sink = effective_sink(&graph)?;
            let muted = sink.muted;
            sink.node
                .set_param(
                    ParamType::Props,
                    ObjectType::Props,
                    0,
                    Box::new(move |builder| {
                        builder.push_property(Prop::Mute, PropertyFlags::empty(), !muted)
                    }),
                )
                .map_err(|error| error.to_string())
        }
        AudioCommand::Refresh(_) => Err("refresh command cannot enter the mutation path".into()),
    }
}

fn effective_sink(graph: &Graph) -> Result<&Sink, String> {
    let sink = graph
        .default_name
        .as_ref()
        .and_then(|name| {
            graph
                .sinks
                .values()
                .find(|sink| sink.exposed && &sink.name == name)
        })
        .or_else(|| graph.sinks.values().find(|sink| sink.exposed))
        .ok_or_else(|| "no PipeWire output is available".to_owned())?;
    Ok(control_sink(graph, sink).unwrap_or(sink))
}

fn control_sink<'a>(graph: &'a Graph, sink: &'a Sink) -> Option<&'a Sink> {
    match sink.driver_id {
        Some(id) => graph.sinks.get(&id),
        None => Some(sink),
    }
}

fn set_effective_volume(graph: &Graph, percent: u8) -> Result<(), String> {
    let sink = effective_sink(graph)?;
    let channels = sink.channel_volumes.len().max(1);
    let normalized = f32::from(percent.min(100)) / 100.0;
    let values = vec![normalized.powi(3); channels];
    sink.node
        .set_param(
            ParamType::Props,
            ObjectType::Props,
            0,
            Box::new(move |builder| {
                builder.push_property(Prop::ChannelVolumes, PropertyFlags::empty(), values)
            }),
        )
        .map_err(|error| error.to_string())
}

fn device_observation(
    graph: &Mutex<Graph>,
) -> Result<super::linux_device_settings::GuardedDeviceObservation, String> {
    use super::linux_device_settings::{GuardedDeviceObservation, NativeIdentity};
    let graph = graph.lock().map_err(|_| "audio observation unavailable")?;
    if graph.invalid_inventory {
        return Err("audio inventory unavailable".into());
    }
    let sink = effective_sink(&graph)?;
    if !sink.volume_observed || !sink.mute_observed {
        return Err("audio properties unobserved".into());
    }
    Ok(GuardedDeviceObservation {
        values: nickel_remote_control::device_settings::Values::Audio {
            volume_percent: average_volume(&sink.channel_volumes),
            muted: sink.muted,
        },
        identity: NativeIdentity::Audio {
            cookie: graph.cookie.ok_or("audio server identity unavailable")?,
            serial: sink.serial.ok_or("audio node identity unavailable")?,
        },
    })
}
pub(super) fn observe_guarded(
    permit: &nickel_remote_control::DesktopPermit,
) -> Result<super::linux_device_settings::GuardedDeviceObservation, String> {
    let snapshot = Arc::new(RwLock::new(AudioStatus::default()));
    let subscribers = Arc::new(Mutex::new(Vec::new()));
    let connection = create_connection(&snapshot, &subscribers, true)?;
    for _ in 0..3 {
        permit.check_live()?;
        audio_roundtrip(
            &connection.core,
            &connection.main_loop,
            &connection.completed,
        )?;
    }
    permit.check_live()?;
    device_observation(&connection.graph)
}

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use super::{average_volume, default_sink_name, refresh, set_volume, status};

    #[test]
    #[ignore = "requires owned private PipeWire daemon and dummy sink"]
    fn private_pipewire_flush_and_abandonment_use_actual_socket_boundary() {
        use super::*;
        assert_eq!(
            std::env::var("PIPEWIRE_REMOTE").unwrap(),
            "nickel-guarded-test"
        );
        assert_eq!(
            std::env::var("XDG_RUNTIME_DIR").unwrap(),
            "/tmp/nickel-guarded-native/runtime"
        );
        fn ready() -> AudioConnection {
            let connection = create_connection(
                &Arc::new(RwLock::new(AudioStatus::default())),
                &Arc::new(Mutex::new(Vec::new())),
                true,
            )
            .unwrap();
            for _ in 0..3 {
                audio_roundtrip(
                    &connection.core,
                    &connection.main_loop,
                    &connection.completed,
                )
                .unwrap();
            }
            assert_eq!(connection.core.try_flush_pending_once(65_536).unwrap(), 0);
            connection
        }
        fn volume(connection: &AudioConnection) -> u8 {
            average_volume(
                &effective_sink(&connection.graph.lock().unwrap())
                    .unwrap()
                    .channel_volumes,
            )
        }
        // Adversarial ready burst: the peer can supply far more responses than
        // one callback budget. Each native iterate must yield to the outer loop.
        for (messages, bytes, expected_batch) in [(7, 262_144, 7), (128, 16_384, 1)] {
            let burst = ready();
            burst
                .core
                .set_dispatch_limits(messages, bytes, 16_384)
                .unwrap();
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed_count = Arc::clone(&count);
            let mut burst_events = CoreEvents::default();
            burst_events.done = Some(Box::new(move |id, _| {
                if id == 0 {
                    observed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }));
            burst.core.add_listener(burst_events);
            for _ in 0..512 {
                burst.core.sync().unwrap();
            }
            assert_eq!(burst.core.try_flush_pending_once(65_536).unwrap(), 0);
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut yielded = false;
            while count.load(std::sync::atomic::Ordering::Relaxed) < 512 {
                assert!(
                    Instant::now() < deadline,
                    "burst callback prevented outer deadline progress"
                );
                let before = count.load(std::sync::atomic::Ordering::Relaxed);
                burst
                    .main_loop
                    .iterate(Some(Duration::from_millis(10)))
                    .unwrap();
                let after = count.load(std::sync::atomic::Ordering::Relaxed);
                assert!(
                    after - before <= expected_batch,
                    "one callback exceeded its message budget"
                );
                yielded |= after > 0 && after < 512;
            }
            assert!(yielded);
            drop(burst);
        }

        // A native peer repeatedly publishes and retires duplicate default
        // metadata. The guarded connection admits no replacement proxies.
        let churn = ready();
        let producer = ready();
        let admitted = churn.graph.lock().unwrap().proxy_admissions;
        for _ in 0..140 {
            let mut properties = Properties::new();
            properties.set("metadata.name", "default".into());
            let object = producer
                .core
                .create_object("metadata", types::interface::METADATA, 3, &properties)
                .unwrap();
            audio_roundtrip(&producer.core, &producer.main_loop, &producer.completed).unwrap();
            audio_roundtrip(&churn.core, &churn.main_loop, &churn.completed).unwrap();
            producer.core.destroy(object.as_ref()).unwrap();
            audio_roundtrip(&producer.core, &producer.main_loop, &producer.completed).unwrap();
            audio_roundtrip(&churn.core, &churn.main_loop, &churn.completed).unwrap();
        }
        assert!(churn.graph.lock().unwrap().invalid_inventory);
        assert_eq!(churn.graph.lock().unwrap().proxy_admissions, admitted);
        drop(producer);
        drop(churn);

        let original = ready();
        let before = volume(&original);
        drop(original);
        let abandoned = ready();
        apply_command(
            AudioCommand::SetVolume(if before == 13 { 14 } else { 13 }),
            &abandoned.graph,
        )
        .unwrap();
        // No iterate and no flush after cancellation: dropping the dedicated
        // connection must not let its buffered setter escape to the dummy sink.
        drop(abandoned);
        let confirmed = ready();
        assert_eq!(volume(&confirmed), before);
        let requested = if before == 29 { 30 } else { 29 };
        apply_command(AudioCommand::SetVolume(requested), &confirmed.graph).unwrap();
        let mut remaining = 1;
        for _ in 0..16 {
            remaining = confirmed.core.try_flush_pending_once(65_536).unwrap();
            if remaining == 0 {
                break;
            }
        }
        assert_eq!(remaining, 0);
        audio_roundtrip(&confirmed.core, &confirmed.main_loop, &confirmed.completed).unwrap();
        drop(confirmed);
        let observed = ready();
        assert_eq!(volume(&observed), requested);
        apply_command(AudioCommand::SetVolume(before), &observed.graph).unwrap();
        assert_eq!(observed.core.try_flush_pending_once(65_536).unwrap(), 0);
        audio_roundtrip(&observed.core, &observed.main_loop, &observed.completed).unwrap();
    }

    #[test]
    fn connection_retries_preserve_queued_commands_and_stop_after_disconnect() {
        use std::{cell::Cell, sync::mpsc};
        let (sender, receiver) = mpsc::channel();
        let mut sender = Some(sender);
        let attempts = Cell::new(0);
        let mut delivered = Vec::new();
        let mut failures = Vec::new();
        super::retry_audio_connection(
            &receiver,
            |pending| {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                if attempt < 2 {
                    return Err(format!("connection failure {attempt}"));
                }
                delivered.extend(pending.take());
                delivered.extend(receiver.try_iter());
                assert!(matches!(
                    receiver.try_recv(),
                    Err(mpsc::TryRecvError::Disconnected)
                ));
                Ok(())
            },
            |error| {
                failures.push(error);
                // Model media presses arriving during reconnect, not a snapshot
                // stream where replacing an intermediate value would be valid.
                sender
                    .as_ref()
                    .unwrap()
                    .send(super::AudioCommand::AdjustVolume(5))
                    .unwrap();
                sender
                    .as_ref()
                    .unwrap()
                    .send(super::AudioCommand::ToggleMute)
                    .unwrap();
                if attempts.get() == 2 {
                    sender.take();
                }
            },
        );
        assert_eq!(attempts.get(), 3);
        assert_eq!(failures.len(), 2);
        assert!(matches!(
            delivered.as_slice(),
            [
                super::AudioCommand::AdjustVolume(5),
                super::AudioCommand::ToggleMute,
                super::AudioCommand::AdjustVolume(5),
                super::AudioCommand::ToggleMute,
            ]
        ));
    }

    #[test]
    fn unavailable_audio_worker_stops_when_empty_command_source_disconnects() {
        let (sender, receiver) = std::sync::mpsc::channel();
        drop(sender);
        let mut attempts = 0;
        super::retry_audio_connection(
            &receiver,
            |_| {
                attempts += 1;
                assert_eq!(attempts, 1, "disconnected worker must not retry forever");
                Err("unavailable".into())
            },
            |_| {},
        );
        assert_eq!(attempts, 1);
    }

    #[test]
    fn audio_ack_requires_matching_sequence_and_dispatches_observed_properties() {
        use std::cell::Cell;
        let completed = Cell::new(Some(4));
        let observed = Cell::new(50);
        let mut requested = Vec::new();
        for sequence in [5, 6] {
            let target = observed.get() + 5;
            requested.push(target);
            super::wait_for_audio_ack(
                sequence,
                Duration::from_secs(1),
                || completed.get(),
                |timeout| {
                    assert!(timeout <= Duration::from_millis(50));
                    observed.set(target);
                    completed.set(Some(sequence));
                    Ok(())
                },
            )
            .unwrap();
        }
        assert_eq!(requested, [55, 60]);
        assert_eq!(observed.get(), 60);
        assert!(
            super::wait_for_audio_ack(
                7,
                Duration::ZERO,
                || completed.get(),
                |_| { panic!("expired acknowledgment must not dispatch or replay a command") }
            )
            .unwrap_err()
            .contains("timed out")
        );
    }

    #[test]
    fn audio_ack_propagates_connection_failure_without_retrying_dispatch() {
        let mut calls = 0;
        let result = super::wait_for_audio_ack(
            1,
            Duration::from_secs(1),
            || None,
            |_| {
                calls += 1;
                Err("disconnected".into())
            },
        );
        assert_eq!(result, Err("disconnected".into()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn volume_normalization_is_bounded() {
        assert_eq!(average_volume(&[0.125]), 50);
        assert_eq!(average_volume(&[2.0]), 100);
        assert_eq!(average_volume(&[]), 0);
    }

    #[test]
    fn default_metadata_is_untrusted_json() {
        assert_eq!(
            default_sink_name(r#"{"name":"speaker"}"#).as_deref(),
            Some("speaker")
        );
        assert_eq!(default_sink_name("not-json"), None);
    }

    #[test]
    #[ignore = "uses the live user PipeWire graph"]
    fn live_pipewire_graph_reports_an_output() {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let current = status();
            if current.available {
                assert!(!current.devices.is_empty());
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("live PipeWire graph did not publish an output");
    }

    #[test]
    #[ignore = "uses the live user PipeWire graph"]
    fn live_pipewire_refresh_returns_a_bounded_snapshot() {
        let refreshed = refresh().expect("live PipeWire refresh");
        assert!(refreshed.audio.devices.len() <= super::AUDIO_DEVICE_LIMIT);
        assert!(refreshed.audio.devices.iter().all(|device| {
            device.id.chars().count() <= crate::platform::AUDIO_TEXT_LIMIT
                && device.name.chars().count() <= crate::platform::AUDIO_TEXT_LIMIT
        }));
    }

    #[test]
    #[ignore = "temporarily mutates and restores the live user PipeWire output"]
    fn live_pipewire_volume_command_is_confirmed_and_restored() {
        let deadline = Instant::now() + Duration::from_secs(3);
        let original = loop {
            let current = status();
            if current.available {
                break current.volume_percent;
            }
            assert!(
                Instant::now() < deadline,
                "live PipeWire output was unavailable"
            );
            thread::sleep(Duration::from_millis(25));
        };
        let target = if original >= 99 {
            original.saturating_sub(1)
        } else {
            original + 1
        };
        assert!(set_volume(target));
        let changed_deadline = Instant::now() + Duration::from_secs(2);
        while status().volume_percent != target {
            if Instant::now() >= changed_deadline {
                let _ = set_volume(original);
                panic!("PipeWire did not confirm requested volume {target}%");
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(set_volume(original));
        let restore_deadline = Instant::now() + Duration::from_secs(2);
        while status().volume_percent != original {
            assert!(
                Instant::now() < restore_deadline,
                "PipeWire did not restore original volume {original}%"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
