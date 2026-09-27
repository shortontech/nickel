// @jsx h
// Ordinary selected-display actions. The Settings host validates connector
// identity and owns topology changes and timed revert.
function App() {
    const data = nickel.data;
    const request = (type, fields = {}) => nickel.request({ type, connector: data.connector, ...fields });
    return h("settings-stack", null,
        h("settings-row", { label: data.enabledLabel, value: "", compact: true },
            h("settings-switch", { id: "display-enabled", label: data.enabledLabel, value: data.enabled ? 'on' : 'off', onClick: () => request('enabled', { enabled: !data.enabled }) })),
        h("settings-grid", null,
            h("settings-button", { id: "display-identify", label: data.identifyLabel, value: "secondary", maxLines: 3, onClick: () => request('identify') }),
            h("settings-button", { id: "display-primary", label: data.primaryLabel, value: "secondary", maxLines: 3, onClick: () => request('primary') }),
            h("settings-button", { id: "display-apply", label: data.applyLabel, value: "primary", maxLines: 3, onClick: () => request('apply') })),
        h("settings-inline", null,
            data.pendingRevert ? h("settings-button", { id: "display-keep", label: data.keepLabel, value: "primary", onClick: () => request('keep') }) : null,
            data.pendingRevert ? h("settings-button", { id: "display-revert", label: data.revertLabel, value: "secondary", onClick: () => request('revert') }) : null));
}
