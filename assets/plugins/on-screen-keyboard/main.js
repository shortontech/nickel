// @jsx h
// Key meanings and the text recipient remain in Nickel's host.
function App() {
    const data = nickel.data || {};
    const rows = data.rows || [];
    const generation = data.generation || 0;
    const request = (type, fields) => nickel.request(Object.assign({ type, generation }, fields || {}));
    const control = (id, label, type, fields) => h(Button, { id: id, onClick: () => request(type, fields) }, label);
    return h(FixedWindow, { width: "100%", height: "100%", className: "keyboard-window", onEscape: () => request("keyboard-hide") },
        h(Column, { className: "keyboard-content" },
            h(Row, { className: "keyboard-toolbar" },
                h(Text, null, data.recipientAvailable ? "English (US)" : "Select a text field"),
                control("osk-plugin-hold", "Hold modifiers", "keyboard-hold"),
                control("osk-plugin-dock", data.dockTop ? "Move down" : "Move up", "keyboard-dock"),
                control("osk-plugin-hide", "Hide", "keyboard-hide")),
            rows.map((row, rowIndex) => h(Row, { key: "row-" + rowIndex, className: "keyboard-key-row" }, row.map(key => h(Button, { key: key.id, id: key.id, className: "keyboard-key", width: key.quarters * 18, onClick: () => key.enabled && request("keyboard-key", { id: key.id }) }, key.label)))),
            h(Row, { className: "keyboard-footer" },
                control("osk-plugin-smaller", "Smaller", "keyboard-resize", { delta: -32 }),
                control("osk-plugin-larger", "Larger", "keyboard-resize", { delta: 32 }))));
}
