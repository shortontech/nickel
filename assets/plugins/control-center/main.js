// @jsx h
// Nickel owns status snapshots and validates every requested system action.
function App() {
    const data = nickel.data;
    const [wifiOpen, setWifiOpen] = useState(false);
    const [bluetoothOpen, setBluetoothOpen] = useState(false);
    const [audioOpen, setAudioOpen] = useState(false);
    const [confirming, setConfirming] = useState(null);
    const request = (action, value) => nickel.request({ type: "control-action", action, value });
    const prepare = action => {
        setConfirming(action);
        request("session-prepare", action);
        nickel.openDialog("session-confirm-dialog");
    };
    return h(Panel, { height: data.height, background: 0xf1222730 },
        h(Column, null,
            h(Text, null, "Control Center"),
            h(ScrollView, { id: "control-center-scroll", height: data.scrollHeight },
                h(Column, null,
                    h(Row, null,
                        h(Text, null, "Wi-Fi: " + (data.network.available ? data.network.enabled ? "On" : "Off" : "Unavailable")),
                        data.network.available ? h(Button, { id: "wifi-power", onClick: () => request("wifi-power", !data.network.enabled) }, data.network.enabled ? "Turn off" : "Turn on") : null,
                        h(Button, { id: "wifi-section", onClick: () => setWifiOpen(!wifiOpen) }, wifiOpen ? "Less" : "More")),
                    wifiOpen ? data.network.networks.map(network => h(Button, {
                        key: network.id, id: "wifi-" + network.id,
                        onClick: () => request("wifi-activate", network.id)
                    }, network.name + (network.connected ? " · Connected" : network.saved ? " · Saved" : ""))) : null,
                    h(Row, null,
                        h(Text, null, "Bluetooth: " + (data.bluetooth.available ? data.bluetooth.powered ? "On" : "Off" : "Unavailable")),
                        data.bluetooth.available ? h(Button, { id: "bluetooth-power", onClick: () => request("bluetooth-power", !data.bluetooth.powered) }, data.bluetooth.powered ? "Turn off" : "Turn on") : null,
                        h(Button, { id: "bluetooth-section", onClick: () => setBluetoothOpen(!bluetoothOpen) }, bluetoothOpen ? "Less" : "More")),
                    bluetoothOpen ? h(Column, null,
                        data.bluetooth.available && data.bluetooth.powered ? h(Button, { id: "bluetooth-scan", onClick: () => request("bluetooth-scan", !data.bluetooth.discovering) }, data.bluetooth.discovering ? "Stop scan" : "Scan nearby") : null,
                        data.bluetooth.devices.map(device => h(Button, {
                            key: device.id, id: "bluetooth-" + device.id,
                            onClick: () => request("bluetooth-device", device.id)
                        }, device.name + (device.connected ? " · Connected" : device.paired ? " · Paired" : "")))) : null,
                    h(Row, null,
                        h(Text, null, "Audio: " + (data.audio.muted ? "Muted" : data.audio.percent + "%")),
                        h(Button, { id: "audio-mute", onClick: () => request("audio-mute", !data.audio.muted) }, data.audio.muted ? "Unmute" : "Mute"),
                        h(Button, { id: "audio-section", onClick: () => setAudioOpen(!audioOpen) }, audioOpen ? "Less" : "More")),
                    h(Row, null,
                        h(Button, { id: "audio-down", onClick: () => request("audio-volume", Math.max(0, data.audio.percent - 10)) }, "−"),
                        h(Progress, { percent: data.audio.percent, width: 220, height: 8 }),
                        h(Button, { id: "audio-up", onClick: () => request("audio-volume", Math.min(100, data.audio.percent + 10)) }, "+")),
                    audioOpen ? data.audio.devices.map(device => h(Button, {
                        key: device.id, id: "audio-" + device.id,
                        onClick: () => request("audio-device", device.id)
                    }, device.name + (device.isDefault ? " · Default" : ""))) : null,
                    h(Text, null, "Workspaces"),
                    h(Row, null,
                        data.workspaces.map((workspace, index) => h(Button, {
                            key: workspace.id, id: "workspace-" + workspace.id,
                            onClick: () => request("workspace-switch", workspace.id)
                        }, (index + 1) + (workspace.active ? " ●" : ""))),
                        h(Button, { id: "workspace-create", onClick: () => request("workspace-create") }, "+"),
                        data.workspaces.length > 1 ? h(Button, { id: "workspace-remove", onClick: () => request("workspace-remove", data.activeWorkspace) }, "−") : null),
                    h(Row, null,
                        h(Button, { id: "show-desktop", onClick: () => request("show-desktop") }, "Show desktop"),
                        h(Button, { id: "show-notifications", onClick: () => request("show-notifications") }, "Notifications")),
                    h(Text, null, "Displays"),
                    data.pendingProjection ? h(Row, null,
                        h(Text, null, "Keep display settings?"),
                        h(Button, { id: "projection-revert", onClick: () => request("projection-cancel") }, "Revert"),
                        h(Button, { id: "projection-keep", onClick: () => request("projection-confirm") }, "Keep"))
                        : h(Row, null, data.projectionModes.map(mode => h(Button, {
                            key: mode.id, id: "projection-" + mode.id,
                            onClick: () => request("projection-preview", mode.id)
                        }, mode.label))),
                    h(Text, null, "Session"),
                    h(Row, null,
                        h(Button, { id: "session-lock", onClick: () => request("session-lock") }, "Lock"),
                        h(Button, { id: "session-suspend", onClick: () => prepare("suspend") }, "Suspend"),
                        h(Button, { id: "session-logout", onClick: () => prepare("logout") }, "Log out")),
                    h(Row, null,
                        h(Button, { id: "session-restart-shell", onClick: () => prepare("restart-shell") }, "Restart Nickel"),
                        h(Button, { id: "session-reboot", onClick: () => prepare("reboot") }, "Restart PC"),
                        h(Button, { id: "session-poweroff", onClick: () => prepare("poweroff") }, "Shut down")))),
            h(Dialog, { id: "session-confirm-dialog", anchor: "session-" + confirming, open: confirming !== null, onClose: () => setConfirming(null), width: 320, height: 128 },
                h(Column, null,
                    h(Text, null, "Confirm " + (confirming || "action") + "?"),
                    h(Row, null,
                        h(Button, { id: "session-cancel", onClick: () => { setConfirming(null); request("session-cancel"); } }, "Cancel"),
                        h(Button, { id: "session-confirm", onClick: () => { request("session-confirm"); setConfirming(null); } }, "Confirm"))))));
}
