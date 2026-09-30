// @jsx h
import "./styles/connectivity.css";

export function Bluetooth() {
    const bluetooth = nickel.bluetooth.get();
    return <Column className="connectivity-page">
        <Row className="connectivity-toolbar">
            <Text className="connectivity-title">{bluetooth.available ? bluetooth.powered ? "Bluetooth is on" : "Bluetooth is off" : "Bluetooth is unavailable"}</Text>
            {bluetooth.operations.setPowered ? <Button id="settings-bluetooth-power"
                onClick={() => nickel.bluetooth.setPowered(!bluetooth.powered)}>{bluetooth.powered ? "Turn off Bluetooth" : "Turn on Bluetooth"}</Button> : null}
        </Row>
        {!bluetooth.available ? <Text wrap={true}>{bluetooth.reason || "The Bluetooth service is unavailable."}</Text> : null}
        {bluetooth.operations.setDiscovery ? <Button id="settings-bluetooth-discovery" disabled={!bluetooth.powered}
            onClick={() => nickel.bluetooth.setDiscovery(!bluetooth.discovering)}>{bluetooth.discovering ? "Stop discovery" : "Discover devices"}</Button> : bluetooth.available ? <Text wrap={true}>Continuous device discovery is unavailable. Devices reported by the native service appear below.</Text> : null}
        {bluetooth.available && !bluetooth.operations.pair ? <Text wrap={true}>Pair new devices in system settings.</Text> : null}
        {bluetooth.available && !bluetooth.operations.connect && !bluetooth.operations.disconnect ? <Text wrap={true}>Manage device connections in system settings.</Text> : null}
        {bluetooth.available && !bluetooth.devices.length ? <Text>{bluetooth.powered ? "No devices are reported by the Bluetooth service." : "Turn on Bluetooth to use devices."}</Text> : null}
        {bluetooth.devices.map(device => <Column key={device.id} className="connectivity-card">
            <Text className="connectivity-name">{device.name || "Unnamed device"}</Text>
            <Text>{device.connected ? "Connected" : device.paired ? "Paired" : "Not paired"}</Text>
            {!device.paired && bluetooth.operations.pair ? <Button id={"settings-bluetooth-pair/" + device.id}
                disabled={!bluetooth.powered} onClick={() => nickel.bluetooth.pair(device.id)}>Pair</Button> : null}
            {device.paired && bluetooth.operations[device.connected ? "disconnect" : "connect"] ? <Button
                id={"settings-bluetooth-connection/" + device.id} disabled={!bluetooth.powered}
                onClick={() => device.connected ? nickel.bluetooth.disconnect(device.id) : nickel.bluetooth.connect(device.id)}>{device.connected ? "Disconnect" : "Connect"}</Button> : null}
        </Column>)}
    </Column>;
}

registerSettingsPage({id:"bluetooth",group:"Network",label:"Bluetooth",description:"Device pairing, discovery, and connections",component:Bluetooth});
