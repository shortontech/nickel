// @jsx h
// Pixels, selection policy, cropping and persistence remain in Nickel's host.
function App() {
    const data = nickel.data || {};
    const width = Math.max(1, data.width || 1);
    const height = Math.max(1, data.height || 1);
    const generation = data.generation || 0;
    const request = action => nickel.request({ type: "screenshot-action", action, generation });
    const button = (id, label, action, buttonWidth) => h(Button, { id: id, width: buttonWidth, onClick: () => request(action) }, label);
    const preview = data.imageRect || { x: 0, y: 70, width: 1, height: 1 };
    const selection = data.selection;
    return h(Surface, { id: "screenshot", width: width, height: height, background: 0xff11151b },
        h(Box, { x: 0, y: 0, width: width, height: 70, background: 0xff242b35 },
            h(Row, null,
                h(Text, { color: 0xffdce3eb }, data.status || "Preparing screenshot…"),
                data.confirmed ? button("screenshot-copy", "Copy", "copy", 112) : null,
                data.confirmed ? button("screenshot-save", "Save", "save", 104) : null,
                data.confirmed ? button("screenshot-path", "Copy file path", "temporary-path", 158) : null,
                button("screenshot-cancel", "Cancel", "cancel", 96))),
        data.imageAvailable ? h(Box, { x: preview.x, y: preview.y, width: preview.width, height: preview.height },
            h(Image, { asset: "capture", width: preview.width, height: preview.height, fit: "stretch" })) : null,
        selection ? h(Box, { x: selection.x, y: selection.y, width: selection.width, height: 2, background: 0xff81b6ff }) : null,
        selection ? h(Box, { x: selection.x, y: selection.y + selection.height - 2, width: selection.width, height: 2, background: 0xff81b6ff }) : null,
        selection ? h(Box, { x: selection.x, y: selection.y, width: 2, height: selection.height, background: 0xff81b6ff }) : null,
        selection ? h(Box, { x: selection.x + selection.width - 2, y: selection.y, width: 2, height: selection.height, background: 0xff81b6ff }) : null);
}
