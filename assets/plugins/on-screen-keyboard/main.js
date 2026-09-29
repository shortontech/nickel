// @jsx h
// Key meanings and the text recipient remain in Nickel's host.
function App() {
    const data = nickel.data || {};
    const rows = data.rows || [];
    const generation = data.generation || 0;
    const request = (type, fields) => nickel.request(Object.assign({ type, generation }, fields || {}));
    const control = (id, label, type, fields) => h(Button, { id: id, onClick: () => request(type, fields) }, label);
    return h(Panel, { height: data.height || 368, background: 0xf1242931 },
        h(Column, null,
            h(Row, null,
                h(Text, null, data.recipientAvailable ? "English (US)" : "Select a text field"),
                control("osk-plugin-hold", "Hold modifiers", "keyboard-hold"),
                control("osk-plugin-dock", data.dockTop ? "Move down" : "Move up", "keyboard-dock"),
                control("osk-plugin-hide", "Hide", "keyboard-hide")),
            rows.map((row, rowIndex) => h(Row, { key: "row-" + rowIndex }, row.map(key => h(Button, { key: key.id, id: key.id, width: key.quarters * 18, onClick: () => key.enabled && request("keyboard-key", { id: key.id }) }, key.label)))),
            h(Row, null,
                control("osk-plugin-smaller", "Smaller", "keyboard-resize", { delta: -32 }),
                control("osk-plugin-larger", "Larger", "keyboard-resize", { delta: 32 }))));
}
