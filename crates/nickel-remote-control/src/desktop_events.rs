//! Bounded payload-free production transitions. Never accepts strings or input data.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_DESKTOP_EVENTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductionEffectKind {
    ShellCommand,
    DeviceControl,
    ApplicationLaunch,
    WindowAction,
    WorkspaceAction,
    DiagnosticAction,
    SemanticAction,
    SettingsTransaction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductionEffectOutcome {
    Confirmed,
    UiUpdated,
    Requested,
    Cancelled,
    Unavailable,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellEventRole {
    Desktop,
    Panel,
    Launcher,
    ControlCenter,
    Notification,
    VolumeOsd,
    WindowPreview,
    WindowContextMenu,
    Screenshot,
    OnScreenKeyboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DesktopEventKind {
    /// Native process evidence made this ordinary window observable.
    WindowIdentityVerified { window_id: u64 },
    /// A previously verified ordinary native window was retired.
    WindowRetired { window_id: u64 },
    /// Compositor focus assignment to a verified ordinary window, not a native
    /// client acknowledgement. Window IDs match inventory generation values.
    KeyboardFocusChanged { window_id: u64 },
    /// Keyboard focus moved to an ordinary compositor-hosted shell surface.
    ShellKeyboardFocusChanged {
        surface_generation: u64,
        role: ShellEventRole,
    },
    /// No remotely observable ordinary recipient owns keyboard focus.
    KeyboardFocusCleared,
    OutputMembershipChanged {
        latest_output_identity_generation: u64,
        outputs: usize,
    },
    /// Current production layout state for one observable output generation.
    /// Connector names, models, EDID and physical dimensions are excluded.
    OutputStateChanged {
        output_generation: u64,
        geometry: [i32; 4],
        work_area: [i32; 4],
        scale_120: u32,
        primary: bool,
        enabled: bool,
    },
    /// A production owner committed a typed shell continuation. Fixed enums
    /// deliberately exclude the command, target, client and application.
    ProductionEffectCompleted {
        effect: ProductionEffectKind,
        outcome: ProductionEffectOutcome,
    },
    /// A bounded installed-application discovery reached its production owner.
    ApplicationInventoryRefreshCompleted { generation: u64, partial: bool },
    /// An allowlisted platform query reached its production owner.
    PlatformRefreshCompleted {
        domain: crate::diagnostics::PlatformRefreshDomain,
        generation: u64,
        partial: bool,
    },
    /// Production workspace owner state after create, remove, or selection.
    /// Window membership is available only through the protected snapshot.
    WorkspaceStateChanged {
        active_workspace: u64,
        workspaces: usize,
    },
    /// An ordinary compositor-owned shell presentation was inserted or
    /// retired. Protected and unsupported roles never enter this event.
    ShellSurfaceVisibilityChanged {
        surface_generation: u64,
        role: ShellEventRole,
        visible: bool,
    },
    /// Changed production state for a still-observable ordinary window.
    /// Titles and application identities remain outside the event stream.
    WindowStateChanged {
        window_id: u64,
        geometry: Option<[i32; 4]>,
        workspace: u64,
        active: bool,
        minimized: bool,
        maximized: bool,
        fullscreen: bool,
    },
    /// Coarse ownership of shared compositor input. Deliberately excludes the
    /// controller, recipient, pressed keys, buttons, text, and coordinates.
    RemoteInputOwnershipChanged {
        keyboard_held: bool,
        pointer_held: bool,
    },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub event: DesktopEventKind,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventSnapshot {
    pub generation: u64,
    pub evicted: u64,
    pub events: Vec<DesktopEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadDesktopEvents {
    pub lease_id: u64,
    pub after_generation: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventBatch {
    pub generation: u64,
    pub history_gap: bool,
    pub events: Vec<DesktopEvent>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventObservation {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub history: DesktopEventBatch,
}

#[derive(Default)]
pub struct DesktopEvents {
    generation: u64,
    evicted: u64,
    events: VecDeque<DesktopEvent>,
    remote_input_ownership: (bool, bool),
}

impl DesktopEvents {
    pub fn record(&mut self, event: DesktopEventKind, observed_at_us: u64) {
        self.generation = self.generation.saturating_add(1);
        if self.events.len() == MAX_DESKTOP_EVENTS {
            self.events.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.events.push_back(DesktopEvent {
            generation: self.generation,
            observed_at_us,
            event,
        });
    }

    pub fn record_workspace_state(
        &mut self,
        active_workspace: u64,
        workspaces: usize,
        observed_at_us: u64,
    ) {
        if self.events.back().is_some_and(|event| {
            event.event
                == (DesktopEventKind::WorkspaceStateChanged {
                    active_workspace,
                    workspaces,
                })
        }) {
            return;
        }
        self.record(
            DesktopEventKind::WorkspaceStateChanged {
                active_workspace,
                workspaces,
            },
            observed_at_us,
        );
    }

    pub fn record_remote_input_ownership(
        &mut self,
        keyboard_held: bool,
        pointer_held: bool,
        observed_at_us: u64,
    ) {
        let state = (keyboard_held, pointer_held);
        if self.remote_input_ownership == state {
            return;
        }
        self.remote_input_ownership = state;
        self.record(
            DesktopEventKind::RemoteInputOwnershipChanged {
                keyboard_held,
                pointer_held,
            },
            observed_at_us,
        );
    }

    pub fn since(&self, after: u64) -> Result<DesktopEventBatch, String> {
        if after > self.generation {
            return Err("event cursor is ahead of this session; take a fresh snapshot".into());
        }
        Ok(DesktopEventBatch {
            generation: self.generation,
            history_gap: self
                .events
                .front()
                .is_some_and(|event| after < event.generation.saturating_sub(1)),
            events: self
                .events
                .iter()
                .filter(|event| event.generation > after)
                .cloned()
                .collect(),
        })
    }

    pub fn snapshot(&self) -> DesktopEventSnapshot {
        DesktopEventSnapshot {
            generation: self.generation,
            evicted: self.evicted,
            events: self.events.iter().cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_reports_eviction_without_duplicates_or_silent_future_cursor_reset() {
        let mut events = DesktopEvents::default();
        assert!(events.since(0).unwrap().events.is_empty());
        assert!(events.since(1).is_err());
        for index in 1..=140 {
            events.record(
                DesktopEventKind::KeyboardFocusChanged { window_id: index },
                index,
            );
        }
        let gap = events.since(1).unwrap();
        assert!(gap.history_gap);
        assert_eq!(gap.events.len(), 128);
        assert_eq!(gap.events[0].generation, 13);
        assert!(!events.since(12).unwrap().history_gap);
        let tail = events.since(139).unwrap();
        assert_eq!(tail.events.len(), 1);
        assert_eq!(tail.generation, 140);
        let caught_up = events.since(tail.generation).unwrap();
        assert!(caught_up.events.is_empty() && !caught_up.history_gap);
        assert!(events.since(141).is_err());
    }

    #[test]
    fn transitions_keep_order_and_bound_retention_without_payload_fields() {
        let mut events = DesktopEvents::default();
        for index in 0..200 {
            events.record(
                DesktopEventKind::KeyboardFocusChanged {
                    window_id: index % 2 + 1,
                },
                index * 100,
            );
        }
        let snapshot = events.snapshot();
        assert_eq!(snapshot.generation, 200);
        assert_eq!(snapshot.evicted, 72);
        assert_eq!(snapshot.events.len(), MAX_DESKTOP_EVENTS);
        assert_eq!(snapshot.events[0].generation, 73);
        assert!(
            snapshot
                .events
                .windows(2)
                .all(|pair| pair[0].generation + 1 == pair[1].generation
                    && pair[0].observed_at_us < pair[1].observed_at_us)
        );
        assert_eq!(
            snapshot.events.last().unwrap().event,
            DesktopEventKind::KeyboardFocusChanged { window_id: 2 }
        );
    }

    #[test]
    fn production_effect_events_serialize_without_targets_or_payload_fields() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::ApplicationLaunch,
                outcome: ProductionEffectOutcome::Confirmed,
            },
            17,
        );
        let json = serde_json::to_string(&events.snapshot()).unwrap();
        assert!(json.contains("application_launch"));
        for excluded in ["command", "target", "client", "path", "application_id"] {
            assert!(!json.contains(excluded));
        }

        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::WindowAction,
                outcome: ProductionEffectOutcome::Requested,
            },
            18,
        );
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::WorkspaceAction,
                outcome: ProductionEffectOutcome::Confirmed,
            },
            19,
        );
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::DiagnosticAction,
                outcome: ProductionEffectOutcome::UiUpdated,
            },
            20,
        );
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::SemanticAction,
                outcome: ProductionEffectOutcome::UiUpdated,
            },
            21,
        );
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::SettingsTransaction,
                outcome: ProductionEffectOutcome::Confirmed,
            },
            22,
        );
        let json = serde_json::to_string(&events.snapshot()).unwrap();
        assert!(json.contains("window_action"));
        assert!(json.contains("workspace_action"));
        assert!(json.contains("diagnostic_action"));
        assert!(json.contains("semantic_action"));
        assert!(json.contains("settings_transaction"));
    }

    #[test]
    fn workspace_state_events_coalesce_redundant_notifications() {
        let mut events = DesktopEvents::default();
        events.record_workspace_state(1, 2, 10);
        events.record_workspace_state(1, 2, 11);
        events.record_workspace_state(2, 2, 12);
        let snapshot = events.snapshot();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.events[0].observed_at_us, 10);
        assert_eq!(
            snapshot.events[1].event,
            DesktopEventKind::WorkspaceStateChanged {
                active_workspace: 2,
                workspaces: 2,
            }
        );
    }

    #[test]
    fn shell_visibility_event_schema_has_only_fixed_identity_and_state() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::ShellSurfaceVisibilityChanged {
                surface_generation: 41,
                role: ShellEventRole::Screenshot,
                visible: true,
            },
            12,
        );
        let json = serde_json::to_string(&events.snapshot()).unwrap();
        assert!(json.contains("screenshot"));
        for excluded in ["title", "text", "path", "output", "client"] {
            assert!(!json.contains(excluded));
        }
    }

    #[test]
    fn window_state_event_schema_excludes_title_and_application_identity() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::WindowStateChanged {
                window_id: 9,
                geometry: Some([-20, 30, 800, 600]),
                workspace: 2,
                active: true,
                minimized: false,
                maximized: true,
                fullscreen: false,
            },
            15,
        );
        let json = serde_json::to_string(&events.snapshot()).unwrap();
        assert!(json.contains("window_state_changed"));
        for excluded in ["title", "application_id", "client", "text"] {
            assert!(!json.contains(excluded));
        }
    }

    #[test]
    fn remote_input_ownership_events_are_coalesced_and_payload_free() {
        let mut events = DesktopEvents::default();
        events.record_remote_input_ownership(false, false, 1);
        events.record_remote_input_ownership(true, false, 2);
        events.record_remote_input_ownership(true, false, 3);
        events.record_remote_input_ownership(true, true, 4);
        events.record_remote_input_ownership(false, false, 5);

        let snapshot = events.snapshot();
        assert_eq!(snapshot.events.len(), 3);
        assert_eq!(snapshot.events[0].observed_at_us, 2);
        assert_eq!(
            snapshot.events[1].event,
            DesktopEventKind::RemoteInputOwnershipChanged {
                keyboard_held: true,
                pointer_held: true,
            }
        );
        let json = serde_json::to_value(&snapshot).unwrap();
        let fields = json["events"][0]["event"].as_object().unwrap();
        assert_eq!(fields.len(), 3);
        assert!(fields.contains_key("kind"));
        assert!(fields.contains_key("keyboard_held"));
        assert!(fields.contains_key("pointer_held"));
    }

    #[test]
    fn output_state_event_schema_excludes_connector_and_hardware_identity() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::OutputStateChanged {
                output_generation: 4,
                geometry: [-1920, 0, 1920, 1080],
                work_area: [-1920, 0, 1920, 1040],
                scale_120: 150,
                primary: false,
                enabled: true,
            },
            19,
        );
        let value = serde_json::to_value(events.snapshot()).unwrap();
        let fields = value["events"][0]["event"].as_object().unwrap();
        assert_eq!(fields.len(), 7);
        for included in [
            "kind",
            "output_generation",
            "geometry",
            "work_area",
            "scale_120",
            "primary",
            "enabled",
        ] {
            assert!(fields.contains_key(included));
        }
    }

    #[test]
    fn shell_focus_event_has_only_generation_and_fixed_role() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::ShellKeyboardFocusChanged {
                surface_generation: 17,
                role: ShellEventRole::Launcher,
            },
            21,
        );
        events.record(DesktopEventKind::KeyboardFocusCleared, 22);
        let value = serde_json::to_value(events.snapshot()).unwrap();
        let focused = value["events"][0]["event"].as_object().unwrap();
        assert_eq!(focused.len(), 3);
        assert_eq!(focused["role"], "launcher");
        assert!(focused.contains_key("surface_generation"));
        let cleared = value["events"][1]["event"].as_object().unwrap();
        assert_eq!(cleared.len(), 1);
    }

    #[test]
    fn refresh_events_correlate_without_inventory_or_provider_payloads() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::ApplicationInventoryRefreshCompleted {
                generation: 4,
                partial: true,
            },
            30,
        );
        events.record(
            DesktopEventKind::PlatformRefreshCompleted {
                domain: crate::diagnostics::PlatformRefreshDomain::Audio,
                generation: 5,
                partial: false,
            },
            31,
        );
        let value = serde_json::to_value(events.snapshot()).unwrap();
        let inventory = value["events"][0]["event"].as_object().unwrap();
        let platform = value["events"][1]["event"].as_object().unwrap();
        assert_eq!(inventory.len(), 3);
        assert_eq!(platform.len(), 4);
        assert_eq!(platform["domain"], "audio");
    }
}
