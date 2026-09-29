// @jsx h
// Optional Features layout and intent. The host validates current feature policy.
function request(type, fields = {}) {
    nickel.request({type, ...fields});
}

function App() {
    const data = nickel.data;
    const keyboard = data.keyboard;
    const codex = data.codex;
    return <div className="features-page">
        <div className="features-card">
            <Text className="features-title">{keyboard.title}</Text>
            <Text className="features-description" wrap={true}>{keyboard.description}</Text>
            <div className="keyboard-options">
                {keyboard.options.map(option =>
                    <div key={option.value} className="keyboard-option">
                        {keyboard.editable
                            ? <Button id={`keyboard-mode-${option.value}`}
                                className={option.selected ? 'feature-option selected' : 'feature-option'}
                                accessibilityLabel={option.label}
                                onClick={() => request('keyboard-mode', {mode: option.value})}>{`${option.selected ? '◉' : '○'}  ${option.label}`}</Button>
                            : <Text className="feature-option-disabled">{`${option.selected ? '◉' : '○'}  ${option.label}`}</Text>}
                        <Text className="features-description" wrap={true}>{option.description}</Text>
                    </div>)}
            </div>
            <div className="features-status-row">
                <Text className="features-label">{keyboard.statusLabel}</Text>
                <Text className="features-description" wrap={true}>{keyboard.status}</Text>
            </div>
        </div>
        <div className="features-card">
            <Text className="features-title">{codex.title}</Text>
            <Text className="features-description" wrap={true}>{codex.description}</Text>
            <div className="codex-control-row">
                <div className="codex-control-label">
                    <Text className="features-label">{codex.enableLabel}</Text>
                    <Text className="features-description">{codex.status}</Text>
                </div>
                <Switch id="optional-feature-codex-enabled"
                    className={`codex-switch ${codex.switchState}`}
                    accessibilityLabel={codex.accessibilityLabel} state={codex.switchState}
                    onClick={codex.editable
                        ? () => request('codex-enabled', {enabled: codex.nextEnabled})
                        : undefined} />
            </div>
            {codex.confirmation ? <div className="features-confirmation">
                <Text className="features-label" wrap={true}>{codex.confirmation.title}</Text>
                <Text className="features-description" wrap={true}>{codex.confirmation.description}</Text>
                <div className="features-actions">
                    <Button id="codex-confirm-disable" className="feature-action primary"
                        onClick={() => request('confirm-disable')}>{codex.confirmation.confirm}</Button>
                    <Button id="codex-cancel-disable" className="feature-action"
                        onClick={() => request('cancel-disable')}>{codex.confirmation.cancel}</Button>
                </div>
            </div> : null}
            {codex.retry ? <Button id="codex-retry" className="feature-action"
                onClick={() => request('retry-codex')}>{codex.retryLabel}</Button> : null}
        </div>
    </div>;
}
