// @jsx h
import "./styles/connectivity.css";
const deviceKey = device => device.id;
const deviceTypes = {
    "audio-card": ["\uf028", "Audio device"],
    "audio-headset": ["\uf025", "Headset"],
    "audio-headphones": ["\uf025", "Headphones"],
    "input-keyboard": ["\uf11c", "Keyboard"],
    "input-mouse": ["\uf245", "Mouse"],
    "phone": ["\uf10b", "Phone"],
    "computer": ["\uf109", "Computer"],
};
const deviceType = kind => Object.hasOwn(deviceTypes, kind) ? deviceTypes[kind] : ["\uf293", "Bluetooth device"];
export function Bluetooth() {
    const bluetooth = nickel.bluetooth.get();
    return h(Column, { className: "connectivity-page" },
        h(Row, { className: "connectivity-toolbar" },
            h(Text, { className: "connectivity-title" }, bluetooth.available ? bluetooth.powered ? "Bluetooth is on" : "Bluetooth is off" : "Bluetooth is unavailable"),
            bluetooth.operations.setPowered ? h(Button, { id: "settings-bluetooth-power", onClick: () => nickel.bluetooth.setPowered(!bluetooth.powered) }, bluetooth.powered ? "Turn off Bluetooth" : "Turn on Bluetooth") : null),
        bluetooth.available && bluetooth.writable === false ? h(Text, null, "Bluetooth controls are read only.") : null,
        bluetooth.adapterName ? h(Text, null, bluetooth.adapterName) : null,
        !bluetooth.available ? h(Text, { wrap: true }, bluetooth.reason || "The Bluetooth service is unavailable.") : null,
        bluetooth.operations.setDiscovery ? h(Button, { id: "settings-bluetooth-discovery", disabled: !bluetooth.powered, onClick: () => nickel.bluetooth.setDiscovery(!bluetooth.discovering) }, bluetooth.discovering ? "Stop discovery" : "Discover devices") : bluetooth.available && bluetooth.writable !== false ? h(Text, { wrap: true }, "Continuous device discovery is unavailable. Devices reported by the native service appear below.") : null,
        bluetooth.available && bluetooth.writable !== false && !bluetooth.operations.pair ? h(Text, { wrap: true }, "Pair new devices in system settings.") : null,
        bluetooth.available && bluetooth.writable !== false && !bluetooth.operations.connect && !bluetooth.operations.disconnect ? h(Text, { wrap: true }, "Manage device connections in system settings.") : null,
        bluetooth.available && !bluetooth.devices.length ? h(Text, null, bluetooth.powered ? "No devices are reported by the Bluetooth service." : "Turn on Bluetooth to use devices.") : null,
        h(VirtualColumn, { id: "settings-bluetooth-devices", items: bluetooth.devices, itemKey: deviceKey, itemHeight: 180, gap: 16, overscan: 96, renderItem: device => h(Column, { key: device.id, className: "connectivity-card" },
                h(Text, { className: "connectivity-name" }, device.name || "Unnamed device"),
                device.kind ? h(Row, { className: "connectivity-device-type" },
                    h(Text, { accessibilityLabel: deviceType(device.kind)[1] }, deviceType(device.kind)[0]),
                    h(Text, null, deviceType(device.kind)[1])) : null,
                device.batteryPercent !== null && device.batteryPercent !== undefined ? h(Text, null, "Battery: " + device.batteryPercent + "%") : null,
                device.signalDbm !== null && device.signalDbm !== undefined ? h(Text, null, "Signal: " + device.signalDbm + " dBm") : null,
                h(Text, null, device.connected ? "Connected" : device.paired ? "Paired" : "Not paired"),
                !device.paired && bluetooth.operations.pair ? h(Button, { id: "settings-bluetooth-pair/" + device.id, disabled: !bluetooth.powered, onClick: () => nickel.bluetooth.pair(device.id) }, "Pair") : null,
                device.paired && bluetooth.operations[device.connected ? "disconnect" : "connect"] ? h(Button, { id: "settings-bluetooth-connection/" + device.id, disabled: !bluetooth.powered, onClick: () => device.connected ? nickel.bluetooth.disconnect(device.id) : nickel.bluetooth.connect(device.id) }, device.connected ? "Disconnect" : "Connect") : null) }));
}
registerSettingsPage({ id: "bluetooth", group: "Network", label: "Bluetooth", description: "Device pairing, discovery, and connections", component: Bluetooth });
