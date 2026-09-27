// @jsx h
// Nickel owns wallpaper pixels and file interactions. The host overlays its
// file plane and menus after this plugin's desktop presentation.
function App() {
    const data = nickel.data;
    return h(Surface, { width: data.width, height: data.height, background: data.background },
        data.wallpaper ? h(Image, {
            asset: "wallpaper",
            width: data.width,
            height: data.height,
            fit: "stretch"
        }) : null,
        data.error ? h(Box, {
            x: 20, y: 20, width: Math.min(500, Math.max(1, data.width - 40)),
            height: 52, background: data.surface, radius: 8
        }, h(Text, { color: data.text }, data.error)) : null);
}
