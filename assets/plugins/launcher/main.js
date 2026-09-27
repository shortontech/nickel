// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    return h(Column, null,
        h(Text, null, "Nickel Launcher"),
        h(TextField, { id: "launcher-query", value: data.query, placeholder: "Search applications", onChange: query => nickel.request({ type: "launcher-set-query", query }) }),
        h(Column, null,
            data.results.length === 0 ? h(Text, null, "No applications found") : null,
            data.results.map(result => h(Button, { id: "launcher-result-" + result.index, onClick: () => nickel.request({ type: "launcher-activate-result", index: result.index, id: result.id }) }, result.name))));
}
