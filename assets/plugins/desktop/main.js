// @jsx h
// Nickel owns wallpaper pixels and file interactions. This plugin paints the
// desktop base; the host overlays its file plane and menus afterward.
function App() {
    const data = nickel.data;
    return h(Surface, { width: data.width, height: data.height, background: data.background },
        data.wallpaper ? h(Image, {
            asset: "wallpaper",
            width: data.width,
            height: data.height,
            fit: "stretch"
        }) : null);
}
