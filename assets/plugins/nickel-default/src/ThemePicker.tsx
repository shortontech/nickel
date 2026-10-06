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
    return <Column className="appearance-card">
        <Text className="appearance-heading">Themes</Text>
        <Text wrap={true}>Choose the shell used for your desktop, bar, and launcher.</Text>
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Themes are unavailable."}</Text> : null}
        {catalog.available && !catalog.writable ? <Text>Theme selection is read only.</Text> : null}
        {catalog.available && !themes.length ? <Text>No shell themes are installed.</Text> : null}
        <VirtualColumn id="appearance-themes" items={themes} itemKey={themeKey}
            itemHeight={60} gap={8} overscan={96}
            renderItem={theme => <Row key={theme.id} className="appearance-theme-row">
                <Text wrap={true}>{theme.name}</Text><Spacer />
                {theme.selected ? <Text>Current theme</Text> : <Button id={"appearance-theme/" + theme.id}
                    accessibilityLabel={"Preview " + theme.name} disabled={!canSelect}
                    onClick={() => theme.enabled ? nickel.plugins.selectShell(theme.id, catalog.revision) : setReview({id:theme.id,revision:catalog.revision})}>Preview</Button>}
            </Row>} />
        {catalog.truncated ? <Text wrap={true}>Some installed themes may be missing from this list.</Text> : null}
        {catalog.lastResult ? <Text wrap={true}>{catalog.lastResult.detail || ({preview:"Theme preview started.",confirmed:"Theme saved.",reverted:"Previous theme restored.",rejected:"Theme change rejected."}[catalog.lastResult.status] || "")}</Text> : null}
        {preview ? <Column>
            <Text wrap={true}>{"Previewing " + name(preview.selectedShell) + ". Previous theme: " + name(preview.previousShell) + "."}</Text>
            <Text wrap={true}>Keep this theme or restore the previous one. Unconfirmed previews revert automatically.</Text>
            <Row className="appearance-choices">
                <Button id="appearance-theme-keep" disabled={!catalog.writable || !preview.canConfirm}
                    onClick={() => nickel.plugins.confirmShell(preview.token, catalog.revision)}>Keep theme</Button>
                <Button id="appearance-theme-revert" disabled={!catalog.writable || !preview.canRevert}
                    onClick={() => nickel.plugins.revertShell(preview.token, catalog.revision)}>Restore previous theme</Button>
            </Row>
        </Column> : null}
        {review ? <Column>
            <Text>Review theme access</Text>
            {candidate ? <Column>
                <Text>{candidate.name + " · " + (candidate.version || "Unspecified version")}</Text>
                <Text>{"Publisher: " + (candidate.author || "Unknown")}</Text>
                <Text wrap={true}>{"Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")}</Text>
                <Text wrap={true}>{"Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")}</Text>
                <Row className="appearance-choices">
                    <Button onClick={() => setReview(null)}>Cancel</Button>
                    <Button id="appearance-theme-enable" disabled={!canSelect || catalog.revision !== review.revision}
                        onClick={() => {nickel.plugins.selectShell(candidate.id, review.revision); setReview(null);}}>Enable and preview theme</Button>
                </Row>
            </Column> : <Text>The theme is no longer installed.</Text>}
        </Column> : null}
    </Column>;
}
