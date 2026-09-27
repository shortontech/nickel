// @jsx h
// Curated default application choices. Rust owns association discovery and
// validates the target again when opening its native picker.
function App() {
    const data = nickel.data;
    return <settings-stack>
        <settings-compact-list>
            {data.rows.map(row => <settings-row key={row.target} label={row.label} value={row.status} compact={true}>
                <settings-button id={`default-app-${row.index}`}
                    label={row.current} value="quiet"
                    accessibilityLabel={`${row.label}: ${row.current}`}
                    onClick={() => nickel.request({type: 'choose-default', index: row.index, target: row.target})} />
            </settings-row>)}
        </settings-compact-list>
        <settings-card label={data.advancedTitle} value={data.advancedStatus}>
            <settings-input id="default-app-advanced-target" value={data.query}
                onChange={value => nickel.request({type: 'search-targets', value})} />
            <settings-grid>
                {data.families.map(family => <settings-button key={family.index}
                    id={`default-app-family-${family.index}`}
                    label={family.label} value={family.selected ? 'primary' : 'quiet'}
                    onClick={() => nickel.request({type: 'set-family', index: family.index})} />)}
            </settings-grid>
        </settings-card>
        <settings-compact-list>
            {data.catalogRows.map(row => <settings-row key={row.key}
                label={row.key} value={row.family}>
                <settings-button id={`default-app-target-${row.index}`}
                    label="Choose app" value="quiet"
                    onClick={() => nickel.request({type: 'browse-target', index: row.index, key: row.key})} />
            </settings-row>)}
        </settings-compact-list>
    </settings-stack>;
}
