// @jsx h
function App() {
    const widgets = nickel.data.slots.metrics || [];
    return h(Panel, { height: 280, background: 0xff202830 },
        h(Column, null,
            h(Text, null, "Widget host"),
            widgets.length === 0 ? h(Text, null, "No metrics yet") : null,
            widgets.map((item, index) => h(Row, { key: item.pluginId + ':' + index },
                h(Text, null, item.label),
                h(Text, null, item.value)))));
}
