// @jsx h
// Ordinary selected-display actions. The Settings host validates connector
// identity and owns topology changes and timed revert.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({type, connector: data.connector, ...fields});
    return <settings-stack>
        <settings-fragment>
            {data.cards.map(card => <settings-card key={card.connector}
                label={card.name}
                value={card.enabled ? card.detail : `${card.detail}  ${data.disabledLabel}`}>
                <settings-button id={`display-card-${card.index}`} label="" value="quiet"
                    state={card.primary ? data.cardPrimaryLabel : ''}
                    onClick={() => request('select-display', {index: card.index, connector: card.connector})} />
            </settings-card>)}
        </settings-fragment>
        <settings-row label={data.enabledLabel} value="" compact={true}>
            <settings-switch id="display-enabled" label={data.enabledLabel}
                value={data.enabled ? 'on' : 'off'}
                onClick={() => request('enabled', {enabled: !data.enabled})} />
        </settings-row>
        <settings-select id="display-resolution" label={data.resolutionLabel} placeholder=""
            value={data.resolutionValue} open={data.resolutionOpen}
            onClick={() => request('toggle-resolution')}>
            {data.resolutions.map(mode => <settings-option key={`${mode.width}x${mode.height}`}
                id={`display-resolution-${mode.width}x${mode.height}`} label={mode.label}
                selected={mode.label === data.resolutionValue}
                onClick={() => request('resolution', {width: mode.width, height: mode.height})} />)}
        </settings-select>
        <settings-select id="display-refresh-rate" label={data.refreshLabel} placeholder=""
            value={data.refreshValue} open={data.refreshOpen}
            onClick={() => request('toggle-refresh')}>
            {data.refreshRates.map(rate => <settings-option key={rate.refresh}
                id={`display-refresh-${rate.refresh}`} label={rate.label}
                selected={rate.label === data.refreshValue}
                onClick={() => request('refresh', {refresh: rate.refresh})} />)}
        </settings-select>
        <settings-slider id="display-scale" label={data.scaleLabel} placeholder=""
            value={data.scaleValue} percent={data.scalePercent}
            onChange={fraction => request('scale', {fraction})} />
        <settings-grid>
            <settings-button id="display-identify" label={data.identifyLabel} value="secondary" maxLines={3}
                onClick={() => request('identify')} />
            <settings-button id="display-primary" label={data.primaryLabel} value="secondary" maxLines={3}
                onClick={() => request('primary')} />
            <settings-button id="display-apply" label={data.applyLabel} value="primary" maxLines={3}
                onClick={() => request('apply')} />
        </settings-grid>
        <settings-inline>
            {data.pendingRevert ? <settings-button id="display-keep" label={data.keepLabel}
                value="primary" onClick={() => request('keep')} /> : null}
            {data.pendingRevert ? <settings-button id="display-revert" label={data.revertLabel}
                value="secondary" onClick={() => request('revert')} /> : null}
        </settings-inline>
        <settings-radio-group id="application-scale-policy">
            <settings-radio id="application-scale-follow" label={data.applicationScaleFollowLabel} value=""
                selected={data.applicationScalePolicy === 'follow'}
                onClick={() => request('application-scale', {policy: 'follow'})} />
            <settings-radio id="application-scale-unchanged" label={data.applicationScaleUnchangedLabel} value=""
                selected={data.applicationScalePolicy === 'unchanged'}
                onClick={() => request('application-scale', {policy: 'unchanged'})} />
            <settings-radio id="application-scale-custom" label={data.applicationScaleCustomLabel} value=""
                selected={data.applicationScalePolicy === 'custom'}
                onClick={() => request('application-scale', {policy: 'custom'})} />
        </settings-radio-group>
        <settings-slider id="application-custom-scale" label={data.customScaleLabel} placeholder=""
            value={data.customScaleValue} percent={data.customScalePercent}
            onChange={fraction => request('application-scale-value', {fraction})} />
    </settings-stack>;
}
