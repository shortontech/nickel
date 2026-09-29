// @jsx h
// Nickel supplies bounded desktop data and image assets. JSX owns presentation
// and requests file actions; Rust validates the file identity and grants.
/** @typedef {{ label: string, value: string, percent: number, color: number }} DesktopWidget */
/** @typedef {{ kind: "file", id: string } | { kind: "background", x: number, y: number,
 * iconsVisible: boolean, iconWidth: number, arrangement: string, foldersFirst: boolean,
 * pasteAvailable: boolean, desktopWritable: boolean }} DesktopContext */
/** @typedef {{ width: number, height: number, background: number,
 * wallpaper: boolean, tiles: NickelFileTileProps[], widgets: DesktopWidget[], error: string | null,
 * context: DesktopContext | null,
 * surfaceColor: number, text: number }} DesktopData */
function App() {
    const data = /** @type {DesktopData} */ ( /** @type {unknown} */(nickel.data));
    const fileContext = data.context?.kind === "file" ? data.context : null;
    const background = data.context?.kind === "background" ? data.context : null;
    /** @param {string} action */
    const backgroundAction = (action) => nickel.request({ type: "desktop-background-action", action });
    /** @param {boolean} selected @param {string} label */
    const checked = (selected, label) => selected ? `✓ ${label}` : label;
    return h(FixedWindow, { id: "main", width: "100%", height: "100%", background: data.background, 'aria-label': "Desktop" },
        data.wallpaper ? h(Image, { asset: "wallpaper", width: data.width, height: data.height, fit: "stretch" }) : null,
        (data.tiles || []).map(tile => h(FileTile, { key: tile.id, ...tile, onSelect: () => nickel.request({ type: "desktop-select", id: tile.id }), onMove: ({ dx, dy }) => nickel.request({ type: "desktop-move", id: tile.id, dx, dy }), onFileAction: ({ action }) => nickel.request({ type: "desktop-file-action", id: tile.id, action }), onClick: () => nickel.request({ type: "desktop-open", id: tile.id }) })),
        fileContext ? h(Menu, { id: "desktop-file-actions", anchor: fileContext.id, open: true },
            h(MenuItem, { id: "open", onClick: () => nickel.request({ type: "desktop-open", id: fileContext.id }) }, "Open"),
            h(MenuItem, { id: "cut", onClick: () => nickel.request({ type: "desktop-file-action", id: fileContext.id, action: "cut" }) }, "Cut"),
            h(MenuItem, { id: "copy", onClick: () => nickel.request({ type: "desktop-file-action", id: fileContext.id, action: "copy" }) }, "Copy"),
            h(MenuItem, { id: "rename", onClick: () => nickel.request({ type: "desktop-file-action", id: fileContext.id, action: "rename" }) }, "Rename"),
            h(MenuItem, { id: "properties", onClick: () => nickel.request({ type: "desktop-file-action", id: fileContext.id, action: "properties" }) }, "Properties"),
            h(MenuItem, { id: "open-terminal", onClick: () => nickel.request({ type: "desktop-file-action", id: fileContext.id, action: "open-terminal" }) }, "Open in Terminal")) : null,
        background ? h(Menu, { id: "desktop-background-actions", anchor: "main", x: background.x, y: background.y, open: true },
            h(MenuItem, { id: "view", label: "View" },
                h(MenuItem, { id: "toggle-icons", shortcut: "Ctrl+Shift+D", onClick: () => backgroundAction("toggle-icons") }, background.iconsVisible ? "Hide desktop icons" : "Show desktop icons"),
                h(MenuItem, { id: "small-icons", onClick: () => backgroundAction("small-icons") }, checked(background.iconWidth <= 72, "Small icons")),
                h(MenuItem, { id: "medium-icons", onClick: () => backgroundAction("medium-icons") }, checked(background.iconWidth > 72 && background.iconWidth < 128, "Medium icons")),
                h(MenuItem, { id: "large-icons", onClick: () => backgroundAction("large-icons") }, checked(background.iconWidth >= 128, "Large icons")),
                h(MenuItem, { id: "align-grid", onClick: () => backgroundAction("align-grid") }, "Align to Grid"),
                h(MenuItem, { id: "auto-arrange", onClick: () => backgroundAction("auto-arrange") }, "Auto Arrange"),
                h(MenuItem, { id: "folders-first", onClick: () => backgroundAction("folders-first") }, checked(background.foldersFirst, "Folders first")),
                h(MenuItem, { id: "folders-mixed", onClick: () => backgroundAction("folders-mixed") }, checked(!background.foldersFirst, "Mix folders and files"))),
            h(MenuItem, { id: "sort-by", label: "Sort By" },
                h(MenuItem, { id: "sort-name", onClick: () => backgroundAction("sort-name") }, checked(background.arrangement === "sort-name", "Name (ascending)")),
                h(MenuItem, { id: "sort-name-descending", onClick: () => backgroundAction("sort-name-descending") }, checked(background.arrangement === "sort-name-descending", "Name (descending)")),
                h(MenuItem, { id: "sort-kind", onClick: () => backgroundAction("sort-kind") }, checked(background.arrangement === "sort-kind", "Type (ascending)")),
                h(MenuItem, { id: "sort-kind-descending", onClick: () => backgroundAction("sort-kind-descending") }, checked(background.arrangement === "sort-kind-descending", "Type (descending)")),
                h(MenuItem, { id: "sort-size", onClick: () => backgroundAction("sort-size") }, checked(background.arrangement === "sort-size", "Size (ascending)")),
                h(MenuItem, { id: "sort-size-descending", onClick: () => backgroundAction("sort-size-descending") }, checked(background.arrangement === "sort-size-descending", "Size (descending)")),
                h(MenuItem, { id: "sort-modified", onClick: () => backgroundAction("sort-modified") }, checked(background.arrangement === "sort-modified", "Modified (newest first)")),
                h(MenuItem, { id: "sort-modified-ascending", onClick: () => backgroundAction("sort-modified-ascending") }, checked(background.arrangement === "sort-modified-ascending", "Modified (oldest first)")),
                h(MenuItem, { id: "manual", onClick: () => backgroundAction("manual") }, checked(background.arrangement === "manual", "Manual arrangement"))),
            h(MenuItem, { id: "refresh", shortcut: "F5", onClick: () => backgroundAction("refresh") }, "Refresh"),
            h(MenuItem, { id: "paste", shortcut: "Ctrl+V", disabledReason: background.pasteAvailable ? undefined : "File clipboard is empty", onClick: background.pasteAvailable ? () => backgroundAction("paste") : undefined }, "Paste"),
            h(MenuItem, { id: "new-folder", disabledReason: background.desktopWritable ? undefined : "Desktop location is not writable", onClick: background.desktopWritable ? () => backgroundAction("new-folder") : undefined }, "New Folder"),
            h(MenuItem, { id: "display-settings", separatorBefore: true, onClick: () => backgroundAction("display-settings") }, "Display Settings"),
            h(MenuItem, { id: "personalize", onClick: () => backgroundAction("personalize") }, "Personalize")) : null,
        (data.widgets || []).slice(0, 3).map((widget, index) => h(Box, { key: index, x: Math.max(0, data.width - 224), y: 20 + index * 92, width: Math.min(204, data.width), height: 76, background: data.surfaceColor, radius: 10 },
            h(Column, null,
                h(Text, { color: data.text }, widget.label),
                h(Text, { color: widget.color }, widget.value),
                h(Progress, { percent: widget.percent, width: Math.min(180, Math.max(1, data.width - 24)), height: 6 })))),
        data.error ? h(Box, { x: 20, y: 20, width: Math.min(500, Math.max(1, data.width - 40)), height: 52, background: data.surfaceColor, radius: 8 },
            h(Text, { color: data.text }, data.error)) : null);
}
