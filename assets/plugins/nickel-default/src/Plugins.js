// @jsx h
import "./styles/plugins.css";
const pluginKey = plugin => plugin.id;
const settingKey = setting => setting.id;
const compositionKey = (_, index) => String(index);
const pluginHeight = plugin => Math.min(8192, 300 + plugin.composition.length * 28 + (plugin.settings || []).length * 128);
const settingHeight = setting => setting.description ? 140 : 100;
function settingType(setting) {
    const kind = setting.kind.kind;
    return kind === "boolean" ? "switch" : kind === "integer" ? "number" : kind === "choice" ? "select" : "text";
}
function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}
function PluginSetting({ plugin, setting, revision, writable, draftScope }) {
    const Control = nickel.component("shell.settings.controls");
    const kind = setting.kind;
    const type = settingType(setting);
    return h(Column, { className: "plugin-setting" },
        h(Text, null, setting.label),
        setting.description ? h(Text, { wrap: true }, setting.description) : null,
        writable ? h(Control, { controlId: "plugin-setting/" + plugin.id + "/" + setting.id, draftState: draftScope.controls[setting.id]?.type === type ? draftScope.controls[setting.id].state : undefined, onDraftChange: next => {
                draftScope.controls[setting.id] = { type, state: next };
            }, setting: { id: setting.id, providerPackage: plugin.id, label: setting.label, type, value: setting.value,
                min: kind.min, max: kind.max, maxLength: kind.max_length,
                options: (kind.options || []).map(option => ({ label: option, value: option })),
                onChange: value => nickel.plugins.setSetting(plugin.id, setting.id, value, revision) } }) : h(Text, null, String(setting.value)));
}
function PluginCard({ plugin, index, revision, writable, canManage, setReview, draftScope }) {
    return h(Column, { className: "plugin-card" },
        h(Row, null,
            h(Text, { className: "plugin-title" }, plugin.name),
            h(Spacer, null),
            h(Button, { id: "plugin-toggle-" + index, disabled: !canManage, accessibilityLabel: (plugin.enabled ? "Disable " : "Enable ") + plugin.name, onClick: () => plugin.enabled ? nickel.plugins.disable(plugin.id, revision) : setReview({ id: plugin.id, revision }) }, plugin.enabled ? "Disable" : "Enable")),
        h(Text, { wrap: true }, plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")),
        h(Text, { wrap: true }, "Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")),
        h(Text, { wrap: true }, "Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")),
        h(Text, { wrap: true }, "Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")),
        plugin.composition.length ? h(VirtualColumn, { id: "plugin-composition/" + plugin.id, items: plugin.composition, itemKey: compositionKey, itemHeight: 20, gap: 8, overscan: 96, renderItem: entry => h(Text, { wrap: true }, entry) }) : null,
        h(Text, { wrap: true }, "JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)),
        h(Text, { wrap: true }, "Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions),
        plugin.settings?.length ? h(VirtualColumn, { id: "plugin-settings/" + plugin.id, items: plugin.settings, itemKey: settingKey, itemHeight: settingHeight, gap: 8, overscan: 96, renderItem: setting => h(PluginSetting, { key: setting.id, plugin: plugin, setting: setting, revision: revision, writable: writable, draftScope: draftScope }) }) : null,
        h(Text, { wrap: true }, "Memory reports tracked package resources. Per component and total process memory are unavailable."));
}
export function Plugins() {
    const catalog = nickel.plugins.get();
    const preview = catalog.shellPreview;
    const canManage = catalog.available && catalog.writable && !preview;
    const [query, setQuery] = useState("");
    const [review, setReview] = useState(null);
    const draftScopes = useRef(Object.create(null));
    // Draft ownership outlives materialized cards, but removed plugin/setting
    // identities and changed editor types must retire their state offscreen too.
    useMemo(() => {
        const present = Object.create(null);
        for (const plugin of catalog.plugins) {
            present[plugin.id] = true;
            const scope = draftScopes.current[plugin.id] || (draftScopes.current[plugin.id] = {
                controls: Object.create(null),
            });
            const paths = Object.create(null);
            for (const setting of plugin.settings || []) {
                const type = settingType(setting);
                paths[setting.id] = type;
            }
            for (const id of Object.keys(scope.controls))
                if (paths[id] !== scope.controls[id].type)
                    delete scope.controls[id];
        }
        for (const id of Object.keys(draftScopes.current))
            if (!present[id])
                delete draftScopes.current[id];
    }, [catalog.plugins]);
    const candidate = review && catalog.plugins.find(plugin => plugin.id === review.id);
    const reviewCurrent = candidate && !candidate.enabled && review.revision === catalog.revision;
    const needle = query.trim().toLowerCase();
    const plugins = useMemo(() => catalog.plugins.filter(plugin => !plugin.shell && (plugin.name + " " + plugin.id).toLowerCase().includes(needle)), [catalog.plugins, needle]);
    return h(Column, { className: "plugins-page" },
        h(TextField, { id: "plugins-search", accessibilityLabel: "Search plugins", placeholder: "Search plugins", value: query, onChange: setQuery }),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Plugin inventory unavailable.") : null,
        catalog.available && !catalog.writable ? h(Text, null, "Plugin management is read only.") : null,
        catalog.truncated ? h(Text, { wrap: true }, "This inventory is incomplete.") : null,
        catalog.lastResult ? h(Text, { wrap: true }, catalog.lastResult.detail || ({ applied: "Plugin state updated.", preview: "Shell preview started.", confirmed: "Shell selection saved.", reverted: "Previous shell restored.", rejected: "Plugin change rejected." }[catalog.lastResult.status] || "Plugin operation: " + catalog.lastResult.status)) : null,
        preview ? h(Text, { wrap: true }, "Finish the temporary shell preview in the Shell setting before changing extensions.") : null,
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
                h(Button, { id: "plugin-review-confirm", disabled: !canManage || !reviewCurrent, onClick: () => { nickel.plugins.enable(candidate.id, review.revision); setReview(null); } }, "Enable plugin"))) : null,
        catalog.available && !plugins.length ? h(Text, null, "No matching extensions.") : null,
        h(VirtualColumn, { id: "settings-plugins", items: plugins, itemKey: pluginKey, itemHeight: pluginHeight, gap: 12, overscan: 96, renderItem: (plugin, index) => h(PluginCard, { key: plugin.id, plugin: plugin, index: index, revision: catalog.revision, writable: catalog.writable, canManage: canManage, setReview: setReview, draftScope: draftScopes.current[plugin.id] }) }));
}
registerSettingsPage({ id: "plugins", group: "Shell", label: "Plugins", description: "Inspect extension status, capabilities and memory; enable or disable packages", component: Plugins });
