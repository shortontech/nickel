// @jsx h
// Ordinary plugin status and settings UI. Permission approval stays with the host.
function action(type, fields = {}) {
    nickel.request({type, ...fields});
}

function Detail({label, value}) {
    return <div className="plugin-detail">
        <Text className="plugin-detail-label detail-label">{label}</Text>
        <Text className="plugin-detail-value detail-value" wrap={true}>{value}</Text>
    </div>;
}

function Setting({plugin, setting, labels}) {
    const base = {id: plugin.id, key: setting.id};
    const kind = setting.kind.kind;
    let control = null;
    if (kind === 'boolean') {
        const state = setting.pending ? (setting.value ? 'disabled-on' : 'disabled-off')
            : (setting.value ? 'on' : 'off');
        control = <Switch id={`plugin-setting-${plugin.id}-${setting.id}`}
            className={`plugin-switch ${state}`}
            accessibilityLabel={setting.displayLabel} state={state}
            onClick={setting.pending ? undefined
                : () => action('set-setting', {...base, value: !setting.value})} />;
    } else if (kind === 'integer') {
        control = <div className="plugin-actions">
            <Button className={setting.pending ? 'plugin-action disabled' : 'plugin-action'} disabled={setting.pending}
                onClick={() => action('step-setting', {...base, direction: 'decrement'})}>−</Button>
            <Button className={setting.pending ? 'plugin-action disabled' : 'plugin-action'} disabled={setting.pending}
                onClick={() => action('step-setting', {...base, direction: 'increment'})}>+</Button>
        </div>;
    } else if (kind === 'choice' && setting.kind.options.length) {
        const index = setting.kind.options.indexOf(setting.value);
        const next = setting.kind.options[(index + 1) % setting.kind.options.length];
        control = <Button className={setting.pending ? 'plugin-action disabled' : 'plugin-action'} disabled={setting.pending}
            onClick={() => action('set-setting', {...base, value: next})}>{labels.change}</Button>;
    } else if (kind === 'text') {
        control = <Button className={setting.pending ? 'plugin-action disabled' : 'plugin-action'} disabled={setting.pending}
            onClick={() => action('edit-text', base)}>{labels.edit}</Button>;
    }
    return <div className="plugin-setting">
        <div className="plugin-detail">
            <div className="plugin-setting-label">
                <Text className="plugin-detail-label">{setting.displayLabel}</Text>
                <Text className="plugin-detail-value" wrap={true}>{kind === 'boolean' ? setting.description : setting.displayValue}</Text>
            </div>
            {control}
        </div>
        {setting.editing ? <div className="plugin-edit">
            <TextField id={`plugin-setting-text-${plugin.id}-${setting.id}`}
                className="plugin-text-field" value={setting.draft || ''}
                placeholder={labels.inputPlaceholder}
                onChange={value => action('text-changed', {value})} />
            <Button className="plugin-action primary" onClick={() => action('save-text')}>{labels.save}</Button>
            <Button className="plugin-action" onClick={() => action('cancel-text')}>{labels.cancel}</Button>
        </div> : null}
    </div>;
}

function Plugin({plugin, labels}) {
    return <div className="plugin-card">
        <Text className="plugin-title" wrap={true}>{plugin.name}</Text>
        <Text className="plugin-id" wrap={true}>{plugin.id}</Text>
        <Detail label={labels.publisher} value={plugin.author} />
        <Detail label={labels.version} value={plugin.version} />
        <div className="plugin-detail">
            <div className="plugin-setting-label">
                <Text className="plugin-detail-label">{labels.enabled}</Text>
                <Text className="plugin-detail-value">{plugin.health}</Text>
            </div>
            <Switch id={`plugin-enable-${plugin.id}`}
                className={`plugin-switch ${plugin.switchState}`}
                accessibilityLabel={plugin.toggleLabel} state={plugin.switchState}
                onClick={plugin.pending ? undefined
                    : () => action(plugin.desiredEnabled ? 'disable' : 'review-enable', {id: plugin.id})} />
        </div>
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
            setting={setting} />)}
    </div>;
}

function App() {
    const data = nickel.data;
    const labels = {...data.labels, inputPlaceholder: data.inputPlaceholder};
    return <div className="plugins-page">
        {data.notice ? <div className="plugin-card">
            <Text className="plugin-title">{labels.pluginStatus}</Text>
            <Text className="plugin-detail-value" wrap={true}>{data.notice}</Text>
        </div> : null}
        {data.available
            ? data.plugins.map(plugin => <Plugin key={plugin.id} plugin={plugin} labels={labels} />)
            : <div className="plugin-card">
                <Text className="plugin-title">{labels.waiting}</Text>
                <Text className="plugin-detail-value">{labels.statusUnavailable}</Text>
                <Button className="plugin-action" onClick={() => action('refresh')}>{labels.refresh}</Button>
            </div>}
    </div>;
}
