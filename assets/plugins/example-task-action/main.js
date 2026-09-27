// @jsx h
function App() {
    return h(Action, {
        id: "find-apps",
        label: "Find apps",
        onClick: applicationId => nickel.request("show-launcher")
    });
}
