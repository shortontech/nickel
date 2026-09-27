// @jsx h
// Nickel validates every menu request against the captured application group.
function App() {
    const menu = nickel.data;
    return h(Panel, { height: menu.closeAll ? 112 : 64, background: 0xf12b303c },
        h(Column, null,
            menu.applicationId ? h(Button, {
                id: "taskbar-menu-pin",
                onClick: () => nickel.request({ type: "taskbar-menu-toggle-pin", id: menu.applicationId })
            }, menu.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar") : null,
            menu.closeAll ? h(Button, {
                id: "taskbar-menu-close-all",
                onClick: () => nickel.request({ type: "taskbar-menu-close-all" })
            }, "Close all windows") : null));
}
