// @jsx h
// Bluetooth presentation. The host checks the live adapter and device before acting.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    return <settings-stack>
        {!data.pairing && <settings-row label={data.powerLabel} value={data.adapterName}>
            <settings-switch id="bluetooth-power" label={data.powerLabel}
                value={data.switchState}
                onClick={data.powerEditable
                    ? () => request('power', {enabled: !data.powered})
                    : undefined} />
        </settings-row>}
        <settings-card label={data.statusLabel} value={data.status} />
        <settings-card label={data.devicesLabel} value="">
            <settings-row label={data.discoveryLabel} value="">
                <settings-button id="bluetooth-discovery-action"
                    label={data.discoveryLabel} value="secondary"
                    onClick={data.discoveryEditable
                        ? () => request(data.pairing ? 'discovery' : 'open-pairing')
                        : undefined} />
            </settings-row>
            {data.devices.length
                ? data.devices.map(device =>
                    <settings-row key={device.id} label={device.name} value={device.detail}>
                        <settings-button id={`bluetooth-device-${device.index}-action`}
                            label={device.actionLabel} value="secondary"
                            onClick={data.deviceEditable
                                ? () => request('device', {index: device.index, id: device.id})
                                : undefined} />
                    </settings-row>)
                : <settings-row label={data.emptyLabel} value="" />}
        </settings-card>
    </settings-stack>;
}
