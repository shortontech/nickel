// @jsx h
// Bundled ordinary Settings pages. The Settings host supplies localized data.
function App() {
    const page = nickel.data;
    return <settings-card label={page.title} value={page.description}>
        {(page.rows || []).map(row =>
            <settings-row key={row.id} label={row.label} value={row.value} />)}
    </settings-card>;
}
