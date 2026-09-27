// @jsx h
// Network presentation. The Settings host verifies live adapter and network
// identity before it acts on these requests.
function request(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function App() {
    const data = nickel.data;
    return h("settings-stack", null,
        h("settings-row", { label: data.wifiLabel, value: data.wifiStatus },
            h("settings-switch", { id: "network-wifi-power", label: data.wifiLabel, value: data.switchState, onClick: data.powerEditable
                    ? () => request('wifi-power', { enabled: !data.wifiEnabled })
                    : undefined })),
        h("settings-card", { label: data.visibleWifi, value: "" }, data.networks.length
            ? data.networks.map(network => h("settings-row", { key: network.profile, label: network.profile, value: network.detail },
                h("settings-button", { id: `wifi-network-${network.index}`, label: network.actionLabel, value: "quiet", accessibilityLabel: `${network.profile}, ${network.detail}`, state: network.connected ? 'connected' : 'not connected', onClick: data.networkPending ? undefined
                        : () => request('wifi-network', { index: network.index, profile: network.profile }) })))
            : h("settings-row", { label: data.wifiStatus, value: "" })),
        h("settings-card", { label: data.adaptersLabel, value: "" }, data.adapters.length
            ? data.adapters.map(adapter => h("settings-row", { key: adapter.name, label: adapter.name, value: adapter.status }))
            : h("settings-row", { label: data.noAdapters, value: "" })));
}
