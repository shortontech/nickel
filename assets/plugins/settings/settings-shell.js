// @jsx h
// The native host owns Settings values and validates navigation requests.
// Page order and grouping for the ordinary Settings navigation shell.
function SettingsDestination(props) {
    const data = nickel.data;
    const header = data.headers[props.id];
    return h("settings-destination", { id: props.id, label: data.labels[props.id], value: props.section ? data.sections[props.section] : '' },
        h("settings-header", { label: header.title, value: header.subtitle }));
}
function SettingsNavigation() {
    const data = nickel.data;
    return h("settings-navigation", null,
        h(SettingsDestination, { id: "display", section: "system" }),
        h(SettingsDestination, { id: "bar", section: "personalization" }),
        h(SettingsDestination, { id: "appearance" }),
        h(SettingsDestination, { id: "network", section: "connectivity" }),
        h(SettingsDestination, { id: "bluetooth" }),
        h(SettingsDestination, { id: "bluetooth-pair" }),
        h(SettingsDestination, { id: "default-apps" }),
        h(SettingsDestination, { id: "optional-features" }),
        h(SettingsDestination, { id: "plugins" }),
        h(SettingsDestination, { id: "keyboard-shortcuts", section: "support" }),
        h(SettingsDestination, { id: "about" }),
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
function App() {
    const data = nickel.data || {};
    const wide = (data.width || 1100) >= 720;
    const pairing = !!data.pairing;
    const showNavigation = !pairing && (wide || !data.active);
    const showContent = pairing || wide || data.active;
    const request = (type, fields) => nickel.request(Object.assign({ type }, fields || {}));
    return h(Window, { id: "main", title: pairing ? "Pair Bluetooth devices" : "Nickel Settings", width: "100%", height: "100%", className: "settings-window" },
        h("div", { className: pairing ? "settings-shell pairing" : wide ? "settings-shell wide" : "settings-shell narrow" },
            showNavigation ? h("div", { className: wide ? "settings-sidebar" : "settings-sidebar narrow" },
                h(TextField, { id: "settings-sidebar-search", className: "settings-search", value: data.query || "", placeholder: data.searchPlaceholder || "Search Settings", onChange: value => request("search", { value }) }),
                h(ScrollView, { id: "settings-sidebar-scroll", height: Math.max(1, (data.height || 800) - 64) },
                    h(Column, { className: "settings-destinations" },
                        data.query ? (data.results || []).map(result => h(Button, { key: result.target, id: "search-result-" + result.target, disabled: !result.available, className: "settings-destination", onClick: () => request("navigate-target", { target: result.target }) }, result.label)) : (data.destinations || []).map(destination => h(Column, { key: destination.id },
                            destination.section ? h(Text, { className: "settings-section" }, destination.section) : null,
                            h(Button, { id: "settings-navigation/destination/" + destination.id, state: destination.active ? "selected" : "unselected", className: destination.active ? "settings-destination active" : "settings-destination", onClick: () => request("navigate", { page: destination.id }) }, destination.label))),
                        data.query && !(data.results || []).length ? h(Text, { className: "settings-empty" }, data.noResults || "No results") : null))) : null,
            showContent ? h("div", { className: pairing ? "settings-detail pairing" : "settings-detail" },
                h(Row, { className: "settings-heading" },
                    !wide && !pairing ? h(Button, { id: "settings-show-navigation", className: "settings-back", onClick: () => request("show-navigation") }, "\u2039") : null,
                    h(Column, null,
                        h(Text, { className: "settings-title" }, data.title || "Settings"),
                        data.subtitle ? h(Text, { className: "settings-subtitle", wrap: true }, data.subtitle) : null)),
                h(Slot, { id: "settings-content", className: "settings-content" })) : null));
}
App.navigation = SettingsNavigation;
