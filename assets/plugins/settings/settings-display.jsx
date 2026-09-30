// @jsx h
// Display cards and draft arrangement live in JSX. The host validates and previews the layout.
function arrangement(outputs) {
    const active = outputs.filter(output => output.enabled);
    if (!active.length) return {cards: [], scale: 1};
    const left = Math.min(...active.map(output => output.geometry.x));
    const top = Math.min(...active.map(output => output.geometry.y));
    const right = Math.max(...active.map(output => output.geometry.x + output.geometry.width));
    const bottom = Math.max(...active.map(output => output.geometry.y + output.geometry.height));
    const scale = Math.min(1, 680 / Math.max(1, right - left), 216 / Math.max(1, bottom - top));
    return {scale, cards: outputs.map(output => ({
        ...output,
        x: Math.round((output.geometry.x - left) * scale),
        y: Math.round((output.geometry.y - top) * scale),
        width: Math.max(120, Math.round(output.geometry.width * scale)),
        height: Math.max(80, Math.round(output.geometry.height * scale)),
    }))};
}
function snapPlacement(moved, outputs) {
    const snap = 32;
    let x = moved.geometry.x, y = moved.geometry.y;
    let bestX = snap + 1, bestY = snap + 1;
    for (const other of outputs) {
        if (other.name === moved.name || !other.enabled) continue;
        const a = moved.geometry, b = other.geometry;
        if (y < b.y + b.height && y + a.height > b.y) {
            for (const candidate of [b.x + b.width, b.x - a.width]) {
                const distance = Math.abs(candidate - x);
                if (distance < bestX) { bestX = distance; x = candidate; }
            }
        }
        if (x < b.x + b.width && x + a.width > b.x) {
            for (const candidate of [b.y + b.height, b.y - a.height]) {
                const distance = Math.abs(candidate - y);
                if (distance < bestY) { bestY = distance; y = candidate; }
            }
        }
    }
    return {x: Math.round(x), y: Math.round(y)};
}
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({type, connector: data.connector, ...fields});
    const snapshot = nickel.displays && nickel.displays.get ? nickel.displays.get() : null;
    const outputs = snapshot && snapshot.available ? snapshot.outputs : data.cards.map(card => ({
        name: card.connector, model: card.name, geometry: card.geometry,
        scale_120: card.scale_120, current_mode: card.current_mode,
        enabled: card.enabled, primary: card.primary,
    }));
    const drag = useRef(null);
    const draft = useRef(null);
    const revision = JSON.stringify(outputs.map(output => [output.name, output.geometry,
        output.enabled, output.primary, output.scale_120, output.current_mode]));
    const sourceRevision = useRef(revision);
    if (sourceRevision.current !== revision) {
        sourceRevision.current = revision;
        draft.current = null;
    }
    const current = draft.current || outputs;
    const view = arrangement(current);
    const selectedName = data.connector;
    const onDrag = (output, gesture) => {
        if (gesture.phase === 'start') {
            drag.current = {name: output.name, x: gesture.x, y: gesture.y};
            return;
        }
        if (gesture.phase === 'cancel') { drag.current = null; return; }
        if (gesture.phase !== 'end' || !drag.current || drag.current.name !== output.name) return;
        const start = drag.current;
        drag.current = null;
        const moved = {...output, geometry: {...output.geometry,
            x: output.geometry.x + Math.round((gesture.x - start.x) / view.scale),
            y: output.geometry.y + Math.round((gesture.y - start.y) / view.scale)}};
        const snapped = snapPlacement(moved, current);
        moved.geometry.x = snapped.x;
        moved.geometry.y = snapped.y;
        const next = current.map(item => item.name === moved.name ? moved : item);
        draft.current = next;
        nickel.displays.setLayout({
            primary: next.find(item => item.primary)?.name || next.find(item => item.enabled)?.name || moved.name,
            placements: next.map(item => ({
                name: item.name, x: item.geometry.x, y: item.geometry.y,
                enabled: item.enabled, scale_120: item.scale_120,
                mode: item.current_mode,
            })),
        });
    };
    return <div className="display-page">
        <Text>{data.title}</Text>
        <Text>{data.subtitle}</Text>
        <div className="display-plane">
            <Layer id="display-arrangement">
                {view.cards.map(card => {
                    const selection = data.cards.find(item => item.connector === card.name);
                    const label = selection ? selection.name : (card.model || card.name);
                    return <Box key={card.name} x={card.x} y={card.y}
                        width={card.width} height={card.height} className="display-card-box">
                        <Button id={`display-card-${selection ? selection.index : card.name}`}
                            className={card.primary ? 'display-card-button primary' : 'display-card-button'}
                            width={card.width} height={card.height}
                            accessibilityLabel={`${label} display, ${card.name}`}
                            onDrag={gesture => onDrag(card, gesture)}
                            onClick={() => selection && request('select-display', {index: selection.index, connector: card.name})}>
                            {`${label}${card.primary ? ' · ' + data.cardPrimaryLabel : ''}${!card.enabled ? ' · ' + data.disabledLabel : ''}`}
                        </Button>
                    </Box>;
                })}
            </Layer>
        </div>
        <div className="display-selected">
            <Text>{data.selectedName}</Text><Text>{data.selectedDetail}</Text>
        </div>
        <Text>{data.status}</Text>
        <div className="display-enabled-row">
            <Text>{data.enabledLabel}</Text>
            <Switch id="display-enabled" state={data.enabled ? 'on' : 'off'}
                accessibilityLabel={data.enabledLabel}
                onClick={() => request('enabled', {enabled: !data.enabled})} />
        </div>
        <div className="display-resolution">
            <Text>{data.resolutionLabel}</Text>
            <Button id="display-resolution" onClick={() => request('toggle-resolution')}>{data.resolutionValue}</Button>
            {data.resolutionOpen ? data.resolutions.map(mode => <Button key={`${mode.width}x${mode.height}`}
                id={`display-resolution-${mode.width}x${mode.height}`}
                onClick={() => request('resolution', {width: mode.width, height: mode.height})}>{mode.label}</Button>) : null}
        </div>
        <div className="display-refresh">
            <Text>{data.refreshLabel}</Text>
            <Button id="display-refresh-rate" onClick={() => request('toggle-refresh')}>{data.refreshValue}</Button>
            {data.refreshOpen ? data.refreshRates.map(rate => <Button key={rate.refresh}
                id={`display-refresh-${rate.refresh}`}
                onClick={() => request('refresh', {refresh: rate.refresh})}>{rate.label}</Button>) : null}
        </div>
        <div className="display-scale">
            <Text>{data.scaleLabel}</Text><Text>{data.scaleValue}</Text>
            <Slider id="display-scale" value={data.scalePercent}
                accessibilityLabel={data.scaleLabel}
                onChange={fraction => request('scale', {fraction})} />
        </div>
        <div className="display-actions">
            <Button id="display-identify" height={56} onClick={() => request('identify')}>{data.identifyLabel}</Button>
            <Button id="display-primary" height={56} onClick={() => request('primary')}>{data.primaryLabel}</Button>
            <Button id="display-apply" height={56} onClick={() => request('apply')}>{data.applyLabel}</Button>
        </div>
        <div className="display-confirmation">
            {data.pendingRevert ? <Button id="display-keep" onClick={() => request('keep')}>{data.keepLabel}</Button> : null}
            {data.pendingRevert ? <Button id="display-revert" onClick={() => request('revert')}>{data.revertLabel}</Button> : null}
        </div>
        <div className="display-application-policy" role="radiogroup" aria-label="Application scale policy">
            {data.applicationPolicies.map(policy => <div key={policy.id}
                id={`application-scale-policy-${policy.id}`}
                className={policy.selected ? 'application-policy selected' : 'application-policy'}
                role="radio" aria-label={policy.label} aria-checked={policy.selected}
                onClick={() => request('application-scale-policy', {policy: policy.id})}>{policy.label}</div>)}
        </div>
        <div className="display-application-scale">
            <Text>{data.customScaleLabel}</Text><Text>{data.customScaleValue}</Text>
            <Slider id="application-custom-scale" value={data.customScalePercent}
                accessibilityLabel={data.customScaleLabel}
                onChange={fraction => request('application-scale-value', {fraction})} />
        </div>
    </div>;
}
