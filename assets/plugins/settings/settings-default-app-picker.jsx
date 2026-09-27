// @jsx h
// Candidate controls inside the host-owned association picker popover.
function App() {
    const data = nickel.data;
    return <settings-stack>
        <settings-card label={data.status} value="">
            <settings-input id={`default-app-handler-search-${data.row}`}
                value={data.query}
                onChange={value => nickel.request({type: 'search-handlers', value})} />
        </settings-card>
        <settings-compact-list>
            {data.handlers.map(handler => <settings-row key={handler.id}
                label={handler.name} value={handler.detail}>
                <settings-button id={`default-app-handler-${handler.id}`}
                    label={handler.current ? data.currentLabel : data.chooseLabel}
                    value={handler.editable ? 'quiet' : 'disabled'}
                    onClick={handler.editable ? () => nickel.request({
                        type: 'choose-handler', row: data.row,
                        target: data.target, handler: handler.id
                    }) : undefined} />
            </settings-row>)}
        </settings-compact-list>
    </settings-stack>;
}
