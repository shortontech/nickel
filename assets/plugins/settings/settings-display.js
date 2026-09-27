// @jsx h
// Ordinary selected-display actions. The Settings host validates connector
// identity and owns topology changes and timed revert.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({ type, connector: data.connector, ...fields });
    return h("settings-stack", null,
        h("settings-row", { label: data.enabledLabel, value: "", compact: true },
            h("settings-switch", { id: "display-enabled", label: data.enabledLabel, value: data.enabled ? 'on' : 'off', onClick: () => request('enabled', { enabled: !data.enabled }) })),
        h("settings-select", { id: "display-resolution", label: data.resolutionLabel, placeholder: "", value: data.resolutionValue, open: data.resolutionOpen, onClick: () => request('toggle-resolution') }, data.resolutions.map(mode => h("settings-option", { key: `${mode.width}x${mode.height}`, id: `display-resolution-${mode.width}x${mode.height}`, label: mode.label, selected: mode.label === data.resolutionValue, onClick: () => request('resolution', { width: mode.width, height: mode.height }) }))),
        h("settings-select", { id: "display-refresh-rate", label: data.refreshLabel, placeholder: "", value: data.refreshValue, open: data.refreshOpen, onClick: () => request('toggle-refresh') }, data.refreshRates.map(rate => h("settings-option", { key: rate.refresh, id: `display-refresh-${rate.refresh}`, label: rate.label, selected: rate.label === data.refreshValue, onClick: () => request('refresh', { refresh: rate.refresh }) }))),
        h("settings-slider", { id: "display-scale", label: data.scaleLabel, placeholder: "", value: data.scaleValue, percent: data.scalePercent, onChange: fraction => request('scale', { fraction }) }),
        h("settings-grid", null,
            h("settings-button", { id: "display-identify", label: data.identifyLabel, value: "secondary", maxLines: 3, onClick: () => request('identify') }),
            h("settings-button", { id: "display-primary", label: data.primaryLabel, value: "secondary", maxLines: 3, onClick: () => request('primary') }),
            h("settings-button", { id: "display-apply", label: data.applyLabel, value: "primary", maxLines: 3, onClick: () => request('apply') })),
        h("settings-inline", null,
            data.pendingRevert ? h("settings-button", { id: "display-keep", label: data.keepLabel, value: "primary", onClick: () => request('keep') }) : null,
            data.pendingRevert ? h("settings-button", { id: "display-revert", label: data.revertLabel, value: "secondary", onClick: () => request('revert') }) : null),
        h("settings-radio-group", { id: "application-scale-policy" },
            h("settings-radio", { id: "application-scale-follow", label: data.applicationScaleFollowLabel, value: "", selected: data.applicationScalePolicy === 'follow', onClick: () => request('application-scale', { policy: 'follow' }) }),
            h("settings-radio", { id: "application-scale-unchanged", label: data.applicationScaleUnchangedLabel, value: "", selected: data.applicationScalePolicy === 'unchanged', onClick: () => request('application-scale', { policy: 'unchanged' }) }),
            h("settings-radio", { id: "application-scale-custom", label: data.applicationScaleCustomLabel, value: "", selected: data.applicationScalePolicy === 'custom', onClick: () => request('application-scale', { policy: 'custom' }) })),
        h("settings-slider", { id: "application-custom-scale", label: data.customScaleLabel, placeholder: "", value: data.customScaleValue, percent: data.customScalePercent, onChange: fraction => request('application-scale-value', { fraction }) }));
}
