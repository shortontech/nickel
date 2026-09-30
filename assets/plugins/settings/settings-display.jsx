// @jsx h
// Display topology, drag geometry, and timed revert remain host-owned.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({type, connector: data.connector, ...fields});
    return <div className="display-page">
        <div className="display-cards">
            {data.cards.map(card => <div key={card.connector} className="display-card">
                <Text>{card.name}</Text><Text>{card.enabled ? card.detail : `${card.detail}  ${data.disabledLabel}`}</Text>
                <Button id={`display-card-${card.index}`}
                    onClick={() => request('select-display', {index: card.index, connector: card.connector})}>
                    {card.primary ? data.cardPrimaryLabel : card.name}
                </Button>
            </div>)}
        </div>
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
                onClick={() => request('application-scale-policy', {policy: policy.id})}>
                <Text wrap={true}>{policy.label}</Text>
            </div>)}
        </div>
        <div className="display-application-scale">
            <Text>{data.customScaleLabel}</Text><Text>{data.customScaleValue}</Text>
            <Slider id="application-custom-scale" value={data.customScalePercent}
                accessibilityLabel={data.customScaleLabel}
                onChange={fraction => request('application-scale-value', {fraction})} />
        </div>
    </div>;
}
