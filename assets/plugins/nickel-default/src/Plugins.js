// @jsx h
import "./styles/plugins.css";
function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}
export function Plugins() {
    const catalog = nickel.plugins.get();
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const plugins = catalog.plugins.filter(plugin => (plugin.name + " " + plugin.id).toLowerCase().includes(needle));
    return h(Column, { className: "plugins-page" },
        h(TextField, { id: "plugins-search", accessibilityLabel: "Search plugins", placeholder: "Search plugins", value: query, onChange: setQuery }),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Plugin inventory unavailable.") : null,
        catalog.available && !catalog.writable ? h(Text, null, "Plugin management is read only.") : null,
        catalog.truncated ? h(Text, { wrap: true }, "This inventory is incomplete.") : null,
        catalog.lastResult ? h(Text, { wrap: true }, catalog.lastResult.status === "applied" ? "Plugin state updated." : catalog.lastResult.detail || "Plugin change rejected.") : null,
        catalog.available && !plugins.length ? h(Text, null, "No matching plugins.") : null,
        plugins.map((plugin, index) => h(Column, { key: plugin.id, className: "plugin-card" },
            h(Row, null,
                h(Text, { className: "plugin-title" }, plugin.name),
                h(Spacer, null),
                h(Button, { id: "plugin-toggle-" + index, disabled: !catalog.writable, accessibilityLabel: (plugin.enabled ? "Disable " : "Enable ") + plugin.name, onClick: () => plugin.enabled ? nickel.plugins.disable(plugin.id, catalog.revision) : nickel.plugins.enable(plugin.id, catalog.revision) }, plugin.enabled ? "Disable" : "Enable")),
            h(Text, { wrap: true }, plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")),
            h(Text, null, "Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")),
            h(Text, { wrap: true }, "Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")),
            h(Text, { wrap: true }, "Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")),
            plugin.composition.map((entry, entryIndex) => h(Text, { key: entryIndex, wrap: true }, entry)),
            h(Text, { wrap: true }, "JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)),
            h(Text, { wrap: true }, "Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions),
            h(Text, { wrap: true }, "Memory reports tracked package resources. Per component and total process memory are unavailable."))));
}
registerSettingsPage({ id: "plugins", group: "Shell", label: "Plugins", description: "Inspect plugin status, capabilities and memory; enable or disable packages", component: Plugins });
