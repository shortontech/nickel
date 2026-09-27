// @jsx h
// Optional Features layout and intent. The host checks current feature policy,
// runtime generations, and environment overrides before applying any request.
function request(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function App() {
    const data = nickel.data;
    const keyboard = data.keyboard;
    const codex = data.codex;
    return h("settings-features", null,
        h("settings-card", { label: keyboard.title, value: keyboard.description },
            h("settings-radio-group", { id: "on-screen-keyboard-mode" }, keyboard.options.map(option => h("settings-radio", { key: option.value, id: `keyboard-mode-${option.value}`, label: option.label, value: option.description, selected: option.selected, onClick: keyboard.editable
                    ? () => request('keyboard-mode', { mode: option.value })
                    : undefined }))),
            h("settings-row", { label: keyboard.statusLabel, value: keyboard.status })),
        h("settings-card", { label: codex.title, value: codex.description },
            h("settings-row", { label: codex.enableLabel, value: codex.status },
                h("settings-switch", { id: "optional-feature-codex-enabled", label: codex.accessibilityLabel, value: codex.switchState, onClick: codex.editable
                        ? () => request('codex-enabled', { enabled: codex.nextEnabled })
                        : undefined })),
            codex.confirmation ? h("settings-row", { label: codex.confirmation.title, value: codex.confirmation.description },
                h("settings-inline", null,
                    h("settings-button", { id: "codex-confirm-disable", label: codex.confirmation.confirm, value: "primary", onClick: () => request('confirm-disable') }),
                    h("settings-button", { id: "codex-cancel-disable", label: codex.confirmation.cancel, value: "quiet", onClick: () => request('cancel-disable') }))) : null,
            codex.retry ? h("settings-button", { id: "codex-retry", label: codex.retryLabel, value: "secondary", onClick: () => request('retry-codex') }) : null));
}
