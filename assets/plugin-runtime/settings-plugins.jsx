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
    const labels = props.labels;
    const label = setting.displayLabel;
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
        control = <settings-button label={labels.change} value="quiet"
            onClick={setting.pending ? undefined : () => action('set-setting', {...base, value: next})} />;
    } else if (kind === 'text') {
        control = <settings-button label={labels.edit} value="quiet"
            onClick={setting.pending ? undefined : () => action('edit-text', base)} />;
    }
    return <settings-fragment>
        <settings-row label={label} value={kind === 'boolean' ? setting.description : setting.displayValue}>
            {control}
        </settings-row>
        {props.editing ? <settings-inline>
            <settings-input id={`plugin-setting-text-${plugin.id}-${setting.id}`}
                value={props.draft} onChange={value => action('text-changed', {value})} />
            <settings-button label={labels.save} value="primary" onClick={() => action('save-text')} />
            <settings-button label={labels.cancel} value="quiet" onClick={() => action('cancel-text')} />
        </settings-inline> : null}
    </settings-fragment>;
}

function Plugin(props) {
    const plugin = props.plugin;
    const labels = props.labels;
    return <settings-card label={plugin.name} value={plugin.id}>
        <Detail label={labels.publisher} value={plugin.author} />
        <Detail label={labels.version} value={plugin.version} />
        <settings-row label={labels.enabled} value={plugin.health}>
            <settings-switch id={`plugin-enable-${plugin.id}`}
                label={plugin.toggleLabel}
                value={plugin.switchState}
                onClick={plugin.pending ? undefined : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', {id: plugin.id})} />
        </settings-row>
        <Detail label={labels.access} value={plugin.access} />
        <Detail label={labels.surfaces} value={plugin.surfaces} />
        <Detail label={labels.composition} value={plugin.composition} />
        <Detail label={labels.trackedMemory} value={plugin.memory.tracked} />
        <Detail label={labels.peakMemory} value={plugin.memory.peak} />
        <Detail label={labels.jsHeap} value={plugin.memory.js} />
        <Detail label={labels.nativeUi} value={plugin.memory.native} />
        <Detail label={labels.textures} value={plugin.memory.textures} />
        <Detail label={labels.timers} value={plugin.memory.timers} />
        {plugin.memory.overlap ? <Detail label={labels.memoryAttribution}
            value={labels.extensionOverlap} /> : null}
        {plugin.settings.map(setting => <Setting key={setting.id} plugin={plugin} labels={labels}
            setting={setting} editing={setting.editing} draft={setting.draft} />)}
    </settings-card>;
}

function App() {
    const data = nickel.data;
    const labels = data.labels;
    return <settings-stack>
        {data.notice ? <settings-card label={labels.pluginStatus} value={data.notice} /> : null}
        {data.available
            ? data.plugins.map(plugin => <Plugin key={plugin.id} plugin={plugin} labels={labels} />)
            : <settings-card label={labels.waiting} value={labels.statusUnavailable}>
                <settings-button label={labels.refresh} value="secondary" onClick={() => action('refresh')} />
            </settings-card>}
    </settings-stack>;
}
