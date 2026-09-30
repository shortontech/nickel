//! Native operations shared by guarded OS adapters. Ordinary controls use public plugin clients.
use nickel_core::display_projection::ProjectionMode;

#[derive(Clone, Debug, PartialEq)]
pub enum ControlAction {
    SetWifiEnabled(bool),
    // Retained for native guarded OS delivery; ordinary JSX uses stable public effects.
    #[allow(dead_code)]
    ActivateWifi {
        id: String,
    },
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    // Retained for native guarded OS delivery; ordinary JSX uses stable public effects.
    #[allow(dead_code)]
    ToggleBluetoothDevice {
        id: String,
    },
    SetAudioVolume(u8),
    // Emitted by the Linux guarded audio owner; Windows uses its native
    // volume path and does not currently construct this shell action.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    SetAudioMuted(bool),
    // Retained for native guarded OS delivery; ordinary JSX uses stable public effects.
    #[allow(dead_code)]
    SelectAudioDevice {
        id: String,
    },
    PreviewProjection(ProjectionMode),
    ConfirmProjection,
    CancelProjection,
}
