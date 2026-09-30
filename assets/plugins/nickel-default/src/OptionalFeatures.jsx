// @jsx h
import "./styles/features.css";

export function OptionalFeatures() {
    const features = nickel.features.get();
    const [confirmDisable, setConfirmDisable] = useState(false);
    const keyboard = features.keyboard;
    const codex = features.codex;
    const enabled = codex.requestedEnabled === true;
    const setCodex = () => {
        if (enabled && codex.disableConfirmationRequired) setConfirmDisable(true);
        else nickel.features.setCodexEnabled(!enabled);
    };
    return <Column className="features-page">
        {!features.available ? <Text wrap={true}>{features.reason || "Optional features are unavailable."}</Text> : null}
        {features.lastResult ? <Text wrap={true}>{features.lastResult.detail}</Text> : null}
        <Column className="feature-card">
            <Text className="feature-title">On-screen keyboard</Text>
            <Text wrap={true}>Automatic mode follows touchscreen availability. Enabled and disabled modes override automatic selection.</Text>
            {keyboard.environmentOverride ? <Text wrap={true}>The keyboard mode is controlled by the environment.</Text> : null}
            {!keyboard.runtimeAvailable ? <Text>Live keyboard status is unavailable.</Text> : <Text>{keyboard.enabled ? "Keyboard is enabled" : "Keyboard is disabled"}</Text>}
            <Row className="feature-options">
                {["automatic","enabled","disabled"].map(mode => <Button key={mode} id={"feature-keyboard-" + mode}
                    className={keyboard.mode === mode ? "feature-option selected" : "feature-option"}
                    state={keyboard.mode === mode ? "selected" : "unselected"}
                    disabled={!features.operations.setKeyboardMode || keyboard.mode === mode}
                    onClick={() => nickel.features.setKeyboardMode(mode)}>{mode.charAt(0).toUpperCase() + mode.slice(1)}</Button>)}
            </Row>
        </Column>
        <Column className="feature-card">
            <Row className="feature-options">
                <Text className="feature-title">Codex integration</Text>
                <Switch id="feature-codex-enabled" accessibilityLabel="Enable Codex integration" state={enabled ? "on" : "off"}
                    disabled={!features.operations.setCodexEnabled} onClick={setCodex} />
            </Row>
            <Text>{"State: " + (codex.state || "unavailable")}</Text>
            <Text>{"Installation: " + (codex.installation || "unknown") + " · Health: " + (codex.health || "unknown")}</Text>
            {codex.source ? <Text wrap={true}>{"Source: " + codex.source}</Text> : null}
            {codex.policy && codex.policy !== "editable" ? <Text wrap={true}>Codex enablement is controlled by system policy.</Text> : null}
            {codex.diagnostic ? <Text wrap={true}>{codex.diagnostic}</Text> : null}
            {codex.runtimeCountersAvailable ? <Text wrap={true}>{"Active windows: " + codex.activeWindows + " · Background workers: " + codex.backgroundWorkers + " · Subscriptions: " + codex.subscriptions + " · Warm surfaces: " + codex.warmSurfaces + " · Cache entries: " + codex.cacheEntries}</Text> : <Text wrap={true}>Runtime resource counts are unavailable on this host.</Text>}
            {features.operations.retryCodex ? <Button id="feature-codex-refresh" onClick={() => nickel.features.retryCodex()}>Refresh Codex status</Button> : null}
            {confirmDisable && enabled && features.operations.setCodexEnabled ? <Column className="feature-confirmation">
                <Text wrap={true}>Disable Codex integration? Existing Codex windows and background work may close.</Text>
                <Row className="feature-options">
                    <Button id="feature-codex-cancel" onClick={() => setConfirmDisable(false)}>Cancel</Button>
                    <Button id="feature-codex-confirm" onClick={() => {nickel.features.setCodexEnabled(false, true); setConfirmDisable(false);}}>Disable Codex</Button>
                </Row>
            </Column> : null}
        </Column>
    </Column>;
}

registerSettingsPage({id:"optional-features",group:"System",label:"Optional features",description:"On-screen keyboard and Codex integration",component:OptionalFeatures});
