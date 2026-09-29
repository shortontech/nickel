// @jsx h
// Rust owns association discovery and the virtual catalog geometry.
function App() {
    const data = nickel.data;
    return <div className="default-app-page">
        <div className="default-app-curated">
            {data.rows.map(row => <div key={row.target} className="default-app-row">
                <div className="default-app-label">
                    <Text className="default-app-name">{row.label}</Text>
                    {row.status ? <Text className="default-app-detail" wrap={true}>{row.status}</Text> : null}
                </div>
                <Button id={`default-app-${row.index}`} className="default-app-action"
                    accessibilityLabel={`${row.label}: ${row.current}`}
                    onClick={() => nickel.request({type: 'choose-default', index: row.index, target: row.target})}>{row.current}</Button>
            </div>)}
        </div>
        <div className="default-app-advanced">
            <Text className="default-app-heading">{data.advancedTitle}</Text>
            {data.advancedStatus ? <Text className="default-app-detail" wrap={true}>{data.advancedStatus}</Text> : null}
            <TextField id="default-app-advanced-target" className="default-app-search"
                value={data.query} placeholder={data.searchPlaceholder}
                onChange={value => nickel.request({type: 'search-targets', value})} />
            <div className="default-app-families">
                {data.families.map(family => <Button key={family.index}
                    id={`default-app-family-${family.index}`}
                    className={family.selected ? 'default-app-family selected' : 'default-app-family'}
                    onClick={() => nickel.request({type: 'set-family', index: family.index})}>{family.label}</Button>)}
            </div>
        </div>
        <div className="default-app-catalog">
            {data.catalogRows.map(row => <div key={row.key} className="default-app-row">
                <div className="default-app-label">
                    <Text className="default-app-name">{row.key}</Text>
                    <Text className="default-app-detail">{row.family}</Text>
                </div>
                <Button id={`default-app-target-${row.index}`} className="default-app-action"
                    onClick={() => nickel.request({type: 'browse-target', index: row.index, key: row.key})}>Choose app</Button>
            </div>)}
        </div>
    </div>;
}
