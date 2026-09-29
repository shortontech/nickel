// @jsx h
// The native host owns Settings values and validates navigation requests.
function App() {
    const data = nickel.data || {};
    const wide = (data.width || 1100) >= 720;
    const showNavigation = wide || !data.active;
    const showContent = wide || data.active;
    const request = (type, fields) => nickel.request(Object.assign({ type }, fields || {}));
    return h(Window, { id: "main", title: "Nickel Settings", width: "100%", height: "100%", className: "settings-window" },
        h("div", { className: wide ? "settings-shell wide" : "settings-shell narrow" },
            showNavigation ? h("div", { className: wide ? "settings-sidebar" : "settings-sidebar narrow" },
                h(TextField, { id: "settings-sidebar-search", className: "settings-search", value: data.query || "", placeholder: data.searchPlaceholder || "Search Settings", onChange: value => request("search", { value }) }),
                h(ScrollView, { id: "settings-sidebar-scroll", height: Math.max(1, (data.height || 800) - 64) },
                    h(Column, { className: "settings-destinations" },
                        data.query ? (data.results || []).map(result => h(Button, { key: result.target, id: "search-result-" + result.target, disabled: !result.available, className: "settings-destination", onClick: () => request("navigate-target", { target: result.target }) }, result.label)) : (data.destinations || []).map(destination => h(Column, { key: destination.id },
                            destination.section ? h(Text, { className: "settings-section" }, destination.section) : null,
                            h(Button, { id: "settings-navigation/destination/" + destination.id, state: destination.active ? "selected" : "unselected", className: destination.active ? "settings-destination active" : "settings-destination", onClick: () => request("navigate", { page: destination.id }) }, destination.label))),
                        data.query && !(data.results || []).length ? h(Text, { className: "settings-empty" }, data.noResults || "No results") : null))) : null,
            showContent ? h("div", { className: "settings-detail" },
                h(Row, { className: "settings-heading" },
                    !wide ? h(Button, { id: "settings-show-navigation", className: "settings-back", onClick: () => request("show-navigation") }, "\u2039") : null,
                    h(Column, null,
                        h(Text, { className: "settings-title" }, data.title || "Settings"),
                        data.subtitle ? h(Text, { className: "settings-subtitle", wrap: true }, data.subtitle) : null)),
                h(Slot, { id: "settings-content", className: "settings-content" })) : null));
}
