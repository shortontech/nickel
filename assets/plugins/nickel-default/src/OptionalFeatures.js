// @jsx h
import "./styles/features.css";
export function OptionalFeatures() {
    const features = nickel.features.get();
    const [confirmDisable, setConfirmDisable] = useState(false);
    const keyboard = features.keyboard;
    const codex = features.codex;
    const enabled = codex.requestedEnabled === true;
    const setCodex = () => {
        if (enabled && codex.disableConfirmationRequired)
            setConfirmDisable(true);
        else
            nickel.features.setCodexEnabled(!enabled);
    };
    return h(Column, { className: "features-page" },
        !features.available ? h(Text, { wrap: true }, features.reason || "Optional features are unavailable.") : null,
        features.lastResult ? h(Text, { wrap: true }, features.lastResult.detail) : null,
        h(Column, { className: "feature-card" },
            h(Text, { className: "feature-title" }, "On-screen keyboard"),
            h(Text, { wrap: true }, "Automatic mode follows touchscreen availability. Enabled and disabled modes override automatic selection."),
            keyboard.environmentOverride ? h(Text, { wrap: true }, "The keyboard mode is controlled by the environment.") : null,
            !keyboard.runtimeAvailable ? h(Text, null, "Live keyboard status is unavailable.") : h(Text, null, keyboard.enabled ? "Keyboard is enabled" : "Keyboard is disabled"),
            h(Row, { className: "feature-options" }, ["automatic", "enabled", "disabled"].map(mode => h(Button, { key: mode, id: "feature-keyboard-" + mode, className: keyboard.mode === mode ? "feature-option selected" : "feature-option", state: keyboard.mode === mode ? "selected" : "unselected", disabled: !features.operations.setKeyboardMode || keyboard.mode === mode, onClick: () => nickel.features.setKeyboardMode(mode) }, mode.charAt(0).toUpperCase() + mode.slice(1))))),
        h(Column, { className: "feature-card" },
            h(Row, { className: "feature-options" },
                h(Text, { className: "feature-title" }, "Codex integration"),
                h(Switch, { id: "feature-codex-enabled", accessibilityLabel: "Enable Codex integration", state: enabled ? "on" : "off", disabled: !features.operations.setCodexEnabled, onClick: setCodex })),
            h(Text, null, "State: " + (codex.state || "unavailable")),
            h(Text, null, "Installation: " + (codex.installation || "unknown") + " · Health: " + (codex.health || "unknown")),
            codex.source ? h(Text, { wrap: true }, "Source: " + codex.source) : null,
            codex.policy && codex.policy !== "editable" ? h(Text, { wrap: true }, "Codex enablement is controlled by system policy.") : null,
            codex.diagnostic ? h(Text, { wrap: true }, codex.diagnostic) : null,
            codex.runtimeCountersAvailable ? h(Text, { wrap: true }, "Active windows: " + codex.activeWindows + " · Background workers: " + codex.backgroundWorkers + " · Subscriptions: " + codex.subscriptions + " · Warm surfaces: " + codex.warmSurfaces + " · Cache entries: " + codex.cacheEntries) : h(Text, { wrap: true }, "Runtime resource counts are unavailable on this host."),
            features.operations.retryCodex ? h(Button, { id: "feature-codex-refresh", onClick: () => nickel.features.retryCodex() }, "Refresh Codex status") : null,
            confirmDisable && enabled && features.operations.setCodexEnabled ? h(Column, { className: "feature-confirmation" },
                h(Text, { wrap: true }, "Disable Codex integration? Existing Codex windows and background work may close."),
                h(Row, { className: "feature-options" },
                    h(Button, { id: "feature-codex-cancel", onClick: () => setConfirmDisable(false) }, "Cancel"),
                    h(Button, { id: "feature-codex-confirm", onClick: () => { nickel.features.setCodexEnabled(false, true); setConfirmDisable(false); } }, "Disable Codex"))) : null));
}
registerSettingsPage({ id: "optional-features", group: "System", label: "Optional features", description: "On-screen keyboard and Codex integration", component: OptionalFeatures });
