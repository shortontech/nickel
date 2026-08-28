#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Availability {
    Available,
    Unavailable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioSnapshot {
    pub availability: Availability,
    pub devices: Vec<AudioDevice>,
    pub volume_percent: u8,
    pub muted: bool,
}

impl AudioSnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            availability: Availability::Unavailable(reason.into()),
            devices: Vec::new(),
            volume_percent: 0,
            muted: false,
        }
    }

    pub fn normalize(&mut self) {
        self.volume_percent = self.volume_percent.min(100);
        self.devices.sort_by(|left, right| {
            right
                .is_default
                .cmp(&left.is_default)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.id.cmp(&right.id))
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BluetoothDevice {
    pub id: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BluetoothSnapshot {
    pub availability: Availability,
    pub powered: bool,
    pub discovering: bool,
    pub devices: Vec<BluetoothDevice>,
}

impl BluetoothSnapshot {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            availability: Availability::Unavailable(reason.into()),
            powered: false,
            discovering: false,
            devices: Vec::new(),
        }
    }

    pub fn normalize(&mut self) {
        self.devices.sort_by(|left, right| {
            right
                .connected
                .cmp(&left.connected)
                .then_with(|| right.paired.cmp(&left.paired))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.id.cmp(&right.id))
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuickSettingsSnapshot {
    pub audio: AudioSnapshot,
    pub bluetooth: BluetoothSnapshot,
}

impl Default for QuickSettingsSnapshot {
    fn default() -> Self {
        Self {
            audio: AudioSnapshot::unavailable("Audio service is not available"),
            bluetooth: BluetoothSnapshot::unavailable("Bluetooth service is not available"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuickSettingsCommand {
    SetVolume(u8),
    SetMuted(bool),
    SelectAudioDevice(String),
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    ConnectBluetoothDevice(String),
    DisconnectBluetoothDevice(String),
    OpenFullSettings,
}

#[cfg(test)]
mod tests {
    use super::{AudioDevice, AudioSnapshot, Availability, BluetoothDevice, BluetoothSnapshot};

    #[test]
    fn audio_normalization_clamps_volume_and_leads_with_default_device() {
        let mut snapshot = AudioSnapshot {
            availability: Availability::Available,
            devices: vec![
                AudioDevice {
                    id: "speakers".into(),
                    name: "Speakers".into(),
                    is_default: false,
                },
                AudioDevice {
                    id: "headset".into(),
                    name: "Headset".into(),
                    is_default: true,
                },
            ],
            volume_percent: 140,
            muted: false,
        };

        snapshot.normalize();

        assert_eq!(snapshot.volume_percent, 100);
        assert_eq!(snapshot.availability, Availability::Available);
        assert!(!snapshot.muted);
        assert_eq!(
            snapshot.devices,
            vec![
                AudioDevice {
                    id: "headset".into(),
                    name: "Headset".into(),
                    is_default: true,
                },
                AudioDevice {
                    id: "speakers".into(),
                    name: "Speakers".into(),
                    is_default: false,
                },
            ]
        );
    }

    #[test]
    fn bluetooth_normalization_leads_with_connected_then_paired_devices() {
        let mut snapshot = BluetoothSnapshot {
            availability: Availability::Available,
            powered: true,
            discovering: false,
            devices: vec![
                BluetoothDevice {
                    id: "new".into(),
                    name: "New device".into(),
                    paired: false,
                    connected: false,
                },
                BluetoothDevice {
                    id: "paired".into(),
                    name: "Paired device".into(),
                    paired: true,
                    connected: false,
                },
                BluetoothDevice {
                    id: "active".into(),
                    name: "Active device".into(),
                    paired: true,
                    connected: true,
                },
            ],
        };

        snapshot.normalize();

        assert_eq!(
            snapshot.devices,
            vec![
                BluetoothDevice {
                    id: "active".into(),
                    name: "Active device".into(),
                    paired: true,
                    connected: true,
                },
                BluetoothDevice {
                    id: "paired".into(),
                    name: "Paired device".into(),
                    paired: true,
                    connected: false,
                },
                BluetoothDevice {
                    id: "new".into(),
                    name: "New device".into(),
                    paired: false,
                    connected: false,
                },
            ]
        );
        assert_eq!(snapshot.availability, Availability::Available);
        assert!(snapshot.powered);
        assert!(!snapshot.discovering);
    }
}
