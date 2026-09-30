// @jsx h
import "./styles/quick-settings.css";
// Nickel owns status snapshots and validates every requested system action.
export function QuickSettings(props) {
    const data = { ...{ scrollHeight: 552, network: { available: false, enabled: false, networks: [] }, bluetooth: { available: false, powered: false, discovering: false, devices: [] }, audio: { muted: false, percent: 0, devices: [] }, workspaces: [], projectionModes: [], slots: {} }, ...props?.data, audio: props?.data?.audio || nickel.audio.get(), network: nickel.wifi.get(), bluetooth: nickel.bluetooth.get() };
    const sections = (data.slots && data.slots["control-section"]) || [];
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
    return h(FixedWindow, { id: "quick-settings", edge: "right", width: 420, height: "100%", className: "control-center", onEscape: () => nickel.surfaces.hide("quick-settings") },
        h(Column, { className: "control-content" },
            h(Text, { className: "control-title" }, "Control Center"),
            h(ScrollView, { id: "control-center-scroll", height: data.scrollHeight },
                h(Column, { className: "control-sections" },
                    h(Row, null,
                        h(Text, null, "Wi-Fi: " + (data.network.available ? data.network.enabled ? "On" : "Off" : "Unavailable")),
                        data.network.operations.setEnabled ? h(Button, { id: "wifi-power", onClick: () => nickel.wifi.setEnabled(!data.network.enabled) }, data.network.enabled ? "Turn off" : "Turn on") : null,
                        h(Button, { id: "wifi-section", onClick: () => setWifiOpen(!wifiOpen) }, wifiOpen ? "Less" : "More")),
                    wifiOpen ? data.network.networks.map(network => h(Button, { key: network.id, id: "wifi-" + network.id, disabled: !network.canConnect, onClick: () => nickel.wifi.connect(network.id) }, network.name + (network.connected ? " · Connected" : network.saved ? " · Saved" : ""))) : null,
                    h(Row, null,
                        h(Text, null, "Bluetooth: " + (data.bluetooth.available ? data.bluetooth.powered ? "On" : "Off" : "Unavailable")),
                        data.bluetooth.operations.setPowered ? h(Button, { id: "bluetooth-power", onClick: () => nickel.bluetooth.setPowered(!data.bluetooth.powered) }, data.bluetooth.powered ? "Turn off" : "Turn on") : null,
                        h(Button, { id: "bluetooth-section", onClick: () => setBluetoothOpen(!bluetoothOpen) }, bluetoothOpen ? "Less" : "More")),
                    bluetoothOpen ? h(Column, null,
                        data.bluetooth.operations.setDiscovery && data.bluetooth.powered ? h(Button, { id: "bluetooth-scan", onClick: () => nickel.bluetooth.setDiscovery(!data.bluetooth.discovering) }, data.bluetooth.discovering ? "Stop scan" : "Scan nearby") : null,
                        data.bluetooth.devices.map(device => h(Button, { key: device.id, id: "bluetooth-" + device.id, disabled: !data.bluetooth.powered || !(device.paired ? data.bluetooth.operations[device.connected ? "disconnect" : "connect"] : data.bluetooth.operations.pair), onClick: () => device.paired ? device.connected ? nickel.bluetooth.disconnect(device.id) : nickel.bluetooth.connect(device.id) : nickel.bluetooth.pair(device.id) }, device.name + (device.connected ? " · Connected" : device.paired ? " · Paired" : "")))) : null,
                    h(Row, null,
                        h(Text, null, "Audio: " + (data.audio.muted ? "Muted" : data.audio.percent + "%")),
                        h(Button, { id: "audio-mute", onClick: () => nickel.audio.setMuted(!data.audio.muted) }, data.audio.muted ? "Unmute" : "Mute"),
                        h(Button, { id: "audio-section", onClick: () => setAudioOpen(!audioOpen) }, audioOpen ? "Less" : "More")),
                    h(Row, null,
                        h(Button, { id: "audio-down", onClick: () => nickel.audio.setVolume(Math.max(0, data.audio.percent - 10)) }, "\u2212"),
                        h(Progress, { className: "audio-progress", percent: data.audio.percent, width: 220, height: 8 }),
                        h(Button, { id: "audio-up", onClick: () => nickel.audio.setVolume(Math.min(100, data.audio.percent + 10)) }, "+")),
                    audioOpen ? data.audio.devices.map(device => h(Button, { key: device.id, id: "audio-" + device.id, onClick: () => nickel.audio.selectOutput(device.id) }, device.name + (device.isDefault ? " · Default" : ""))) : null,
                    h(Text, { className: "control-section-title" }, "Workspaces"),
                    h(Row, null,
                        data.workspaces.map((workspace, index) => h(Button, { key: workspace.id, id: "workspace-" + workspace.id, onClick: () => request("workspace-switch", workspace.id) }, (index + 1) + (workspace.active ? " ●" : ""))),
                        h(Button, { id: "workspace-create", onClick: () => request("workspace-create") }, "+"),
                        data.workspaces.length > 1 ? h(Button, { id: "workspace-remove", onClick: () => request("workspace-remove", data.activeWorkspace) }, "\u2212") : null),
                    h(Row, null,
                        h(Button, { id: "show-desktop", onClick: () => request("show-desktop") }, "Show desktop"),
                        h(Button, { id: "show-notifications", onClick: () => nickel.surfaces.show("notifications") }, "Notifications")),
                    sections.length ? h(Text, { className: "control-section-title" }, "Extensions") : null,
                    sections.map((section, index) => h(Row, { key: `${section.pluginId}:${section.id}` },
                        h(Text, null, section.label + ": " + section.value),
                        h(Button, { id: `control-extension-${index}`, onClick: () => nickel.request({
                                type: "invoke-plugin-slot-section", slot: "control-section",
                                pluginId: section.pluginId, id: section.id
                            }) }, "Open"))),
                    h(Text, { className: "control-section-title" }, "Displays"),
                    data.pendingProjection ? h(Row, null,
                        h(Text, null, "Keep display settings?"),
                        h(Button, { id: "projection-revert", onClick: () => request("projection-cancel") }, "Revert"),
                        h(Button, { id: "projection-keep", onClick: () => request("projection-confirm") }, "Keep")) : h(Row, null, data.projectionModes.map(mode => h(Button, { key: mode.id, id: "projection-" + mode.id, onClick: () => request("projection-preview", mode.id) }, mode.label))),
                    h(Text, { className: "control-section-title" }, "Session"),
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
