// @jsx h
import "./styles/connectivity.css";

export function Wifi() {
    const wifi = nickel.wifi.get();
    return <Column className="connectivity-page">
        <Row className="connectivity-toolbar">
            <Text className="connectivity-title">{wifi.available ? wifi.enabled ? "Wi-Fi is on" : "Wi-Fi is off" : "Wi-Fi is unavailable"}</Text>
            {wifi.operations.setEnabled ? <Button id="settings-wifi-power"
                onClick={() => nickel.wifi.setEnabled(!wifi.enabled)}>{wifi.enabled ? "Turn off Wi-Fi" : "Turn on Wi-Fi"}</Button> : null}
        </Row>
        {wifi.available && wifi.writable === false ? <Text>Wi-Fi controls are read only.</Text> : null}
        {!wifi.available ? <Text wrap={true}>{wifi.reason || "The Wi-Fi service is unavailable."}</Text> : null}
        <Text className="connectivity-description" wrap={true}>Nickel can connect saved network profiles. Set up a new network or enter its password in system settings.</Text>
        {wifi.available && !wifi.networks.length ? <Text>{wifi.enabled ? "No networks are reported by the Wi-Fi service." : "Turn on Wi-Fi to view networks."}</Text> : null}
        {wifi.networks.map(network => <Column key={network.id} className="connectivity-card">
            <Text className="connectivity-name">{network.name || "Unnamed network"}</Text>
            <Text>{network.connected ? "Connected" : network.saved ? "Saved profile" : "No saved profile"}</Text>
            <Text className="connectivity-description">{"Signal: " + network.signalPercent + "%"}</Text>
            {network.connected ? wifi.operations.disconnect ? <Button id={"settings-wifi-disconnect/" + network.id} disabled={!network.canDisconnect} onClick={() => nickel.wifi.disconnect(network.id)}>Disconnect</Button> : null : network.saved ? wifi.operations.connect ? <Button
                id={"settings-wifi-connect/" + network.id} disabled={!network.canConnect}
                onClick={() => nickel.wifi.connect(network.id)}>Connect</Button> : <Text>Connecting is unavailable.</Text> : <Text wrap={true}>Save this network in system settings before connecting.</Text>}
        </Column>)}
        <Text className="connectivity-title">Network adapters</Text>
        {!wifi.adaptersAvailable ? <Text>Network adapter inventory is unavailable.</Text> : !(wifi.adapters || []).length ? <Text>No network adapters are reported.</Text> : null}
        {(wifi.adapters || []).map((adapter,index) => <Column key={index} className="connectivity-card"><Text>{adapter.name}</Text><Text wrap={true}>{adapter.description + " · " + (adapter.connected ? "Connected" : "Disconnected") + (adapter.speedBitsPerSecond ? " · " + Math.round(adapter.speedBitsPerSecond / 1000000) + " Mbps" : "")}</Text></Column>)}
    </Column>;
}

registerSettingsPage({id:"wifi",group:"Network",label:"Wi-Fi",description:"Saved network profiles and wireless connectivity",component:Wifi});
