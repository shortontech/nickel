// @jsx h
// Bundled ordinary Settings pages. The Settings host supplies localized data.
function App() {
    const page = nickel.data;
    return <div className="settings-card">
        <Text className="settings-title" wrap={true}>{page.title}</Text>
        {page.description ? <Text className="settings-description" wrap={true}>{page.description}</Text> : null}
        {(page.rows || []).map(row =>
            <div key={row.id} className="settings-row">
                <Text className="settings-label" wrap={true}>{row.label}</Text>
                <Text className="settings-value" wrap={true}>{row.value}</Text>
            </div>)}
    </div>;
}
