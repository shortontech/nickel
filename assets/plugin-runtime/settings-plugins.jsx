// @jsx h
// Ordinary plugin status and settings UI. Permission approval stays with the host.
function action(type, fields = {}) {
    nickel.request({type, ...fields});
}

function Detail(props) {
    return <settings-row label={props.label} value={props.value} />;
}

function Setting(props) {
    const plugin = props.plugin;
    const setting = props.setting;
    const label = `Setting: ${setting.label}`;
    const base = {id: plugin.id, key: setting.id};
    let control = null;
    const kind = setting.kind.kind;
    if (kind === 'boolean') {
        control = <settings-switch id={`plugin-setting-${plugin.id}-${setting.id}`}
            label={label} value={setting.value ? 'on' : 'off'}
            onClick={setting.pending ? undefined : () => action('set-setting', {...base, value: !setting.value})} />;
    } else if (kind === 'integer') {
        control = <settings-inline>
            <settings-button label="−" value="quiet"
                onClick={setting.pending ? undefined : () => action('step-setting', {...base, direction: 'decrement'})} />
            <settings-button label="+" value="quiet"
                onClick={setting.pending ? undefined : () => action('step-setting', {...base, direction: 'increment'})} />
        </settings-inline>;
    } else if (kind === 'choice' && setting.kind.options.length) {
        const index = setting.kind.options.indexOf(setting.value);
        const next = setting.kind.options[(index + 1) % setting.kind.options.length];
        control = <settings-button label="Change" value="quiet"
            onClick={setting.pending ? undefined : () => action('set-setting', {...base, value: next})} />;
    } else if (kind === 'text') {
        control = <settings-button label="Edit" value="quiet"
            onClick={setting.pending ? undefined : () => action('edit-text', base)} />;
    }
    return <settings-fragment>
        <settings-row label={label} value={kind === 'boolean' ? setting.description : setting.displayValue}>
            {control}
        </settings-row>
        {props.editing ? <settings-inline>
            <settings-input id={`plugin-setting-text-${plugin.id}-${setting.id}`}
                value={props.draft} onChange={value => action('text-changed', {value})} />
            <settings-button label="Save" value="primary" onClick={() => action('save-text')} />
            <settings-button label="Cancel" value="quiet" onClick={() => action('cancel-text')} />
        </settings-inline> : null}
    </settings-fragment>;
}

function Plugin(props) {
    const plugin = props.plugin;
    return <settings-card label={plugin.name} value={plugin.id}>
        <Detail label="Publisher" value={plugin.author} />
        <Detail label="Version" value={plugin.version} />
        <settings-row label="Enabled" value={plugin.health}>
            <settings-switch id={`plugin-enable-${plugin.id}`}
                label={`${plugin.desiredEnabled ? 'Disable' : 'Enable'} ${plugin.name}`}
                value={plugin.switchState}
                onClick={plugin.pending ? undefined : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', {id: plugin.id})} />
        </settings-row>
        <Detail label="Access" value={plugin.access} />
        <Detail label="Surfaces" value={plugin.surfaces} />
        <Detail label="Composition" value={plugin.composition} />
        <Detail label="Tracked memory (lower bound)" value={plugin.memory.tracked} />
        <Detail label="Peak tracked memory (lower bound)" value={plugin.memory.peak} />
        <Detail label="JavaScript heap" value={plugin.memory.js} />
        <Detail label="Native UI (lower bound)" value={plugin.memory.native} />
        <Detail label="Textures" value={plugin.memory.textures} />
        <Detail label="Timers and subscriptions" value={plugin.memory.timers} />
        {plugin.memory.overlap ? <Detail label="Memory attribution"
            value="Extension UI also appears in the target plugin's native UI count" /> : null}
        {plugin.settings.map(setting => <Setting key={setting.id} plugin={plugin}
            setting={setting} editing={setting.editing} draft={setting.draft} />)}
    </settings-card>;
}

function App() {
    const data = nickel.data;
    return <settings-stack>
        {data.notice ? <settings-card label="Plugin status" value={data.notice} /> : null}
        {data.available
            ? data.plugins.map(plugin => <Plugin key={plugin.id} plugin={plugin} />)
            : <settings-card label="Waiting for Nickel" value="Live plugin status is unavailable.">
                <settings-button label="Refresh" value="secondary" onClick={() => action('refresh')} />
            </settings-card>}
    </settings-stack>;
}
