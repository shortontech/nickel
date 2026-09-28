// @jsx h
function App() {
    const dialog = nickel.data.surface.id === "confirm";
    return h(Panel, { height: dialog ? 180 : 280, background: 0xff202830 }, dialog
        ? h(Column, null,
            h(Text, null, "This is a separate plugin dialog."),
            h(Button, { id: "open-settings", onClick: () => nickel.request({ type: "show-settings" }) }, "Open Settings"),
            h(Button, { id: "dismiss-dialog", onClick: () => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "confirm",
                }) }, "Dismiss"))
        : h(Column, null,
            h(Text, null, "The dialog starts closed."),
            h(Button, { id: "open-dialog", onClick: () => nickel.request({
                    type: "show-plugin-surface",
                    surfaceId: "confirm",
                }) }, "Open dialog"),
            h(Button, { id: "close-home", onClick: () => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "home",
                }) }, "Close home")));
}
