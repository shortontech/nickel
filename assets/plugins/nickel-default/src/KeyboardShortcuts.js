// @jsx h
import "./styles/features.css";
export function KeyboardShortcuts() {
    const snapshot = nickel.shortcuts.get();
    return h(Column, { className: "features-page" },
        h(Text, { wrap: true }, snapshot.reason || "Keyboard shortcut reference"),
        snapshot.globalReason ? h(Text, { wrap: true }, snapshot.globalReason) : null,
        snapshot.shortcuts.map(shortcut => h(Column, { key: shortcut.id, className: "feature-card" },
            h(Text, { className: "feature-title" }, shortcut.action),
            h(Text, { wrap: true }, shortcut.keys),
            h(Text, null, shortcut.scope + (shortcut.available ? "" : " · Unavailable")))));
}
registerSettingsPage({ id: "keyboard-shortcuts", group: "Input", label: "Keyboard shortcuts", description: "Shell shortcuts and navigation", component: KeyboardShortcuts });
