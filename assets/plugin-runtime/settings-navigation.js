// @jsx h
// Page order and grouping for the ordinary Settings navigation shell.
function Destination(props) {
    const data = nickel.data;
    const header = data.headers[props.id];
    return h("settings-destination", { id: props.id, label: data.labels[props.id], value: props.section ? data.sections[props.section] : '' },
        h("settings-header", { label: header.title, value: header.subtitle }));
}
function App() {
    return h("settings-navigation", null,
        h(Destination, { id: "display", section: "system" }),
        h(Destination, { id: "bar", section: "personalization" }),
        h(Destination, { id: "appearance" }),
        h(Destination, { id: "network", section: "connectivity" }),
        h(Destination, { id: "bluetooth" }),
        h(Destination, { id: "bluetooth-pair" }),
        h(Destination, { id: "default-apps" }),
        h(Destination, { id: "optional-features" }),
        h(Destination, { id: "plugins" }),
        h(Destination, { id: "keyboard-shortcuts", section: "support" }),
        h(Destination, { id: "about" }));
}
