// @jsx h
// Window identity, thumbnails, and action authority come from the host.
function PreviewCard({ window }) {
    const request = action => nickel.request({ type: "preview-action", action, window: window.id });
    return h(Column, null,
        h(Row, null,
            h(Text, null, window.selected ? "● " + window.title : window.title),
            window.closable ? h(Button, { id: "preview-close-" + window.id, accessibilityLabel: "Close " + window.title, onClick: () => request("close") }, "\u00D7") : null),
        h(ImageButton, { id: "preview-window-" + window.id, asset: "window:" + window.id, width: window.imageWidth, height: 116, fit: "contain", accessibilityLabel: window.accessibleName, onClick: () => request("activate"), onContextMenu: () => request("menu") }));
}
function App() {
    return h(Window, { id: "main", placement: "fixed", width: "100%", height: "100%", className: "window-preview" },
        h("div", { className: "preview-content" }, (nickel.data.windows || []).map(window => h(PreviewCard, { key: window.id, window: window }))));
}
