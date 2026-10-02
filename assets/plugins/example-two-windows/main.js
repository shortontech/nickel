// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return h(Window, { width: home ? 400 : 450, height: home ? 240 : 260, className: "example-window" }, home
        ? h("div", { className: "details-content" },
            h(Button, { id: "reopen-details", onClick: () => nickel.surfaces.show("details") }, "Reopen details"),
            h(Button, { id: "focus-details", onClick: () => nickel.surfaces.focus("details") }, "Focus details"))
        : h("div", { className: "details-content" },
            h(Text, null, "Details window"),
            h(Button, { id: "close-details", onClick: () => nickel.surfaces.hide("details") }, "Close details")));
}
