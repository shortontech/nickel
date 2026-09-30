// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return h(Window, { width: home ? 400 : 450, height: home ? 240 : 260, className: "example-window" }, home
        ? h(Button, { id: "reopen-details", onClick: () => nickel.request({
                type: "surface.show",
                id: "details",
            }) }, "Reopen details")
        : h("div", { className: "details-content" },
            h(Text, null, "Details window"),
            h(Button, { id: "close-details", onClick: () => nickel.request({
                    type: "surface.hide",
                    id: "details",
                }) }, "Close details")));
}
