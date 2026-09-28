// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return h(Panel, { height: home ? 240 : 260, background: 0xff202830 }, home
        ? h(Button, { id: "reopen-details", onClick: () => nickel.request({
                type: "show-plugin-surface",
                surfaceId: "details",
            }) }, "Reopen details")
        : h(Text, null, "Details window"));
}
