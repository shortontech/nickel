// @jsx h
// Window identity, thumbnails, and action authority come from the host.
function PreviewCard(props) {
    const window = props.window;
    const request = action => nickel.request({ type: "preview-action", action, window: window.id });
    return h(Column, null,
        h(Row, null,
            h(Text, null, window.selected ? "● " + window.title : window.title),
            window.closable ? h(Button, {
                id: "preview-close-" + window.id,
                accessibilityLabel: "Close " + window.title,
                onClick: () => request("close")
            }, "×") : null),
        h(ImageButton, {
            id: "preview-window-" + window.id,
            asset: "window:" + window.id,
            width: window.imageWidth,
            height: 116,
            fit: "contain",
            accessibilityLabel: window.accessibleName,
            onClick: () => request("activate"),
            onContextMenu: () => request("menu")
        }));
}
function App() {
    const data = nickel.data;
    return h(Panel, { height: 214, background: 0xf1222730 },
        h(Row, null, data.windows.map(window => h(PreviewCard, { key: window.id, window }))));
}
