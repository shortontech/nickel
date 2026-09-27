// @jsx h
// Ordinary plugin status and settings UI. Permission approval stays with the host.
function action(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function Detail(props) {
    return h("settings-row", { label: props.label, value: props.value });
}
function Setting(props) {
    const plugin = props.plugin;
    const setting = props.setting;
    const labels = props.labels;
    const label = setting.displayLabel;
    const base = { id: plugin.id, key: setting.id };
    let control = null;
    const kind = setting.kind.kind;
    if (kind === 'boolean') {
        control = h("settings-switch", { id: `plugin-setting-${plugin.id}-${setting.id}`, label: label, value: setting.value ? 'on' : 'off', onClick: setting.pending ? undefined : () => action('set-setting', { ...base, value: !setting.value }) });
    }
    else if (kind === 'integer') {
        control = h("settings-inline", null,
            h("settings-button", { label: "\u2212", value: "quiet", onClick: setting.pending ? undefined : () => action('step-setting', { ...base, direction: 'decrement' }) }),
            h("settings-button", { label: "+", value: "quiet", onClick: setting.pending ? undefined : () => action('step-setting', { ...base, direction: 'increment' }) }));
    }
    else if (kind === 'choice' && setting.kind.options.length) {
        const index = setting.kind.options.indexOf(setting.value);
        const next = setting.kind.options[(index + 1) % setting.kind.options.length];
        control = h("settings-button", { label: labels.change, value: "quiet", onClick: setting.pending ? undefined : () => action('set-setting', { ...base, value: next }) });
    }
    else if (kind === 'text') {
        control = h("settings-button", { label: labels.edit, value: "quiet", onClick: setting.pending ? undefined : () => action('edit-text', base) });
    }
    return h("settings-fragment", null,
        h("settings-row", { label: label, value: kind === 'boolean' ? setting.description : setting.displayValue }, control),
        props.editing ? h("settings-inline", null,
            h("settings-input", { id: `plugin-setting-text-${plugin.id}-${setting.id}`, value: props.draft, onChange: value => action('text-changed', { value }) }),
            h("settings-button", { label: labels.save, value: "primary", onClick: () => action('save-text') }),
            h("settings-button", { label: labels.cancel, value: "quiet", onClick: () => action('cancel-text') })) : null);
}
function Plugin(props) {
    const plugin = props.plugin;
    const labels = props.labels;
    return h("settings-card", { label: plugin.name, value: plugin.id },
        h(Detail, { label: labels.publisher, value: plugin.author }),
        h(Detail, { label: labels.version, value: plugin.version }),
        h("settings-row", { label: labels.enabled, value: plugin.health },
            h("settings-switch", { id: `plugin-enable-${plugin.id}`, label: plugin.toggleLabel, value: plugin.switchState, onClick: plugin.pending ? undefined : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', { id: plugin.id }) })),
        h(Detail, { label: labels.access, value: plugin.access }),
        h(Detail, { label: labels.surfaces, value: plugin.surfaces }),
        h(Detail, { label: labels.composition, value: plugin.composition }),
        h(Detail, { label: labels.trackedMemory, value: plugin.memory.tracked }),
        h(Detail, { label: labels.peakMemory, value: plugin.memory.peak }),
        h(Detail, { label: labels.jsHeap, value: plugin.memory.js }),
        h(Detail, { label: labels.nativeUi, value: plugin.memory.native }),
        h(Detail, { label: labels.textures, value: plugin.memory.textures }),
        h(Detail, { label: labels.timers, value: plugin.memory.timers }),
        plugin.memory.overlap ? h(Detail, { label: labels.memoryAttribution, value: labels.extensionOverlap }) : null,
        plugin.settings.map(setting => h(Setting, { key: setting.id, plugin: plugin, labels: labels, setting: setting, editing: setting.editing, draft: setting.draft })));
}
function App() {
    const data = nickel.data;
    const labels = data.labels;
    return h("settings-stack", null,
        data.notice ? h("settings-card", { label: labels.pluginStatus, value: data.notice }) : null,
        data.available
            ? data.plugins.map(plugin => h(Plugin, { key: plugin.id, plugin: plugin, labels: labels }))
            : h("settings-card", { label: labels.waiting, value: labels.statusUnavailable },
                h("settings-button", { label: labels.refresh, value: "secondary", onClick: () => action('refresh') })));
}
