// @jsx h
// Ordinary plugin status and settings UI. Permission approval stays with the host.
function action(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function Detail({ label, value }) {
    return h("div", { className: "plugin-detail" },
        h(Text, { className: "plugin-detail-label detail-label" }, label),
        h(Text, { className: "plugin-detail-value detail-value", wrap: true }, value));
}
function Setting({ plugin, setting, labels }) {
    const base = { id: plugin.id, key: setting.id };
    const kind = setting.kind.kind;
    let control = null;
    if (kind === 'boolean') {
        const state = setting.pending ? (setting.value ? 'disabled-on' : 'disabled-off')
            : (setting.value ? 'on' : 'off');
        control = h(Switch, { id: `plugin-setting-${plugin.id}-${setting.id}`, className: `plugin-switch ${state}`, accessibilityLabel: setting.displayLabel, state: state, onClick: setting.pending ? undefined
                : () => action('set-setting', { ...base, value: !setting.value }) });
    }
    else if (kind === 'integer') {
        control = h("div", { className: "plugin-actions" },
            h(Button, { className: setting.pending ? 'plugin-action disabled' : 'plugin-action', disabled: setting.pending, onClick: () => action('step-setting', { ...base, direction: 'decrement' }) }, "\u2212"),
            h(Button, { className: setting.pending ? 'plugin-action disabled' : 'plugin-action', disabled: setting.pending, onClick: () => action('step-setting', { ...base, direction: 'increment' }) }, "+"));
    }
    else if (kind === 'choice' && setting.kind.options.length) {
        const index = setting.kind.options.indexOf(setting.value);
        const next = setting.kind.options[(index + 1) % setting.kind.options.length];
        control = h(Button, { className: setting.pending ? 'plugin-action disabled' : 'plugin-action', disabled: setting.pending, onClick: () => action('set-setting', { ...base, value: next }) }, labels.change);
    }
    else if (kind === 'text') {
        control = h(Button, { className: setting.pending ? 'plugin-action disabled' : 'plugin-action', disabled: setting.pending, onClick: () => action('edit-text', base) }, labels.edit);
    }
    return h("div", { className: "plugin-setting" },
        h("div", { className: "plugin-detail" },
            h("div", { className: "plugin-setting-label" },
                h(Text, { className: "plugin-detail-label" }, setting.displayLabel),
                h(Text, { className: "plugin-detail-value", wrap: true }, kind === 'boolean' ? setting.description : setting.displayValue)),
            control),
        setting.editing ? h("div", { className: "plugin-edit" },
            h(TextField, { id: `plugin-setting-text-${plugin.id}-${setting.id}`, className: "plugin-text-field", value: setting.draft || '', placeholder: labels.inputPlaceholder, onChange: value => action('text-changed', { value }) }),
            h(Button, { className: "plugin-action primary", onClick: () => action('save-text') }, labels.save),
            h(Button, { className: "plugin-action", onClick: () => action('cancel-text') }, labels.cancel)) : null);
}
function Plugin({ plugin, labels }) {
    return h("div", { className: "plugin-card" },
        h(Text, { className: "plugin-title", wrap: true }, plugin.name),
        h(Text, { className: "plugin-id", wrap: true }, plugin.id),
        h(Detail, { label: labels.publisher, value: plugin.author }),
        h(Detail, { label: labels.version, value: plugin.version }),
        h("div", { className: "plugin-detail" },
            h("div", { className: "plugin-setting-label" },
                h(Text, { className: "plugin-detail-label" }, labels.enabled),
                h(Text, { className: "plugin-detail-value" }, plugin.health)),
            h(Switch, { id: `plugin-enable-${plugin.id}`, className: `plugin-switch ${plugin.switchState}`, accessibilityLabel: plugin.toggleLabel, state: plugin.switchState, onClick: plugin.pending ? undefined
                    : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', { id: plugin.id }) })),
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
        plugin.settings.map(setting => h(Setting, { key: setting.id, plugin: plugin, labels: labels, setting: setting })));
}
function App() {
    const data = nickel.data;
    const labels = { ...data.labels, inputPlaceholder: data.inputPlaceholder };
    return h("div", { className: "plugins-page" },
        data.notice ? h("div", { className: "plugin-card" },
            h(Text, { className: "plugin-title" }, labels.pluginStatus),
            h(Text, { className: "plugin-detail-value", wrap: true }, data.notice)) : null,
        data.available
            ? data.plugins.map(plugin => h(Plugin, { key: plugin.id, plugin: plugin, labels: labels }))
            : h("div", { className: "plugin-card" },
                h(Text, { className: "plugin-title" }, labels.waiting),
                h(Text, { className: "plugin-detail-value" }, labels.statusUnavailable),
                h(Button, { className: "plugin-action", onClick: () => action('refresh') }, labels.refresh)));
}
