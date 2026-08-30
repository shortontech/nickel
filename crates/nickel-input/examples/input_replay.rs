//! Deterministic, content-free input trace replay for acceptance diagnostics.
//!
//! Trace lines are `device order key edge repeat`, for example:
//! `1 2 KeyR pressed false`. `focus-lost order` and
//! `device-removed device order` are also accepted. Text is deliberately not
//! part of this diagnostic format.

use std::{collections::BTreeSet, env, fs, process::ExitCode};

use nickel_input::{
    AggregateModifier, Binding, DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent,
    KeyLocation, LogicalKey, ModifierState, NamedKey, NativeCode, NativeKey, PhysicalKey, Shortcut,
    ShortcutEngine, ShortcutKey, ShortcutTrigger,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureAction {
    Run,
    Screenshot,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("input replay failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let trace = match env::args().nth(1) {
        Some(path) => {
            fs::read_to_string(&path).map_err(|error| format!("could not read {path}: {error}"))?
        }
        None => DEFAULT_TRACE.into(),
    };
    let events = trace
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(index, line)| {
            parse_event(line).map_err(|error| format!("line {}: {error}", index + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut engine = fixture_engine();
    let mut outcomes = Vec::new();
    for event in &events {
        outcomes.extend(engine.handle(event));
    }
    println!("events={} outcomes={}", events.len(), outcomes.len());
    for outcome in outcomes {
        println!("action={:?} suppress={}", outcome.action, outcome.suppress);
    }
    Ok(())
}

fn fixture_engine() -> ShortcutEngine<FixtureAction> {
    ShortcutEngine::new([
        Binding {
            shortcut: Shortcut {
                key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyR)),
                modifiers: BTreeSet::from([AggregateModifier::Super]),
                trigger: ShortcutTrigger::Pressed,
            },
            action: FixtureAction::Run,
            suppress: true,
        },
        Binding {
            shortcut: Shortcut {
                key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::PrintScreen)),
                modifiers: BTreeSet::new(),
                trigger: ShortcutTrigger::Pressed,
            },
            action: FixtureAction::Screenshot,
            suppress: true,
        },
    ])
}

fn parse_event(line: &str) -> Result<InputEvent, String> {
    let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
    match fields.as_slice() {
        ["focus-gained", order] => Ok(InputEvent::FocusGained {
            order: EventOrder(parse(order, "order")?),
        }),
        ["focus-lost", order] => Ok(InputEvent::FocusLost {
            order: EventOrder(parse(order, "order")?),
        }),
        ["device-removed", device, order] => Ok(InputEvent::DeviceRemoved {
            device: DeviceId(parse(device, "device")?),
            order: EventOrder(parse(order, "order")?),
        }),
        [device, order, key, edge, repeat] => {
            let key = key_code(key)?;
            Ok(InputEvent::Key(KeyEvent {
                device: DeviceId(parse(device, "device")?),
                order: EventOrder(parse(order, "order")?),
                physical: PhysicalKey::Code(key),
                logical: logical_key(key),
                location: location(key),
                edge: match *edge {
                    "pressed" => KeyEdge::Pressed,
                    "released" => KeyEdge::Released,
                    _ => return Err(format!("unknown edge {edge}")),
                },
                repeat: parse(repeat, "repeat")?,
                modifiers: ModifierState::default(),
            }))
        }
        _ => Err("expected a key, focus-gained, focus-lost, or device-removed record".into()),
    }
}

fn parse<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
    value.parse().map_err(|_| format!("invalid {name} {value}"))
}

fn key_code(value: &str) -> Result<KeyCode, String> {
    Ok(match value {
        "SuperLeft" => KeyCode::SuperLeft,
        "SuperRight" => KeyCode::SuperRight,
        "AltLeft" => KeyCode::AltLeft,
        "AltRight" => KeyCode::AltRight,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "ShiftRight" => KeyCode::ShiftRight,
        "ControlLeft" => KeyCode::ControlLeft,
        "ControlRight" => KeyCode::ControlRight,
        "KeyR" => KeyCode::KeyR,
        "Tab" => KeyCode::Tab,
        "PrintScreen" => KeyCode::PrintScreen,
        _ => return Err(format!("unknown fixture key {value}")),
    })
}

fn logical_key(key: KeyCode) -> LogicalKey {
    match key {
        KeyCode::KeyR => LogicalKey::Character("r".into()),
        KeyCode::Tab => LogicalKey::Named(NamedKey::Tab),
        KeyCode::PrintScreen => LogicalKey::Named(NamedKey::PrintScreen),
        _ => LogicalKey::Native(NativeKey {
            namespace: "fixture".into(),
            code: NativeCode::Numeric(key as u64),
        }),
    }
}

fn location(key: KeyCode) -> KeyLocation {
    match key {
        KeyCode::SuperLeft | KeyCode::AltLeft | KeyCode::ShiftLeft | KeyCode::ControlLeft => {
            KeyLocation::Left
        }
        KeyCode::SuperRight | KeyCode::AltRight | KeyCode::ShiftRight | KeyCode::ControlRight => {
            KeyLocation::Right
        }
        _ => KeyLocation::Standard,
    }
}

const DEFAULT_TRACE: &str = "\
1 1 SuperLeft pressed false
1 2 KeyR pressed false
1 3 KeyR pressed true
1 4 KeyR released false
1 5 SuperLeft released false
1 6 PrintScreen pressed false
1 7 PrintScreen released false
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_trace_replays_once_per_physical_press() {
        let mut engine = fixture_engine();
        let outcomes = DEFAULT_TRACE
            .lines()
            .filter(|line| !line.is_empty())
            .flat_map(|line| engine.handle(&parse_event(line).unwrap()))
            .map(|outcome| outcome.action)
            .collect::<Vec<_>>();
        assert_eq!(outcomes, [FixtureAction::Run, FixtureAction::Screenshot]);
    }

    #[test]
    fn reset_records_do_not_manufacture_actions() {
        let mut engine = fixture_engine();
        assert!(
            engine
                .handle(&parse_event("1 1 SuperLeft pressed false").unwrap())
                .is_empty()
        );
        assert!(
            engine
                .handle(&parse_event("focus-lost 2").unwrap())
                .is_empty()
        );
        assert!(
            engine
                .handle(&parse_event("1 3 KeyR pressed false").unwrap())
                .is_empty()
        );
    }
}
