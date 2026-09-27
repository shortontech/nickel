// @jsx h
// Page order and grouping for the ordinary Settings navigation shell.
function Destination(props) {
    const data = nickel.data;
    const header = data.headers[props.id];
    return h("settings-destination", { id: props.id, label: data.labels[props.id], value: props.section ? data.sections[props.section] : '' },
        h("settings-header", { label: header.title, value: header.subtitle }));
}
function App() {
    const data = nickel.data;
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
        h(Destination, { id: "about" }),
        h("settings-search-index", null,
            h("settings-search-entry", { id: "appearance-mode-system", state: data.labels.appearance, label: data.search.automatic, value: data.search.mode }),
            h("settings-search-entry", { id: "appearance-hue", state: data.labels.appearance, label: data.search.startingHue, value: data.search.interface }),
            h("settings-search-entry", { id: "appearance-intensity", state: data.labels.appearance, label: data.search.colorIntensity, value: data.search.interface }),
            h("settings-search-entry", { id: "appearance-transparency", state: data.labels.appearance, label: data.search.reduceTransparency, value: data.search.interface }),
            h("settings-search-entry", { id: "appearance-animations", state: data.labels.appearance, label: data.search.animations, value: data.search.interface }),
            h("settings-search-entry", { id: "optional-feature-codex-enabled", state: data.labels['optional-features'], label: "Use Codex projects and conversations in Nickel", value: "Codex" }),
            h("settings-search-entry", { id: "on-screen-keyboard-mode", state: data.labels['optional-features'], label: "Screen keyboard \u00B7 touch keyboard \u00B7 virtual keyboard", value: "On-screen keyboard" }),
            h("settings-search-entry", { id: "plugins-page", state: data.labels.plugins, label: "Enable or disable shell plugins and review their access", value: "Plugin memory and permissions" })));
}
