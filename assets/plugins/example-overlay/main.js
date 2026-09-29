// @jsx h
function App() {
    const overlay = nickel.data.surface.id === "notice";
    return h(Window, { width: overlay ? 300 : 420, height: overlay ? 120 : 250, placement: overlay ? "fixed" : "managed", className: "example-overlay-window", background: overlay ? 0xb0202830 : 0xff202830 }, overlay
        ? h(Column, null,
            h(Text, null, "This overlay is a separate surface."),
            h(Button, { id: "hide-overlay", onClick: () => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "notice",
                }) }, "Close overlay"))
        : h(Column, null,
            h(Text, null, "The overlay starts closed."),
            h(Button, { id: "show-overlay", onClick: () => nickel.request({
                    type: "show-plugin-surface",
                    surfaceId: "notice",
                }) }, "Show overlay")));
}
