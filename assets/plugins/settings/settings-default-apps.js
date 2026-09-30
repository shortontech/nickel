// @jsx h
// Association discovery stays native; this component owns the bounded catalog view.
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
        h("div", { className: "default-app-catalog" },
            data.catalogLoading ? h(Text, { className: "default-app-detail" }, "Loading file and protocol associations\u2026") : null,
            !data.catalogLoading && data.catalogTotal === 0 ? h(Text, { className: "default-app-detail" }, data.catalogEmpty) : null,
            data.catalogHasPages ? h("div", { className: "default-app-pages" },
                h(Button, { id: "default-app-previous", className: "default-app-family", disabled: !data.catalogCanPrevious, onClick: () => nickel.request({ type: 'page-targets', direction: 'previous' }) }, "Previous"),
                h(Text, { className: "default-app-detail" }, data.catalogPageLabel),
                h(Button, { id: "default-app-next", className: "default-app-family", disabled: !data.catalogCanNext, onClick: () => nickel.request({ type: 'page-targets', direction: 'next' }) }, "Next")) : null,
            data.catalogRows.map(row => h("div", { key: row.key, className: "default-app-row" },
                h("div", { className: "default-app-label" },
                    h(Text, { className: "default-app-name" }, row.key),
                    h(Text, { className: "default-app-detail" }, row.family)),
                h(Button, { id: `default-app-target-${row.index}`, className: "default-app-action", onClick: () => nickel.request({ type: 'browse-target', index: row.index, key: row.key }) }, "Choose app")))));
}
