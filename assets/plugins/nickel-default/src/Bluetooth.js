// @jsx h
import "./styles/connectivity.css";
export function Bluetooth() {
    const bluetooth = nickel.bluetooth.get();
    return h(Column, { className: "connectivity-page" },
        h(Row, { className: "connectivity-toolbar" },
            h(Text, { className: "connectivity-title" }, bluetooth.available ? bluetooth.powered ? "Bluetooth is on" : "Bluetooth is off" : "Bluetooth is unavailable"),
            bluetooth.operations.setPowered ? h(Button, { id: "settings-bluetooth-power", onClick: () => nickel.bluetooth.setPowered(!bluetooth.powered) }, bluetooth.powered ? "Turn off Bluetooth" : "Turn on Bluetooth") : null),
        !bluetooth.available ? h(Text, { wrap: true }, bluetooth.reason || "The Bluetooth service is unavailable.") : null,
        bluetooth.operations.setDiscovery ? h(Button, { id: "settings-bluetooth-discovery", disabled: !bluetooth.powered, onClick: () => nickel.bluetooth.setDiscovery(!bluetooth.discovering) }, bluetooth.discovering ? "Stop discovery" : "Discover devices") : bluetooth.available ? h(Text, { wrap: true }, "Continuous device discovery is unavailable. Devices reported by the native service appear below.") : null,
        bluetooth.available && !bluetooth.operations.pair ? h(Text, { wrap: true }, "Pair new devices in system settings.") : null,
        bluetooth.available && !bluetooth.operations.connect && !bluetooth.operations.disconnect ? h(Text, { wrap: true }, "Manage device connections in system settings.") : null,
        bluetooth.available && !bluetooth.devices.length ? h(Text, null, bluetooth.powered ? "No devices are reported by the Bluetooth service." : "Turn on Bluetooth to use devices.") : null,
        bluetooth.devices.map(device => h(Column, { key: device.id, className: "connectivity-card" },
            h(Text, { className: "connectivity-name" }, device.name || "Unnamed device"),
            h(Text, null, device.connected ? "Connected" : device.paired ? "Paired" : "Not paired"),
            !device.paired && bluetooth.operations.pair ? h(Button, { id: "settings-bluetooth-pair/" + device.id, disabled: !bluetooth.powered, onClick: () => nickel.bluetooth.pair(device.id) }, "Pair") : null,
            device.paired && bluetooth.operations[device.connected ? "disconnect" : "connect"] ? h(Button, { id: "settings-bluetooth-connection/" + device.id, disabled: !bluetooth.powered, onClick: () => device.connected ? nickel.bluetooth.disconnect(device.id) : nickel.bluetooth.connect(device.id) }, device.connected ? "Disconnect" : "Connect") : null)));
}
registerSettingsPage({ id: "bluetooth", group: "Network", label: "Bluetooth", description: "Device pairing, discovery, and connections", component: Bluetooth });
