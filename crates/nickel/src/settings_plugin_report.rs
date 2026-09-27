//! Bounded, short-lived telemetry from the separate Settings plugin process.

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use nickel_core::plugins::PluginManifest;
use nickel_session_protocol::{
    PluginMemorySnapshot, PluginRuntimeHealth, PluginStatus, PluginStatusSnapshot,
};

pub(crate) const ID: &str = "org.nickel.settings";
const MAX_REPORTED_BYTES: u64 = 1 << 40;
const MAX_AGE: Duration = Duration::from_secs(5);
const MANIFEST_SOURCE: &str = include_str!("../../../assets/plugins/settings/plugin.json");
static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();

pub(crate) fn manifest() -> &'static PluginManifest {
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(MANIFEST_SOURCE)
            .expect("bundled Settings manifest is validated by the Settings package")
    })
}

pub(crate) struct SettingsPluginReport {
    enabled: bool,
    memory: PluginMemorySnapshot,
    received_at: Instant,
}

impl SettingsPluginReport {
    pub(crate) fn new(
        enabled: bool,
        memory: PluginMemorySnapshot,
        received_at: Instant,
    ) -> Result<Self, &'static str> {
        if [
            memory.js_heap_bytes,
            memory.native_ui_bytes,
            memory.texture_bytes,
            memory.tracked_peak_bytes,
        ]
        .into_iter()
        .flatten()
        .any(|bytes| bytes > MAX_REPORTED_BYTES)
            || memory.timers > 100_000
            || memory.subscriptions > 100_000
        {
            return Err("Settings plugin memory exceeds limits");
        }
        Ok(Self {
            enabled,
            memory,
            received_at,
        })
    }

    pub(crate) fn append_to(&self, snapshot: &mut PluginStatusSnapshot, now: Instant) {
        if now.duration_since(self.received_at) > MAX_AGE {
            return;
        }
        if let Some(plugin) = snapshot.plugins.iter_mut().find(|plugin| plugin.id == ID) {
            if plugin.desired_enabled == self.enabled {
                plugin.health = if self.enabled {
                    PluginRuntimeHealth::Running
                } else {
                    PluginRuntimeHealth::Disabled
                };
                plugin.memory = self.memory.clone();
            }
            return;
        }
        let manifest = manifest();
        snapshot.plugins.retain(|plugin| plugin.id != ID);
        snapshot.plugins.push(PluginStatus {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            author: manifest.author.clone(),
            version: manifest.version.clone(),
            desired_enabled: self.enabled,
            health: if self.enabled {
                PluginRuntimeHealth::Running
            } else {
                PluginRuntimeHealth::Disabled
            },
            capabilities: manifest
                .capabilities
                .iter()
                .map(|grant| grant.as_str().to_owned())
                .collect(),
            surfaces: manifest
                .surfaces
                .iter()
                .map(|surface| format!("{}: {}", surface.id, surface.kind.as_str()))
                .collect(),
            composition: Vec::new(),
            settings: Vec::new(),
            memory: self.memory.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_memory_report_is_bounded_and_expires() {
        let now = Instant::now();
        let report = SettingsPluginReport::new(
            true,
            PluginMemorySnapshot {
                native_ui_bytes: Some(1234),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        let mut snapshot = PluginStatusSnapshot {
            activation_generation: 1,
            plugins: Vec::new(),
        };
        report.append_to(&mut snapshot, now);
        assert_eq!(snapshot.plugins[0].id, ID);
        assert_eq!(snapshot.plugins[0].memory.native_ui_bytes, Some(1234));
        let mut disabled = PluginStatusSnapshot {
            activation_generation: 2,
            plugins: vec![PluginStatus {
                id: ID.into(),
                desired_enabled: false,
                memory: PluginMemorySnapshot::default(),
                ..snapshot.plugins[0].clone()
            }],
        };
        report.append_to(&mut disabled, now);
        assert!(!disabled.plugins[0].desired_enabled);
        assert_eq!(disabled.plugins[0].memory.native_ui_bytes, None);
        let mut stale = PluginStatusSnapshot {
            activation_generation: 1,
            plugins: Vec::new(),
        };
        report.append_to(&mut stale, now + MAX_AGE + Duration::from_millis(1));
        assert!(stale.plugins.is_empty());
        assert!(
            SettingsPluginReport::new(
                true,
                PluginMemorySnapshot {
                    native_ui_bytes: Some(MAX_REPORTED_BYTES + 1),
                    ..Default::default()
                },
                now,
            )
            .is_err()
        );
    }
}
