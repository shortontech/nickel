// @jsx h
function App() {
    const widgets = nickel.data.slots.metrics || [];
    const actions = nickel.data.slots.commands || [];
    return h(Window, { width: 420, height: 280, className: "widget-host" },
        h(Column, null,
            h(Text, null, "Widget host"),
            actions.map(item => h(Button, { key: item.pluginId + ':' + item.id, id: 'slot-action-' + item.id, onClick: () => nickel.request({ type: 'invoke-plugin-slot-action',
                    slot: 'commands', pluginId: item.pluginId, id: item.id }) }, item.label)),
            widgets.length === 0 ? h(Text, null, "No metrics yet") : null,
            widgets.map((item, index) => h(Row, { key: item.pluginId + ':' + index },
                h(Text, null, item.label),
                h(Text, null, item.value)))));
}
