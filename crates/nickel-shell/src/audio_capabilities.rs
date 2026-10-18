//! Bounded audio service state shared by all authorized package clients.
use crate::platform::AudioStatus;

pub(crate) fn snapshot(audio: &AudioStatus, locked: bool) -> serde_json::Value {
    let devices = if locked {
        Vec::new()
    } else {
        audio
            .devices
            .iter()
            .filter(|device| !device.id.is_empty() && device.id.chars().count() <= 512)
            .take(64)
            .map(|device| {
                serde_json::json!({
                    "id": device.id,
                    "name": device.name.chars().take(120).collect::<String>(),
                    "isDefault": device.is_default,
                })
            })
            .collect()
    };
    serde_json::json!({
        "available": audio.available,
        "percent": audio.volume_percent.min(100),
        "muted": audio.muted,
        "devices": devices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_service_snapshot_is_bounded_and_redacts_locked_devices() {
        let audio = AudioStatus {
            available: true,
            volume_percent: 140,
            muted: true,
            devices: vec![crate::platform::AudioDeviceStatus {
                id: "sink:1".into(),
                name: "Desk speakers".into(),
                is_default: true,
            }],
        };
        let state = snapshot(&audio, false);
        assert_eq!(state["percent"], 100);
        assert_eq!(state["muted"], true);
        assert_eq!(state["devices"][0]["id"], "sink:1");
        assert_eq!(state["devices"][0]["isDefault"], true);
        assert_eq!(snapshot(&audio, true)["devices"], serde_json::json!([]));
        assert!(state.get("label").is_none());
        let mut many = audio.clone();
        many.devices = (0..100)
            .map(|id| crate::platform::AudioDeviceStatus {
                id: format!("sink:{id}"),
                name: "x".repeat(200),
                is_default: id == 0,
            })
            .collect();
        let bounded = snapshot(&many, false);
        assert_eq!(bounded["devices"].as_array().unwrap().len(), 64);
        assert_eq!(bounded["devices"][0]["name"].as_str().unwrap().len(), 120);
    }
}
