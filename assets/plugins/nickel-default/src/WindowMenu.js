// @jsx h
import "./styles/window-menu.css";
// Native window facts and permitted operations remain owned by the window service.
export function WindowMenu() {
    const target = nickel.windows.menu().targetId;
    const window = nickel.windows.list().find(item => item.id === target);
    const destinations = nickel.windows.destinations();
    const dismiss = () => nickel.windows.dismissMenu();
    const act = operation => { operation(window.id); dismiss(); };
    return h(FixedWindow, { id: "window-menu", width: 320, height: 400, className: "window-menu", onEscape: dismiss, onBlur: () => nickel.windows.dismissMenu({ restoreFocus: false }) },
        h(ScrollView, { id: "window-menu-scroll", height: 368 },
            h(Column, null,
                h(Text, { className: "window-menu-title" }, window?.title || "Window unavailable"),
                window ? h(Column, null,
                    window.canActivate ? h(Button, { id: "window-activate", onClick: () => act(nickel.windows.activate) }, window.minimized ? "Restore" : "Activate") : null,
                    window.canMinimize && !window.minimized ? h(Button, { id: "window-minimize", onClick: () => act(nickel.windows.minimize) }, "Minimize") : null,
                    window.canMaximize ? h(Button, { id: "window-maximize", onClick: () => act(nickel.windows.toggleMaximize) }, window.maximized ? "Restore size" : "Maximize") : null,
                    window.canSnap ? h(Row, null,
                        h(Button, { id: "window-snap-leading", onClick: () => act(nickel.windows.snapLeading) }, "Snap left"),
                        h(Button, { id: "window-snap-trailing", onClick: () => act(nickel.windows.snapTrailing) }, "Snap right")) : null,
                    window.canFullscreen ? h(Button, { id: "window-fullscreen", onClick: () => act(nickel.windows.toggleFullscreen) }, window.fullscreen ? "Leave fullscreen" : "Fullscreen") : null,
                    window.canMoveToWorkspace ? destinations.workspaces.filter(item => item.id !== String(window.workspace)).map(item => h(Button, { key: item.id, id: "window-workspace-" + item.id, onClick: () => { nickel.windows.moveToWorkspace(window.id, item.id); dismiss(); } }, "Move to " + item.name)) : null,
                    window.canMoveToOutput ? destinations.outputs.filter(output => output !== window.output).map(output => h(Button, { key: output, id: "window-output-" + output, onClick: () => { nickel.windows.moveToOutput(window.id, output); dismiss(); } }, "Move to " + output)) : null,
                    window.canClose ? h(Button, { id: "window-close", onClick: () => act(nickel.windows.close) }, "Close window") : null) : null,
                h(Button, { id: "window-menu-dismiss", onClick: dismiss }, "Cancel"))));
}
