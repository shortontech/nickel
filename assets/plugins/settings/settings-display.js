// @jsx h
// Display topology, drag geometry, and timed revert remain host-owned.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({ type, connector: data.connector, ...fields });
    return h("div", { className: "display-page" },
        h("div", { className: "display-cards" }, data.cards.map(card => h("div", { key: card.connector, className: "display-card" },
            h(Text, null, card.name),
            h(Text, null, card.enabled ? card.detail : `${card.detail}  ${data.disabledLabel}`),
            h(Button, { id: `display-card-${card.index}`, onClick: () => request('select-display', { index: card.index, connector: card.connector }) }, card.primary ? data.cardPrimaryLabel : card.name)))),
        h("div", { className: "display-enabled-row" },
            h(Text, null, data.enabledLabel),
            h(Switch, { id: "display-enabled", state: data.enabled ? 'on' : 'off', accessibilityLabel: data.enabledLabel, onClick: () => request('enabled', { enabled: !data.enabled }) })),
        h("div", { className: "display-resolution" },
            h(Text, null, data.resolutionLabel),
            h(Button, { id: "display-resolution", onClick: () => request('toggle-resolution') }, data.resolutionValue),
            data.resolutionOpen ? data.resolutions.map(mode => h(Button, { key: `${mode.width}x${mode.height}`, id: `display-resolution-${mode.width}x${mode.height}`, onClick: () => request('resolution', { width: mode.width, height: mode.height }) }, mode.label)) : null),
        h("div", { className: "display-refresh" },
            h(Text, null, data.refreshLabel),
            h(Button, { id: "display-refresh-rate", onClick: () => request('toggle-refresh') }, data.refreshValue),
            data.refreshOpen ? data.refreshRates.map(rate => h(Button, { key: rate.refresh, id: `display-refresh-${rate.refresh}`, onClick: () => request('refresh', { refresh: rate.refresh }) }, rate.label)) : null),
        h("div", { className: "display-scale" },
            h(Text, null, data.scaleLabel),
            h(Text, null, data.scaleValue),
            h(Slider, { id: "display-scale", value: data.scalePercent, accessibilityLabel: data.scaleLabel, onChange: fraction => request('scale', { fraction }) })),
        h("div", { className: "display-actions" },
            h(Button, { id: "display-identify", height: 56, onClick: () => request('identify') }, data.identifyLabel),
            h(Button, { id: "display-primary", height: 56, onClick: () => request('primary') }, data.primaryLabel),
            h(Button, { id: "display-apply", height: 56, onClick: () => request('apply') }, data.applyLabel)),
        h("div", { className: "display-confirmation" },
            data.pendingRevert ? h(Button, { id: "display-keep", onClick: () => request('keep') }, data.keepLabel) : null,
            data.pendingRevert ? h(Button, { id: "display-revert", onClick: () => request('revert') }, data.revertLabel) : null),
        h("div", { className: "display-application-policy", role: "radiogroup", "aria-label": "Application scale policy" }, data.applicationPolicies.map(policy => h("div", { key: policy.id, id: `application-scale-policy-${policy.id}`, className: policy.selected ? 'application-policy selected' : 'application-policy', role: "radio", "aria-label": policy.label, "aria-checked": policy.selected, onClick: () => request('application-scale-policy', { policy: policy.id }) }, policy.label))),
        h("div", { className: "display-application-scale" },
            h(Text, null, data.customScaleLabel),
            h(Text, null, data.customScaleValue),
            h(Slider, { id: "application-custom-scale", value: data.customScalePercent, accessibilityLabel: data.customScaleLabel, onChange: fraction => request('application-scale-value', { fraction }) })));
}
