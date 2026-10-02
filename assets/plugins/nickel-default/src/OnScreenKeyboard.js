// @jsx h
import "./styles/keyboard.css";
// Key meanings and secure recipient policy belong to the native keyboard service.
export function OnScreenKeyboard() {
    const data = nickel.keyboard.get();
    const operations = data.operations || {};
    const rows = data.rows || [];
    const control = (id, label, operation, action) => h(Button, { id: id, disabled: !operations[operation], onClick: action }, label);
    return h(FixedWindow, { id: "keyboard", width: nickel.data.surface?.width || 1056, height: data.height || nickel.data.surface?.height || 368, anchor: nickel.data.surface?.anchor || "bottom-left", passive: true, className: "keyboard-window", onEscape: () => nickel.keyboard.hide() },
        h(Column, { className: "keyboard-content" },
            h(Row, { className: "keyboard-toolbar" },
                h(Text, null, data.recipientAvailable ? "English (US)" : "Select a text field"),
                control("osk-plugin-hold", "Hold modifiers", "holdModifiers", () => nickel.keyboard.holdModifiers()),
                control("osk-plugin-dock", data.dockTop ? "Move down" : "Move up", "toggleDock", () => nickel.keyboard.toggleDock()),
                control("osk-plugin-hide", "Hide", "hide", () => nickel.keyboard.hide())),
            rows.map((row, rowIndex) => h(Row, { key: "row-" + rowIndex, className: "keyboard-key-row" }, row.map(key => h(Button, { key: key.id, id: key.id, className: "keyboard-key", disabled: !key.enabled || !operations.press, width: key.quarters * 18, onClick: () => key.enabled && nickel.keyboard.press(key.id) }, key.label)))),
            h(Row, { className: "keyboard-footer" },
                control("osk-plugin-smaller", "Smaller", "resize", () => nickel.keyboard.resize(-32)),
                control("osk-plugin-larger", "Larger", "resize", () => nickel.keyboard.resize(32)))));
}
