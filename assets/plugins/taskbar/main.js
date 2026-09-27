// @jsx h
// The host supplies grouped tasks and performs all window and launch actions.
function Task(props) {
    const item = props.item;
    const label = (item.active ? "●" : "") + (item.name.charAt(0).toUpperCase() || "?");
    return h(Button, { id: "taskbar-item-" + item.index, accessibilityLabel: item.name, icon: item.icon ? "task:" + item.index : null, onContextMenu: () => nickel.request({ type: "taskbar-context-item", index: item.index, id: item.id }), onClick: () => nickel.request({ type: "taskbar-activate-item", index: item.index, id: item.id }) }, label);
}
function TrayItem(props) {
    const item = props.item;
    return h(Button, { id: "taskbar-tray-" + item.id, accessibilityLabel: item.title, icon: item.icon ? "tray:" + item.id : null, onContextMenu: () => nickel.request({ type: "taskbar-context-tray", id: item.id }), onClick: () => nickel.request({ type: "taskbar-activate-tray", id: item.id }) }, item.title.charAt(0).toUpperCase() || "?");
}
function App() {
    const data = nickel.data;
    return h(Panel, { height: 56, background: 0xf1222730 },
        h(Row, null,
            h(Button, { id: "taskbar-launcher", icon: "logo", accessibilityLabel: "Open Nickel Start", onClick: () => nickel.request({ type: "taskbar-toggle-launcher" }) }, "Nickel"),
            data.items.flatMap(item => [
                h(Task, { key: item.id, item: item }),
                ...(item.badges || []).map((badge, index) => h(Badge, {
                    key: item.id + ":badge:" + index,
                    label: badge.label, count: badge.count, color: badge.color
                }))
            ]),
            h(Spacer, null),
            data.keyboardEnabled ? h(Button, { id: "taskbar-keyboard", accessibilityLabel: "On-screen keyboard", onClick: () => nickel.request({ type: "taskbar-toggle-keyboard" }) }, "\u2328") : null,
            data.codexAvailable ? h(Button, { id: "taskbar-codex", icon: "codex", accessibilityLabel: "Codex projects", onClick: () => nickel.request({ type: "taskbar-toggle-codex" }) }, "Codex") : null,
            data.tray.map(item => h(TrayItem, { key: item.id, item: item })),
            h(Button, { id: "taskbar-control", onClick: () => nickel.request({ type: "taskbar-toggle-control" }) }, data.clock)));
}
