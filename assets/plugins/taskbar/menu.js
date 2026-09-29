// @jsx h
// Nickel validates every menu request against the captured application group.
function App() {
    const menu = nickel.data;
    return h(FixedWindow, { width: "100%", height: "100%", className: "taskbar-menu" },
        h(Column, null,
            menu.applicationId ? h(Button, { id: "taskbar-menu-pin", className: "taskbar-menu-button", onClick: () => nickel.request({ type: "applications.togglePin", id: menu.applicationId }) }, menu.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar") : null,
            menu.closeAll ? h(Button, { id: "taskbar-menu-close-all", className: "taskbar-menu-button", onClick: () => nickel.request({ type: "taskbar-menu-close-all" }) }, "Close all windows") : null,
            menu.actions.map((action, index) => h(Button, { key: `${action.plugin}:${action.id}`, id: `taskbar-extension-${index}`, className: "taskbar-menu-button", onClick: () => nickel.request({ type: "taskbar-extension-action",
                    plugin: action.plugin, id: action.id, applicationId: menu.applicationId }) }, action.label))));
}
