// @jsx h
// Candidate controls inside the host-owned association picker popover.
function App() {
    const data = nickel.data;
    return h("div", { className: "app-picker-page" },
        h("div", { className: "app-picker-header" },
            h(Text, { className: "app-picker-status", wrap: true }, data.status),
            h(TextField, { id: `default-app-handler-search-${data.row}`, className: "app-picker-search", value: data.query, placeholder: data.searchPlaceholder, onChange: value => nickel.request({ type: 'search-handlers', value }) })),
        h(ScrollView, { id: `default-app-handler-scroll-${data.row}`, height: 280 },
            h("div", { className: "app-picker-candidates" },
                data.handlerState ? h(Text, { className: "app-picker-detail" }, data.handlerState) : null,
                data.handlerHasPages ? h("div", { className: "app-picker-pages" },
                    h(Button, { id: "default-app-handler-previous", className: "app-picker-action", disabled: !data.handlerCanPrevious, onClick: () => nickel.request({ type: 'page-handlers', direction: 'previous' }) }, "Previous"),
                    h(Text, { className: "app-picker-detail" }, data.handlerPageLabel),
                    h(Button, { id: "default-app-handler-next", className: "app-picker-action", disabled: !data.handlerCanNext, onClick: () => nickel.request({ type: 'page-handlers', direction: 'next' }) }, "Next")) : null,
                data.handlers.map(handler => h("div", { key: handler.id, className: "app-picker-row" },
                    h("div", { className: "app-picker-label" },
                        h(Text, { className: "app-picker-name" }, handler.name),
                        h(Text, { className: "app-picker-detail" }, handler.detail)),
                    h(Button, { id: `default-app-handler-${handler.id}`, className: handler.editable ? 'app-picker-action' : 'app-picker-action disabled', disabled: !handler.editable, onClick: () => nickel.request({
                            type: 'choose-handler', row: data.row,
                            target: data.target, handler: handler.id
                        }) }, handler.current ? data.currentLabel : data.chooseLabel))))));
}
