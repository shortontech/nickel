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
    const label = `Setting: ${setting.label}`;
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
        control = h("settings-button", { label: "Change", value: "quiet", onClick: setting.pending ? undefined : () => action('set-setting', { ...base, value: next }) });
    }
    else if (kind === 'text') {
        control = h("settings-button", { label: "Edit", value: "quiet", onClick: setting.pending ? undefined : () => action('edit-text', base) });
    }
    return h("settings-fragment", null,
        h("settings-row", { label: label, value: kind === 'boolean' ? setting.description : setting.displayValue }, control),
        props.editing ? h("settings-inline", null,
            h("settings-input", { id: `plugin-setting-text-${plugin.id}-${setting.id}`, value: props.draft, onChange: value => action('text-changed', { value }) }),
            h("settings-button", { label: "Save", value: "primary", onClick: () => action('save-text') }),
            h("settings-button", { label: "Cancel", value: "quiet", onClick: () => action('cancel-text') })) : null);
}
function Plugin(props) {
    const plugin = props.plugin;
    return h("settings-card", { label: plugin.name, value: plugin.id },
        h(Detail, { label: "Publisher", value: plugin.author }),
        h(Detail, { label: "Version", value: plugin.version }),
        h("settings-row", { label: "Enabled", value: plugin.health },
            h("settings-switch", { id: `plugin-enable-${plugin.id}`, label: `${plugin.desiredEnabled ? 'Disable' : 'Enable'} ${plugin.name}`, value: plugin.switchState, onClick: plugin.pending ? undefined : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', { id: plugin.id }) })),
        h(Detail, { label: "Access", value: plugin.access }),
        h(Detail, { label: "Surfaces", value: plugin.surfaces }),
        h(Detail, { label: "Composition", value: plugin.composition }),
        h(Detail, { label: "Tracked memory (lower bound)", value: plugin.memory.tracked }),
        h(Detail, { label: "Peak tracked memory (lower bound)", value: plugin.memory.peak }),
        h(Detail, { label: "JavaScript heap", value: plugin.memory.js }),
        h(Detail, { label: "Native UI (lower bound)", value: plugin.memory.native }),
        h(Detail, { label: "Textures", value: plugin.memory.textures }),
        h(Detail, { label: "Timers and subscriptions", value: plugin.memory.timers }),
        plugin.memory.overlap ? h(Detail, { label: "Memory attribution", value: "Extension UI also appears in the target plugin's native UI count" }) : null,
        plugin.settings.map(setting => h(Setting, { key: setting.id, plugin: plugin, setting: setting, editing: setting.editing, draft: setting.draft })));
}
function App() {
    const data = nickel.data;
    return h("settings-stack", null,
        data.notice ? h("settings-card", { label: "Plugin status", value: data.notice }) : null,
        data.available
            ? data.plugins.map(plugin => h(Plugin, { key: plugin.id, plugin: plugin }))
            : h("settings-card", { label: "Waiting for Nickel", value: "Live plugin status is unavailable." },
                h("settings-button", { label: "Refresh", value: "secondary", onClick: () => action('refresh') })));
}
