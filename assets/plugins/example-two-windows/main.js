// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return h(Panel, { height: home ? 240 : 260, background: 0xff202830 }, home
        ? h(Button, { id: "reopen-details", onClick: () => nickel.request({
                type: "show-plugin-surface",
                surfaceId: "details",
            }) }, "Reopen details")
        : h(Column, null,
            h(Text, null, "Details window"),
            h(Button, { id: "close-details", onClick: () => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "details",
                }) }, "Close details")));
}
