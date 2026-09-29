// @jsx h
// The host verifies current adapter and network identity before applying requests.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    return <div className="network-page">
        <div className="network-card">
            <div className="network-power-row">
                <div className="network-power-label">
                    <Text className="network-title">{data.wifiLabel}</Text>
                    <Text className="network-detail" wrap={true}>{data.wifiStatus}</Text>
                </div>
                <Switch id="network-wifi-power" className={`network-switch ${data.switchState}`}
                    accessibilityLabel={data.wifiLabel} state={data.switchState}
                onClick={data.powerEditable
                    ? () => request('wifi-power', {enabled: !data.wifiEnabled})
                    : undefined} />
            </div>
        </div>
        <div className="network-card">
            <Text className="network-title">{data.visibleWifi}</Text>
            {data.networks.length
                ? data.networks.map(network => <div key={network.profile} className="network-row">
                    <div className="network-label">
                        <Text className="network-name">{network.profile}</Text>
                        <Text className="network-detail" wrap={true}>{network.detail}</Text>
                    </div>
                    {data.networkPending
                        ? <Text className="network-detail">{network.actionLabel}</Text>
                        : <Button id={`wifi-network-${network.index}`} className="network-action"
                            accessibilityLabel={`${network.profile}, ${network.detail}`}
                            state={network.connected ? 'connected' : 'not connected'}
                            onClick={() => request('wifi-network', {index: network.index, profile: network.profile})}>{network.actionLabel}</Button>}
                </div>)
                : <Text className="network-detail" wrap={true}>{data.wifiStatus}</Text>}
        </div>
        <div className="network-card">
            <Text className="network-title">{data.adaptersLabel}</Text>
            {data.adapters.length
                ? data.adapters.map(adapter => <div key={adapter.name} className="network-label">
                    <Text className="network-name">{adapter.name}</Text>
                    <Text className="network-detail" wrap={true}>{adapter.status}</Text>
                </div>)
                : <Text className="network-detail">{data.noAdapters}</Text>}
        </div>
    </div>;
}
