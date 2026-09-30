// @jsx h
// Nickel validates every menu request against the captured application group.
function App() {
    const menu = nickel.data;
    const actions = (menu.slots && menu.slots["task-action"]) || [];
    return <FixedWindow width="100%" height="100%" className="taskbar-menu">
        <Column>
            {menu.applicationId ? <Button id="taskbar-menu-pin" className="taskbar-menu-button"
                onClick={() => nickel.request({type: "applications.togglePin", id: menu.applicationId})}>
                {menu.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"}
            </Button> : null}
            {menu.closeAll ? <Button id="taskbar-menu-close-all" className="taskbar-menu-button"
                onClick={() => nickel.request({type: "taskbar-menu-close-all"})}>
                Close all windows
            </Button> : null}
            {actions.map((action, index) => <Button key={`${action.pluginId}:${action.id}`}
                id={`taskbar-extension-${index}`} className="taskbar-menu-button"
                onClick={() => nickel.request({type: "invoke-plugin-slot-action",
                    slot: "task-action", pluginId: action.pluginId, id: action.id,
                    item: menu.applicationId})}>
                {action.label}
            </Button>)}
        </Column>
    </FixedWindow>;
}
