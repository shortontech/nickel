// @jsx h
import "./styles/plugins.css";
function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}
function PluginSetting({ plugin, setting, revision, writable }) {
    const Control = nickel.component("shell.settings.controls");
    const kind = setting.kind;
    const type = kind.kind === "boolean" ? "switch" : kind.kind === "integer" ? "number" : kind.kind === "choice" ? "select" : "text";
    return h(Column, { className: "plugin-setting" },
        h(Text, null, setting.label),
        setting.description ? h(Text, { wrap: true }, setting.description) : null,
        writable ? h(Control, { controlId: "plugin-setting/" + plugin.id + "/" + setting.id, setting: { id: setting.id, providerPackage: plugin.id, label: setting.label, type, value: setting.value,
                min: kind.min, max: kind.max, maxLength: kind.max_length,
                options: (kind.options || []).map(option => ({ label: option, value: option })),
                onChange: value => nickel.plugins.setSetting(plugin.id, setting.id, value, revision) } }) : h(Text, null, String(setting.value)));
}
export function Plugins() {
    const catalog = nickel.plugins.get();
    const preview = catalog.shellPreview;
    const canManage = catalog.available && catalog.writable && !preview;
    const shellName = id => catalog.plugins.find(plugin => plugin.id === id)?.name || id;
    const [query, setQuery] = useState("");
    const [review, setReview] = useState(null);
    const candidate = review && catalog.plugins.find(plugin => plugin.id === review.id);
    const reviewCurrent = candidate && !candidate.enabled && review.revision === catalog.revision;
    const needle = query.trim().toLowerCase();
    const plugins = catalog.plugins.filter(plugin => (plugin.name + " " + plugin.id).toLowerCase().includes(needle));
    return h(Column, { className: "plugins-page" },
        h(TextField, { id: "plugins-search", accessibilityLabel: "Search plugins", placeholder: "Search plugins", value: query, onChange: setQuery }),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Plugin inventory unavailable.") : null,
        catalog.available && !catalog.writable ? h(Text, null, "Plugin management is read only.") : null,
        catalog.truncated ? h(Text, { wrap: true }, "This inventory is incomplete.") : null,
        catalog.lastResult ? h(Text, { wrap: true }, catalog.lastResult.detail || ({ applied: "Plugin state updated.", preview: "Shell preview started.", confirmed: "Shell selection saved.", reverted: "Previous shell restored.", rejected: "Plugin change rejected." }[catalog.lastResult.status] || "Plugin operation: " + catalog.lastResult.status)) : null,
        preview ? h(Column, { className: "plugin-card" },
            h(Text, { className: "plugin-title" }, "Temporary shell preview"),
            h(Text, { wrap: true }, "Previewing " + shellName(preview.selectedShell) + ". Previous shell: " + shellName(preview.previousShell) + "."),
            h(Text, { wrap: true }, "Keep this shell before the recovery timer expires, or restore the previous shell. Unconfirmed previews revert automatically."),
            h(Row, null,
                h(Button, { id: "plugin-shell-confirm", disabled: !catalog.available || !catalog.writable || !preview.canConfirm, onClick: () => nickel.plugins.confirmShell(preview.token, catalog.revision) }, "Keep this shell"),
                h(Button, { id: "plugin-shell-revert", disabled: !catalog.available || !catalog.writable || !preview.canRevert, onClick: () => nickel.plugins.revertShell(preview.token, catalog.revision) }, "Restore previous shell")),
            h(Text, { wrap: true }, "Finish this preview before changing plugin activation or selecting another shell.")) : null,
        review ? h(Column, { className: "plugin-card" },
            h(Text, { className: "plugin-title" }, "Review plugin access"),
            candidate ? h(Column, null,
                h(Text, null, candidate.name + " · " + (candidate.version || "Unspecified version")),
                h(Text, null, "Publisher: " + (candidate.author || "Unknown")),
                h(Text, { wrap: true }, "Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")),
                h(Text, { wrap: true }, "Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")),
                h(Text, { wrap: true }, "Composition changes: " + (candidate.composition.length ? candidate.composition.join(", ") : "None"))) : null,
            !reviewCurrent ? h(Text, { wrap: true }, "Plugin access changed. Cancel and review it again before enabling.") : null,
            h(Row, null,
                h(Button, { id: "plugin-review-cancel", onClick: () => setReview(null) }, "Cancel"),
                h(Button, { id: "plugin-review-confirm", disabled: !canManage || !reviewCurrent, onClick: () => { review.selectShell ? nickel.plugins.selectShell(candidate.id, review.revision) : nickel.plugins.enable(candidate.id, review.revision); setReview(null); } }, review.selectShell ? "Enable and preview shell" : "Enable plugin"))) : null,
        catalog.available && !plugins.length ? h(Text, null, "No matching plugins.") : null,
        plugins.map((plugin, index) => h(Column, { key: plugin.id, className: "plugin-card" },
            h(Row, null,
                h(Text, { className: "plugin-title" }, plugin.name),
                h(Spacer, null),
                h(Button, { id: "plugin-toggle-" + index, disabled: !canManage, accessibilityLabel: (plugin.enabled ? "Disable " : "Enable ") + plugin.name, onClick: () => plugin.enabled ? nickel.plugins.disable(plugin.id, catalog.revision) : setReview({ id: plugin.id, revision: catalog.revision }) }, plugin.enabled ? "Disable" : "Enable")),
            plugin.shell ? plugin.selected ? h(Text, null, "Selected shell") : h(Button, { id: "plugin-preview-shell-" + index, disabled: !canManage, onClick: () => plugin.enabled ? nickel.plugins.selectShell(plugin.id, catalog.revision) : setReview({ id: plugin.id, revision: catalog.revision, selectShell: true }) }, "Preview shell") : null,
            h(Text, { wrap: true }, plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")),
            h(Text, { wrap: true }, "Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")),
            h(Text, { wrap: true }, "Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")),
            h(Text, { wrap: true }, "Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")),
            plugin.composition.map((entry, entryIndex) => h(Text, { key: entryIndex, wrap: true }, entry)),
            h(Text, { wrap: true }, "JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)),
            h(Text, { wrap: true }, "Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions),
            (plugin.settings || []).map(setting => h(PluginSetting, { key: setting.id, plugin: plugin, setting: setting, revision: catalog.revision, writable: catalog.writable })),
            h(Text, { wrap: true }, "Memory reports tracked package resources. Per component and total process memory are unavailable."))));
}
registerSettingsPage({ id: "plugins", group: "Shell", label: "Plugins", description: "Inspect plugin status, capabilities and memory; enable or disable packages", component: Plugins });
