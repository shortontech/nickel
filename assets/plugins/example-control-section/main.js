// @jsx h
function App() {
    return h(Section, {
        id: "find-apps",
        label: "Applications",
        value: "Search Nickel's catalog",
        onClick: () => nickel.request("show-launcher")
    });
}
