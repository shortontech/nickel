// @jsx h
// Bundled ordinary Settings pages. The Settings host supplies localized data.
function App() {
    const page = nickel.data;
    return h("div", { className: "settings-card" },
        h(Text, { className: "settings-title", wrap: true }, page.title),
        page.description ? h(Text, { className: "settings-description", wrap: true }, page.description) : null,
        (page.rows || []).map(row => h("div", { key: row.id, className: "settings-row" },
            h(Text, { className: "settings-label", wrap: true }, row.label),
            h(Text, { className: "settings-value", wrap: true }, row.value))));
}
