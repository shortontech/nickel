// @jsx h
const themeKey = theme => theme.id;
export function ThemePicker() {
    const catalog = nickel.plugins.get();
    const [review, setReview] = useState(null);
    const themes = catalog.plugins.filter(plugin => plugin.shell);
    const preview = catalog.shellPreview;
    const canSelect = catalog.available && catalog.writable && !preview;
    const candidate = review && themes.find(theme => theme.id === review.id);
    const name = id => themes.find(theme => theme.id === id)?.name || id;
    return h(Column, { className: "appearance-card" },
        h(Text, { className: "appearance-heading" }, "Themes"),
        h(Text, { wrap: true }, "Choose the shell used for your desktop, bar, and launcher."),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Themes are unavailable.") : null,
        catalog.available && !catalog.writable ? h(Text, null, "Theme selection is read only.") : null,
        catalog.available && !themes.length ? h(Text, null, "No shell themes are installed.") : null,
        h(VirtualColumn, { id: "appearance-themes", items: themes, itemKey: themeKey, itemHeight: 60, gap: 8, overscan: 96, renderItem: theme => h(Row, { key: theme.id, className: "appearance-theme-row" },
                h(Text, { wrap: true }, theme.name),
                h(Spacer, null),
                theme.selected ? h(Text, null, "Current theme") : h(Button, { id: "appearance-theme/" + theme.id, accessibilityLabel: "Preview " + theme.name, disabled: !canSelect, onClick: () => theme.enabled ? nickel.plugins.selectShell(theme.id, catalog.revision) : setReview({ id: theme.id, revision: catalog.revision }) }, "Preview")) }),
        catalog.truncated ? h(Text, { wrap: true }, "Some installed themes may be missing from this list.") : null,
        catalog.lastResult ? h(Text, { wrap: true }, catalog.lastResult.detail || ({ preview: "Theme preview started.", confirmed: "Theme saved.", reverted: "Previous theme restored.", rejected: "Theme change rejected." }[catalog.lastResult.status] || "")) : null,
        preview ? h(Column, null,
            h(Text, { wrap: true }, "Previewing " + name(preview.selectedShell) + ". Previous theme: " + name(preview.previousShell) + "."),
            h(Text, { wrap: true }, "Keep this theme or restore the previous one. Unconfirmed previews revert automatically."),
            h(Row, { className: "appearance-choices" },
                h(Button, { id: "appearance-theme-keep", disabled: !catalog.writable || !preview.canConfirm, onClick: () => nickel.plugins.confirmShell(preview.token, catalog.revision) }, "Keep theme"),
                h(Button, { id: "appearance-theme-revert", disabled: !catalog.writable || !preview.canRevert, onClick: () => nickel.plugins.revertShell(preview.token, catalog.revision) }, "Restore previous theme"))) : null,
        review ? h(Column, null,
            h(Text, null, "Review theme access"),
            candidate ? h(Column, null,
                h(Text, null, candidate.name + " · " + (candidate.version || "Unspecified version")),
                h(Text, null, "Publisher: " + (candidate.author || "Unknown")),
                h(Text, { wrap: true }, "Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")),
                h(Text, { wrap: true }, "Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")),
                h(Row, { className: "appearance-choices" },
                    h(Button, { onClick: () => setReview(null) }, "Cancel"),
                    h(Button, { id: "appearance-theme-enable", disabled: !canSelect || catalog.revision !== review.revision, onClick: () => { nickel.plugins.selectShell(candidate.id, review.revision); setReview(null); } }, "Enable and preview theme"))) : h(Text, null, "The theme is no longer installed.")) : null);
}
