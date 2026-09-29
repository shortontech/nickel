// @jsx h
// The host checks current adapter and device identity before applying requests.
function request(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function App() {
    const data = nickel.data;
    return h("div", { className: data.pairing ? 'bluetooth-page pairing' : 'bluetooth-page' },
        !data.pairing && h("div", { className: "bluetooth-card" },
            h("div", { className: "bluetooth-row" },
                h("div", { className: "bluetooth-label" },
                    h(Text, { className: "bluetooth-title" }, data.powerLabel),
                    h(Text, { className: "bluetooth-detail" }, data.adapterName)),
                h(Switch, { id: "bluetooth-power", className: `bluetooth-switch ${data.switchState}`, accessibilityLabel: data.powerLabel, state: data.switchState, onClick: data.powerEditable
                        ? () => request('power', { enabled: !data.powered })
                        : undefined }))),
        h("div", { className: "bluetooth-card" },
            h(Text, { className: "bluetooth-title" }, data.statusLabel),
            h(Text, { className: "bluetooth-detail", wrap: true }, data.status)),
        h("div", { className: "bluetooth-card" },
            h(Text, { className: "bluetooth-title" }, data.devicesLabel),
            h("div", { className: "bluetooth-row" },
                h(Text, { className: "bluetooth-detail" }, data.discoveryLabel),
                data.discoveryEditable
                    ? h(Button, { id: "bluetooth-discovery-action", className: "bluetooth-action", onClick: () => request(data.pairing ? 'discovery' : 'open-pairing') }, data.discoveryLabel)
                    : h(Text, { className: "bluetooth-detail" }, data.discoveryLabel)),
            data.devices.length
                ? data.devices.map(device => h("div", { key: device.id, className: "bluetooth-row" },
                    h("div", { className: "bluetooth-label" },
                        h(Text, { className: "bluetooth-name" }, device.name),
                        h(Text, { className: "bluetooth-detail", wrap: true }, device.detail)),
                    data.deviceEditable
                        ? h(Button, { id: `bluetooth-device-${device.index}-action`, className: "bluetooth-action", onClick: () => request('device', { index: device.index, id: device.id }) }, device.actionLabel)
                        : h(Text, { className: "bluetooth-detail" }, device.actionLabel)))
                : h(Text, { className: "bluetooth-detail" }, data.emptyLabel)));
}
