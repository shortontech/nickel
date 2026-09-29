// @jsx h
// The host checks current adapter and device identity before applying requests.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    return <div className={data.pairing ? 'bluetooth-page pairing' : 'bluetooth-page'}>
        {!data.pairing && <div className="bluetooth-card">
            <div className="bluetooth-row">
                <div className="bluetooth-label">
                    <Text className="bluetooth-title">{data.powerLabel}</Text>
                    <Text className="bluetooth-detail">{data.adapterName}</Text>
                </div>
                <Switch id="bluetooth-power" className={`bluetooth-switch ${data.switchState}`}
                    accessibilityLabel={data.powerLabel} state={data.switchState}
                    onClick={data.powerEditable
                        ? () => request('power', {enabled: !data.powered})
                        : undefined} />
            </div>
        </div>}
        <div className="bluetooth-card">
            <Text className="bluetooth-title">{data.statusLabel}</Text>
            <Text className="bluetooth-detail" wrap={true}>{data.status}</Text>
        </div>
        <div className="bluetooth-card">
            <Text className="bluetooth-title">{data.devicesLabel}</Text>
            <div className="bluetooth-row">
                <Text className="bluetooth-detail">{data.discoveryLabel}</Text>
                {data.discoveryEditable
                    ? <Button id="bluetooth-discovery-action" className="bluetooth-action"
                        onClick={() => request(data.pairing ? 'discovery' : 'open-pairing')}>{data.discoveryLabel}</Button>
                    : <Text className="bluetooth-detail">{data.discoveryLabel}</Text>}
            </div>
            {data.devices.length
                ? data.devices.map(device => <div key={device.id} className="bluetooth-row">
                    <div className="bluetooth-label">
                        <Text className="bluetooth-name">{device.name}</Text>
                        <Text className="bluetooth-detail" wrap={true}>{device.detail}</Text>
                    </div>
                    {data.deviceEditable
                        ? <Button id={`bluetooth-device-${device.index}-action`}
                            className="bluetooth-action"
                            onClick={() => request('device', {index: device.index, id: device.id})}>{device.actionLabel}</Button>
                        : <Text className="bluetooth-detail">{device.actionLabel}</Text>}
                </div>)
                : <Text className="bluetooth-detail">{data.emptyLabel}</Text>}
        </div>
    </div>;
}
