// @jsx h
// The host supplies grouped tasks and performs all window and launch actions.
function Task(props) {
    const item = props.item;
    const label = (item.active ? "●" : "") + (item.name.charAt(0).toUpperCase() || "?");
    const pendingDrag = useRef(0);
    const suppressClick = useRef(false);
    const onDrag = gesture => {
        if (gesture.phase === "start" || gesture.phase === "cancel") {
            pendingDrag.current = 0;
            suppressClick.current = false;
            return;
        }
        const direction = gesture.x < gesture.bounds.x ? -1
            : gesture.x > gesture.bounds.x + gesture.bounds.width ? 1 : 0;
        if (gesture.phase === "move") {
            if (direction)
                pendingDrag.current = direction;
            return;
        }
        if (gesture.phase === "end" && item.pinned) {
            const move = pendingDrag.current || direction;
            pendingDrag.current = 0;
            if (move) {
                suppressClick.current = true;
                nickel.request({ type: "taskbar-move-pin", index: item.index,
                    id: item.id, direction: move < 0 ? "left" : "right" });
            }
        }
    };
    return h(Button, { id: "taskbar-item-" + item.index, className: item.active ? "task-button is-active" : "task-button", accessibilityLabel: item.name, icon: item.icon ? "task:" + item.index : null, onDrag: onDrag, onContextMenu: () => nickel.request({ type: "taskbar-context-item", index: item.index, id: item.id }), onClick: () => {
            if (suppressClick.current) {
                suppressClick.current = false;
                return;
            }
            nickel.request({ type: "taskbar-activate-item", index: item.index, id: item.id });
        } }, label);
}
function TrayItem(props) {
    const item = props.item;
    return h(Button, { id: "taskbar-tray-" + item.id, className: "tray-button", accessibilityLabel: item.title, icon: item.icon ? "tray:" + item.id : null, onContextMenu: () => nickel.request({ type: "taskbar-context-tray", id: item.id }), onClick: () => nickel.request({ type: "taskbar-activate-tray", id: item.id }) }, item.title.charAt(0).toUpperCase() || "?");
}
function App() {
    const data = nickel.data;
    const items = data.items || [];
    const tray = data.tray || [];
    return h(FixedWindow, { width: "100%", height: 56, output: "all", edge: "bottom", reserveWorkArea: true, className: "taskbar" },
        h("div", { className: "taskbar-content" },
            h(Button, { id: "taskbar-launcher", className: "launcher-button", icon: "logo", accessibilityLabel: "Open Nickel Start", onClick: () => nickel.request({ type: "taskbar-toggle-launcher" }) }, "Nickel"),
            items.flatMap(item => [
                h(Task, { key: item.id, item: item }),
                ...(item.badges || []).map((badge, index) => h(Badge, { key: item.id + ":badge:" + index, label: badge.label, count: badge.count, color: badge.color }))
            ]),
            h(Spacer, { className: "taskbar-spacer" }),
            data.keyboardEnabled ? h(Button, { id: "taskbar-keyboard", className: "utility-button", accessibilityLabel: "On-screen keyboard", onClick: () => nickel.request({ type: "taskbar-toggle-keyboard" }) }, "\u2328") : null,
            data.codexAvailable ? h(Button, { id: "taskbar-codex", className: "utility-button", icon: "codex", accessibilityLabel: "Codex projects", onClick: () => nickel.request({ type: "taskbar-toggle-codex" }) }, "Codex") : null,
            tray.map(item => h(TrayItem, { key: item.id, item: item })),
            h(Button, { id: "taskbar-control", className: "clock-button", onClick: () => nickel.request({ type: "toggle-control-center" }) }, data.clock || "")));
}
