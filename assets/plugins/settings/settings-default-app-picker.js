// @jsx h
// Candidate controls inside the host-owned association picker popover.
function App() {
    const data = nickel.data;
    return h("settings-stack", null,
        h("settings-card", { label: data.status, value: "" },
            h("settings-input", { id: `default-app-handler-search-${data.row}`, value: data.query, onChange: value => nickel.request({ type: 'search-handlers', value }) })),
        h("settings-compact-list", null, data.handlers.map(handler => h("settings-row", { key: handler.id, label: handler.name, value: handler.detail },
            h("settings-button", { id: `default-app-handler-${handler.id}`, label: handler.current ? data.currentLabel : data.chooseLabel, value: handler.editable ? 'quiet' : 'disabled', onClick: handler.editable ? () => nickel.request({
                    type: 'choose-handler', row: data.row,
                    target: data.target, handler: handler.id
                }) : undefined })))));
}
