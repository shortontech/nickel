// @jsx h
// The host verifies current adapter and network identity before applying requests.
function request(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function App() {
    const data = nickel.data;
    return h("div", { className: "network-page" },
        h("div", { className: "network-card" },
            h("div", { className: "network-power-row" },
                h("div", { className: "network-power-label" },
                    h(Text, { className: "network-title" }, data.wifiLabel),
                    h(Text, { className: "network-detail", wrap: true }, data.wifiStatus)),
                h(Switch, { id: "network-wifi-power", className: `network-switch ${data.switchState}`, accessibilityLabel: data.wifiLabel, state: data.switchState, onClick: data.powerEditable
                        ? () => request('wifi-power', { enabled: !data.wifiEnabled })
                        : undefined }))),
        h("div", { className: "network-card" },
            h(Text, { className: "network-title" }, data.visibleWifi),
            data.networks.length
                ? data.networks.map(network => h("div", { key: network.profile, className: "network-row" },
                    h("div", { className: "network-label" },
                        h(Text, { className: "network-name" }, network.profile),
                        h(Text, { className: "network-detail", wrap: true }, network.detail)),
                    data.networkPending
                        ? h(Text, { className: "network-detail" }, network.actionLabel)
                        : h(Button, { id: `wifi-network-${network.index}`, className: "network-action", accessibilityLabel: `${network.profile}, ${network.detail}`, state: network.connected ? 'connected' : 'not connected', onClick: () => request('wifi-network', { index: network.index, profile: network.profile }) }, network.actionLabel)))
                : h(Text, { className: "network-detail", wrap: true }, data.wifiStatus)),
        h("div", { className: "network-card" },
            h(Text, { className: "network-title" }, data.adaptersLabel),
            data.adapters.length
                ? data.adapters.map(adapter => h("div", { key: adapter.name, className: "network-label" },
                    h(Text, { className: "network-name" }, adapter.name),
                    h(Text, { className: "network-detail", wrap: true }, adapter.status)))
                : h(Text, { className: "network-detail" }, data.noAdapters)));
}
