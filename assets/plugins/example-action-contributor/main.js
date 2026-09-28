// @jsx h
function App() {
    return h(Action, { id: "open-launcher", label: "Open launcher",
        onClick: () => nickel.request('show-launcher') });
}
