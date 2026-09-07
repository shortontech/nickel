use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, RwLock, mpsc},
    thread,
    time::Duration,
};

use pipewire_native::{
    self as pipewire,
    context::Context,
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

use super::super::{AudioDeviceStatus, AudioStatus};

#[derive(Debug)]
enum AudioCommand {
    SetVolume(u8),
    AdjustVolume(i8),
    ToggleMute,
    SelectOutput(String),
}

struct AudioBackend {
    snapshot: Arc<RwLock<AudioStatus>>,
    commands: mpsc::Sender<AudioCommand>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<AudioStatus>>>>,
}

#[derive(Clone)]
struct Sink {
    node: Node,
    name: String,
    description: String,
    exposed: bool,
    driver_id: Option<u32>,
    channel_volumes: Vec<f32>,
    muted: bool,
}

#[derive(Default)]
struct Graph {
    sinks: HashMap<u32, Sink>,
    default_name: Option<String>,
    metadata: Option<Metadata>,
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

pub fn subscribe() -> mpsc::Receiver<AudioStatus> {
    let backend = backend();
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut subscribers) = backend.subscribers.lock() {
        subscribers.push(sender);
    }
    receiver
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
    subscribers: Arc<Mutex<Vec<mpsc::Sender<AudioStatus>>>>,
    commands: mpsc::Receiver<AudioCommand>,
) {
    pipewire::init();
    loop {
        if let Err(error) = run_connection(&snapshot, &subscribers, &commands) {
            tracing::warn!(%error, "PipeWire audio connection failed; retrying");
            publish(&snapshot, &subscribers, &Graph::default());
            thread::sleep(Duration::from_millis(500));
        }
        if matches!(commands.try_recv(), Err(mpsc::TryRecvError::Disconnected)) {
            return;
        }
    }
}

fn run_connection(
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<AudioStatus>>>>,
    commands: &mpsc::Receiver<AudioCommand>,
) -> Result<(), String> {
    let main_loop = MainLoop::new(&Properties::new())
        .ok_or_else(|| "could not create PipeWire main loop".to_owned())?;
    let context = Context::new(&main_loop, Properties::new()).map_err(|error| error.to_string())?;
    let core = context.connect(None).map_err(|error| error.to_string())?;
    let registry = core.registry().map_err(|error| error.to_string())?;
    let graph = Arc::new(Mutex::new(Graph::default()));
    let listener_graph = Arc::clone(&graph);
    let listener_snapshot = Arc::clone(snapshot);
    let listener_subscribers = Arc::clone(subscribers);

    registry.add_listener(RegistryEvents {
        global: some_closure!([registry ^(listener_graph, listener_snapshot, listener_subscribers)] id, _permissions, type_, version, props, {
            if type_ == types::interface::NODE
                && props.get("media.class").is_some_and(|class| class.starts_with("Audio/Sink"))
            {
                let name = props.get("node.name").unwrap_or("unknown-output").to_owned();
                let description = props
                    .get("node.description")
                    .or_else(|| props.get("node.nick"))
                    .unwrap_or(&name)
                    .to_owned();
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
                        description,
                        exposed,
                        driver_id,
                        channel_volumes: vec![0.0],
                        muted: false,
                    });
                    publish(listener_snapshot, listener_subscribers, &graph);
                }
                let _ = node.subscribe_params(&[ParamType::Props]);
                let _ = node.enum_params(0, Some(ParamType::Props), 0, u32::MAX, None);
            } else if type_ == types::interface::METADATA && props.get("metadata.name") == Some("default") {
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
                }
            }
        }),
        global_remove: some_closure!([^(graph, snapshot, subscribers)] id, {
            if let Ok(mut graph) = graph.lock() {
                graph.sinks.remove(&id);
                publish(snapshot, subscribers, &graph);
            }
        }),
    });
    let _ = core.sync();

    loop {
        main_loop
            .iterate(Some(Duration::from_millis(50)))
            .map_err(|error| error.to_string())?;
        loop {
            match commands.try_recv() {
                Ok(command) => apply_command(command, &graph)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
}

fn update_props(
    id: u32,
    data: &[u8],
    graph: &Arc<Mutex<Graph>>,
    snapshot: &Arc<RwLock<AudioStatus>>,
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<AudioStatus>>>>,
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
            sink.channel_volumes = volume;
        }
        if let Some(muted) = muted {
            sink.muted = muted;
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
    subscribers: &Arc<Mutex<Vec<mpsc::Sender<AudioStatus>>>>,
    graph: &Graph,
) {
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
    let status = AudioStatus {
        available: effective_control.is_some(),
        devices,
        volume_percent: effective_control
            .map(|sink| average_volume(&sink.channel_volumes))
            .unwrap_or(0),
        muted: effective_control.is_some_and(|sink| sink.muted),
    };
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
        subscribers.retain(|sender| sender.send(status.clone()).is_ok());
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

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use super::{average_volume, default_sink_name, set_volume, status};

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
