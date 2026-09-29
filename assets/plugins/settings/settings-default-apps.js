// @jsx h
// Rust owns association discovery and the virtual catalog geometry.
function App() {
    const data = nickel.data;
    return h("div", { className: "default-app-page" },
        h("div", { className: "default-app-curated" }, data.rows.map(row => h("div", { key: row.target, className: "default-app-row" },
            h("div", { className: "default-app-label" },
                h(Text, { className: "default-app-name" }, row.label),
                row.status ? h(Text, { className: "default-app-detail", wrap: true }, row.status) : null),
            h(Button, { id: `default-app-${row.index}`, className: "default-app-action", accessibilityLabel: `${row.label}: ${row.current}`, onClick: () => nickel.request({ type: 'choose-default', index: row.index, target: row.target }) }, row.current)))),
        h("div", { className: "default-app-advanced" },
            h(Text, { className: "default-app-heading" }, data.advancedTitle),
            data.advancedStatus ? h(Text, { className: "default-app-detail", wrap: true }, data.advancedStatus) : null,
            h(TextField, { id: "default-app-advanced-target", className: "default-app-search", value: data.query, placeholder: data.searchPlaceholder, onChange: value => nickel.request({ type: 'search-targets', value }) }),
            h("div", { className: "default-app-families" }, data.families.map(family => h(Button, { key: family.index, id: `default-app-family-${family.index}`, className: family.selected ? 'default-app-family selected' : 'default-app-family', onClick: () => nickel.request({ type: 'set-family', index: family.index }) }, family.label)))),
        h("div", { className: "default-app-catalog" }, data.catalogRows.map(row => h("div", { key: row.key, className: "default-app-row" },
            h("div", { className: "default-app-label" },
                h(Text, { className: "default-app-name" }, row.key),
                h(Text, { className: "default-app-detail" }, row.family)),
            h(Button, { id: `default-app-target-${row.index}`, className: "default-app-action", onClick: () => nickel.request({ type: 'browse-target', index: row.index, key: row.key }) }, "Choose app")))));
}
