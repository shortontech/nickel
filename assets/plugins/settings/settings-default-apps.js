// @jsx h
// Curated default application choices. Rust owns association discovery and
// validates the target again when opening its native picker.
function App() {
    const data = nickel.data;
    return h("settings-stack", null,
        h("settings-compact-list", null, data.rows.map(row => h("settings-row", { key: row.target, label: row.label, value: row.status, compact: true },
            h("settings-button", { id: `default-app-${row.index}`, label: row.current, value: "quiet", accessibilityLabel: `${row.label}: ${row.current}`, onClick: () => nickel.request({ type: 'choose-default', index: row.index, target: row.target }) })))),
        h("settings-card", { label: data.advancedTitle, value: data.advancedStatus },
            h("settings-input", { id: "default-app-advanced-target", value: data.query, onChange: value => nickel.request({ type: 'search-targets', value }) }),
            h("settings-grid", null, data.families.map(family => h("settings-button", { key: family.index, id: `default-app-family-${family.index}`, label: family.label, value: family.selected ? 'primary' : 'quiet', onClick: () => nickel.request({ type: 'set-family', index: family.index }) })))));
}
