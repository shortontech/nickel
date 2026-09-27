// @jsx h
// Nickel supplies bounded desktop data and image assets. JSX owns presentation
// and requests file activation; Rust validates the current file identity.
/** @typedef {{ width: number, height: number, background: number,
 * wallpaper: boolean, tiles: NickelFileTileProps[], error: string | null,
 * surfaceColor: number, text: number }} DesktopData */
function App() {
    const data = /** @type {DesktopData} */ ( /** @type {unknown} */(nickel.data));
    return h(Surface, { width: data.width, height: data.height, background: data.background },
        data.wallpaper ? h(Image, { asset: "wallpaper", width: data.width, height: data.height, fit: "stretch" }) : null,
        (data.tiles || []).map(tile => h(FileTile, { key: tile.id, ...tile, onClick: () => nickel.request({ type: "desktop-open", id: tile.id }) })),
        data.error ? h(Box, { x: 20, y: 20, width: Math.min(500, Math.max(1, data.width - 40)), height: 52, background: data.surfaceColor, radius: 8 },
            h(Text, { color: data.text }, data.error)) : null);
}
