// @jsx h
// Nickel supplies bounded desktop data and image assets. JSX owns presentation
// and requests file actions; Rust validates the file identity and grants.
/** @typedef {{ label: string, value: string, percent: number, color: number }} DesktopWidget */
/** @typedef {{ width: number, height: number, background: number,
 * wallpaper: boolean, tiles: NickelFileTileProps[], widgets: DesktopWidget[], error: string | null,
 * surfaceColor: number, text: number }} DesktopData */
function App() {
    const data = /** @type {DesktopData} */ ( /** @type {unknown} */(nickel.data));
    return h(Surface, { width: data.width, height: data.height, background: data.background },
        data.wallpaper ? h(Image, { asset: "wallpaper", width: data.width, height: data.height, fit: "stretch" }) : null,
        (data.tiles || []).map(tile => h(FileTile, { key: tile.id, ...tile, onSelect: () => nickel.request({ type: "desktop-select", id: tile.id }), onMove: ({ dx, dy }) => nickel.request({ type: "desktop-move", id: tile.id, dx, dy }), onFileAction: ({ action }) => nickel.request({ type: "desktop-file-action", id: tile.id, action }), onClick: () => nickel.request({ type: "desktop-open", id: tile.id }) })),
        (data.widgets || []).slice(0, 3).map((widget, index) => h(Box, { key: index, x: Math.max(0, data.width - 224), y: 20 + index * 92, width: Math.min(204, data.width), height: 76, background: data.surfaceColor, radius: 10 },
            h(Column, null,
                h(Text, { color: data.text }, widget.label),
                h(Text, { color: widget.color }, widget.value),
                h(Progress, { percent: widget.percent, width: Math.min(180, Math.max(1, data.width - 24)), height: 6 })))),
        data.error ? h(Box, { x: 20, y: 20, width: Math.min(500, Math.max(1, data.width - 40)), height: 52, background: data.surfaceColor, radius: 8 },
            h(Text, { color: data.text }, data.error)) : null);
}
