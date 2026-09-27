// @jsx h
// Optional Features layout and intent. The host checks current feature policy,
// runtime generations, and environment overrides before applying any request.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    const keyboard = data.keyboard;
    const codex = data.codex;
    return <settings-features>
        <settings-card label={keyboard.title} value={keyboard.description}>
            <settings-radio-group id="on-screen-keyboard-mode">
                {keyboard.options.map(option =>
                    <settings-radio key={option.value} id={`keyboard-mode-${option.value}`} label={option.label}
                        value={option.description} selected={option.selected}
                        onClick={keyboard.editable
                            ? () => request('keyboard-mode', {mode: option.value})
                            : undefined} />)}
            </settings-radio-group>
            <settings-row label={keyboard.statusLabel} value={keyboard.status} />
        </settings-card>
        <settings-card label={codex.title} value={codex.description}>
            <settings-row label={codex.enableLabel} value={codex.status}>
                <settings-switch id="optional-feature-codex-enabled"
                    label={codex.accessibilityLabel} value={codex.switchState}
                    onClick={codex.editable
                        ? () => request('codex-enabled', {enabled: codex.nextEnabled})
                        : undefined} />
            </settings-row>
            {codex.confirmation ? <settings-row label={codex.confirmation.title}
                value={codex.confirmation.description}>
                <settings-inline>
                    <settings-button id="codex-confirm-disable" label={codex.confirmation.confirm} value="primary"
                        onClick={() => request('confirm-disable')} />
                    <settings-button id="codex-cancel-disable" label={codex.confirmation.cancel} value="quiet"
                        onClick={() => request('cancel-disable')} />
                </settings-inline>
            </settings-row> : null}
            {codex.retry ? <settings-button id="codex-retry" label={codex.retryLabel} value="secondary"
                onClick={() => request('retry-codex')} /> : null}
        </settings-card>
    </settings-features>;
}
