// @jsx h
// Optional Features layout and intent. The host validates current feature policy.
function request(type, fields = {}) {
    nickel.request({ type, ...fields });
}
function App() {
    const data = nickel.data;
    const keyboard = data.keyboard;
    const codex = data.codex;
    return h("div", { className: "features-page" },
        h("div", { className: "features-card" },
            h(Text, { className: "features-title" }, keyboard.title),
            h(Text, { className: "features-description", wrap: true }, keyboard.description),
            h("div", { className: "keyboard-options", role: "radiogroup", "aria-label": keyboard.title }, keyboard.options.map(option => h("div", { key: option.value, id: `keyboard-mode-${option.value}`, className: option.selected ? 'keyboard-option selected' : 'keyboard-option', role: "radio", "aria-label": option.label, "aria-checked": option.selected, disabled: !keyboard.editable, onClick: keyboard.editable
                    ? () => request('keyboard-mode', { mode: option.value })
                    : undefined },
                h(Text, { className: "keyboard-option-label" }, `${option.selected ? '◉' : '○'}  ${option.label}`),
                h(Text, { className: "keyboard-option-description", wrap: true }, option.description)))),
            h("div", { className: "features-status-row" },
                h(Text, { className: "features-label" }, keyboard.statusLabel),
                h(Text, { className: "features-description", wrap: true }, keyboard.status))),
        h("div", { className: "features-card" },
            h(Text, { className: "features-title" }, codex.title),
            h(Text, { className: "features-description", wrap: true }, codex.description),
            h("div", { className: "codex-control-row" },
                h("div", { className: "codex-control-label" },
                    h(Text, { className: "features-label" }, codex.enableLabel),
                    h(Text, { className: "features-description" }, codex.status)),
                h(Switch, { id: "optional-feature-codex-enabled", className: `codex-switch ${codex.switchState}`, accessibilityLabel: codex.accessibilityLabel, state: codex.switchState, onClick: codex.editable
                        ? () => request('codex-enabled', { enabled: codex.nextEnabled })
                        : undefined })),
            codex.confirmation ? h("div", { className: "features-confirmation" },
                h(Text, { className: "features-label", wrap: true }, codex.confirmation.title),
                h(Text, { className: "features-description", wrap: true }, codex.confirmation.description),
                h("div", { className: "features-actions" },
                    h(Button, { id: "codex-confirm-disable", className: "feature-action primary", onClick: () => request('confirm-disable') }, codex.confirmation.confirm),
                    h(Button, { id: "codex-cancel-disable", className: "feature-action", onClick: () => request('cancel-disable') }, codex.confirmation.cancel))) : null,
            codex.retry ? h(Button, { id: "codex-retry", className: "feature-action", onClick: () => request('retry-codex') }, codex.retryLabel) : null));
}
