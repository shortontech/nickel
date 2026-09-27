// @jsx h
// Nickel validates every menu request against the captured application group.
function App() {
    const menu = nickel.data;
    const rows = (menu.applicationId ? 1 : 0) + (menu.closeAll ? 1 : 0) + menu.actions.length;
    return h(Panel, { height: 16 + rows * 48, background: 0xf12b303c },
        h(Column, null,
            menu.applicationId ? h(Button, {
                id: "taskbar-menu-pin",
                onClick: () => nickel.request({ type: "taskbar-menu-toggle-pin", id: menu.applicationId })
            }, menu.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar") : null,
            menu.closeAll ? h(Button, {
                id: "taskbar-menu-close-all",
                onClick: () => nickel.request({ type: "taskbar-menu-close-all" })
            }, "Close all windows") : null,
            menu.actions.map((action, index) => h(Button, {
                key: `${action.plugin}:${action.id}`,
                id: `taskbar-extension-${index}`,
                onClick: () => nickel.request({
                    type: "taskbar-extension-action",
                    plugin: action.plugin,
                    id: action.id,
                    applicationId: menu.applicationId
                })
            }, action.label))));
}
