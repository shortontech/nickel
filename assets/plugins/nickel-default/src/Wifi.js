// @jsx h
import "./styles/connectivity.css";
export function Wifi() {
    const wifi = nickel.wifi.get();
    return h(Column, { className: "connectivity-page" },
        h(Row, { className: "connectivity-toolbar" },
            h(Text, { className: "connectivity-title" }, wifi.available ? wifi.enabled ? "Wi-Fi is on" : "Wi-Fi is off" : "Wi-Fi is unavailable"),
            wifi.operations.setEnabled ? h(Button, { id: "settings-wifi-power", onClick: () => nickel.wifi.setEnabled(!wifi.enabled) }, wifi.enabled ? "Turn off Wi-Fi" : "Turn on Wi-Fi") : null),
        !wifi.available ? h(Text, { wrap: true }, wifi.reason || "The Wi-Fi service is unavailable.") : null,
        h(Text, { className: "connectivity-description", wrap: true }, "Nickel can connect saved network profiles. Set up a new network or enter its password in system settings."),
        wifi.available && !wifi.networks.length ? h(Text, null, wifi.enabled ? "No networks are reported by the Wi-Fi service." : "Turn on Wi-Fi to view networks.") : null,
        wifi.networks.map(network => h(Column, { key: network.id, className: "connectivity-card" },
            h(Text, { className: "connectivity-name" }, network.name || "Unnamed network"),
            h(Text, null, network.connected ? "Connected" : network.saved ? "Saved profile" : "No saved profile"),
            h(Text, { className: "connectivity-description" }, "Signal: " + network.signalPercent + "%"),
            network.connected ? null : network.saved ? wifi.operations.connect ? h(Button, { id: "settings-wifi-connect/" + network.id, disabled: !network.canConnect, onClick: () => nickel.wifi.connect(network.id) }, "Connect") : h(Text, null, "Connecting is unavailable.") : h(Text, { wrap: true }, "Save this network in system settings before connecting."))));
}
registerSettingsPage({ id: "wifi", group: "Network", label: "Wi-Fi", description: "Saved network profiles and wireless connectivity", component: Wifi });
