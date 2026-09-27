// @jsx h
// Network presentation. The Settings host verifies live adapter and network
// identity before it acts on these requests.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    return <settings-stack>
        <settings-row label={data.wifiLabel} value={data.wifiStatus}>
            <settings-switch id="network-wifi-power" label={data.wifiLabel}
                value={data.switchState}
                onClick={data.powerEditable
                    ? () => request('wifi-power', {enabled: !data.wifiEnabled})
                    : undefined} />
        </settings-row>
        <settings-card label={data.visibleWifi} value="">
            {data.networks.length
                ? data.networks.map(network =>
                    <settings-row key={network.profile} label={network.profile} value={network.detail}>
                        <settings-button id={`wifi-network-${network.index}`}
                            label={network.actionLabel} value="quiet"
                            accessibilityLabel={`${network.profile}, ${network.detail}`}
                            state={network.connected ? 'connected' : 'not connected'}
                            onClick={data.networkPending ? undefined
                                : () => request('wifi-network', {index: network.index, profile: network.profile})} />
                    </settings-row>)
                : <settings-row label={data.wifiStatus} value="" />}
        </settings-card>
        <settings-card label={data.adaptersLabel} value="">
            {data.adapters.length
                ? data.adapters.map(adapter => <settings-row key={adapter.name}
                    label={adapter.name} value={adapter.status} />)
                : <settings-row label={data.noAdapters} value="" />}
        </settings-card>
    </settings-stack>;
}
