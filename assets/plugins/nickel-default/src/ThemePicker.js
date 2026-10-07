// @jsx h
const themeKey = theme => theme.id;
export function ThemePicker() {
    const catalog = nickel.plugins.get();
    const [review, setReview] = useState(null);
    const shells = catalog.plugins.filter(plugin => plugin.shell);
    const preview = catalog.shellPreview;
    const canSelect = catalog.available && catalog.writable && !preview;
    const candidate = review && shells.find(shell => shell.id === review.id);
    const name = id => shells.find(shell => shell.id === id)?.name || id;
    return h(Column, { className: "appearance-card" },
        h(Text, { className: "appearance-heading" }, "Shell"),
        h(Text, { wrap: true }, "Switch the complete desktop shell, including its desktop, taskbar, launcher, and settings."),
        h(Text, { wrap: true }, "The selected shell reloads immediately as a temporary preview. Keep it to save the change; otherwise Nickel restores the previous shell automatically."),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Shell selection is unavailable.") : null,
        catalog.available && !catalog.writable ? h(Text, null, "Shell selection is read only.") : null,
        catalog.available && !shells.length ? h(Text, null, "No shells are installed.") : null,
        h(VirtualColumn, { id: "appearance-themes", items: shells, itemKey: themeKey, itemHeight: 60, gap: 8, overscan: 96, renderItem: theme => h(Row, { key: theme.id, className: "appearance-theme-row" },
                h(Text, { wrap: true }, theme.name),
                h(Spacer, null),
                theme.selected ? h(Text, null, "Current shell") : h(Button, { id: "appearance-theme/" + theme.id, accessibilityLabel: "Try " + theme.name, disabled: !canSelect, onClick: () => theme.enabled ? nickel.plugins.selectShell(theme.id, catalog.revision) : setReview({ id: theme.id, revision: catalog.revision }) }, "Try shell")) }),
        catalog.truncated ? h(Text, { wrap: true }, "Some installed shells may be missing from this list.") : null,
        catalog.lastResult ? h(Text, { wrap: true }, catalog.lastResult.detail || ({ preview: "Shell preview started.", confirmed: "Shell saved.", reverted: "Previous shell restored.", rejected: "Shell change rejected." }[catalog.lastResult.status] || "")) : null,
        preview ? h(Column, null,
            h(Text, { wrap: true }, "Previewing " + name(preview.selectedShell) + ". Previous shell: " + name(preview.previousShell) + "."),
            h(Text, { wrap: true }, "Keep this shell or restore the previous one. Unconfirmed previews revert automatically."),
            h(Row, { className: "appearance-choices" },
                h(Button, { id: "appearance-theme-keep", disabled: !catalog.writable || !preview.canConfirm, onClick: () => nickel.plugins.confirmShell(preview.token, catalog.revision) }, "Keep shell"),
                h(Button, { id: "appearance-theme-revert", disabled: !catalog.writable || !preview.canRevert, onClick: () => nickel.plugins.revertShell(preview.token, catalog.revision) }, "Restore previous shell"))) : null,
        review ? h(Column, null,
            h(Text, null, "Review shell access"),
            candidate ? h(Column, null,
                h(Text, null, candidate.name + " · " + (candidate.version || "Unspecified version")),
                h(Text, null, "Publisher: " + (candidate.author || "Unknown")),
                h(Text, { wrap: true }, "Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")),
                h(Text, { wrap: true }, "Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")),
                h(Row, { className: "appearance-choices" },
                    h(Button, { onClick: () => setReview(null) }, "Cancel"),
                    h(Button, { id: "appearance-theme-enable", disabled: !canSelect || catalog.revision !== review.revision, onClick: () => { nickel.plugins.selectShell(candidate.id, review.revision); setReview(null); } }, "Enable and try shell"))) : h(Text, null, "The shell is no longer installed.")) : null);
}
registerSettingsPage({ id: "shell", group: "Personalization", label: "Shell", description: "Switch the desktop, taskbar, launcher, and settings shell", component: ThemePicker });
