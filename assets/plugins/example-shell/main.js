// @jsx h
let showTitle = true;
registerSetting({ id: "show-title", group: "Example shell", label: "Show shell title", type: "switch", defaultValue: true,
    value: () => showTitle,
    onChange: value => showTitle = value });
export function Taskbar() {
    const windows = nickel.windows.list();
    return h(FixedWindow, { id: "taskbar", output: "all", edge: "bottom", reserveWorkArea: true, className: "example-taskbar" },
        h(Row, null,
            showTitle !== false ? h(Text, null, "Example shell") : null,
            windows.map(window => h(Button, { key: window.id, id: "example-window-" + window.id, disabled: window.canActivate === false, onClick: () => nickel.windows.activate(window.id) }, window.title || "Window")),
            h(Spacer, null),
            h(Button, { id: "example-quick-settings", onClick: () => nickel.surfaces.show("quick-settings") }, "Quick Settings"),
            h(Button, { id: "example-settings", onClick: () => nickel.surfaces.show("settings") }, "Settings")));
}
export function Shell() {
    const surface = nickel.data.surface || {};
    const Component = nickel.component(surface.id === "settings" ? "shell.settings"
        : surface.id === "quick-settings" ? "shell.quickSettings" : "shell.taskbar");
    return h(Component, null);
}
export default Shell;
