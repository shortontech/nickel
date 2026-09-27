// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    return h(Column, null,
        h(Text, null, "Nickel Launcher"),
        h(TextField, { id: "launcher-query", value: data.query, placeholder: "Search applications", onChange: query => nickel.request({ type: "launcher-set-query", query }) }),
        data.dashboardVisible ? h(Column, null,
            h(Text, null, "Places"),
            data.places.map(place => h(Button, { id: "launcher-place-" + place.index, onClick: () => nickel.request({ type: "launcher-launch-dashboard", id: place.id }) }, place.name)),
            h(Text, null, "Pinned and recent"),
            data.dashboard.map(app => h(Button, { id: "launcher-dashboard-" + app.index, onClick: () => nickel.request({ type: "launcher-launch-dashboard", id: app.id }) }, app.name))) : null,
        !data.dashboardVisible ?
            h(Column, null,
                data.results.length === 0 ? h(Text, null, "No applications found") : null,
                data.results.map(result => h(Button, { id: "launcher-result-" + result.index, onClick: () => nickel.request({ type: "launcher-activate-result", index: result.index, id: result.id }) }, result.name))) : null);
}
