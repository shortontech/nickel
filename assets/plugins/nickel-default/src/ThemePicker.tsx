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
    return <Column className="appearance-card">
        <Text className="appearance-heading">Shell</Text>
        <Text wrap={true}>Switch the complete desktop shell, including its desktop, taskbar, launcher, and settings.</Text>
        <Text wrap={true}>The selected shell reloads immediately as a temporary preview. Keep it to save the change; otherwise Nickel restores the previous shell automatically.</Text>
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Shell selection is unavailable."}</Text> : null}
        {catalog.available && !catalog.writable ? <Text>Shell selection is read only.</Text> : null}
        {catalog.available && !shells.length ? <Text>No shells are installed.</Text> : null}
        <VirtualColumn id="appearance-themes" items={shells} itemKey={themeKey}
            itemHeight={60} gap={8} overscan={96}
            renderItem={theme => <Row key={theme.id} className="appearance-theme-row">
                <Text wrap={true}>{theme.name}</Text><Spacer />
                {theme.selected ? <Text>Current shell</Text> : <Button id={"appearance-theme/" + theme.id}
                    accessibilityLabel={"Try " + theme.name} disabled={!canSelect}
                    onClick={() => theme.enabled ? nickel.plugins.selectShell(theme.id, catalog.revision) : setReview({id:theme.id,revision:catalog.revision})}>Try shell</Button>}
            </Row>} />
        {catalog.truncated ? <Text wrap={true}>Some installed shells may be missing from this list.</Text> : null}
        {catalog.lastResult ? <Text wrap={true}>{catalog.lastResult.detail || ({preview:"Shell preview started.",confirmed:"Shell saved.",reverted:"Previous shell restored.",rejected:"Shell change rejected."}[catalog.lastResult.status] || "")}</Text> : null}
        {preview ? <Column>
            <Text wrap={true}>{"Previewing " + name(preview.selectedShell) + ". Previous shell: " + name(preview.previousShell) + "."}</Text>
            <Text wrap={true}>Keep this shell or restore the previous one. Unconfirmed previews revert automatically.</Text>
            <Row className="appearance-choices">
                <Button id="appearance-theme-keep" disabled={!catalog.writable || !preview.canConfirm}
                    onClick={() => nickel.plugins.confirmShell(preview.token, catalog.revision)}>Keep shell</Button>
                <Button id="appearance-theme-revert" disabled={!catalog.writable || !preview.canRevert}
                    onClick={() => nickel.plugins.revertShell(preview.token, catalog.revision)}>Restore previous shell</Button>
            </Row>
        </Column> : null}
        {review ? <Column>
            <Text>Review shell access</Text>
            {candidate ? <Column>
                <Text>{candidate.name + " · " + (candidate.version || "Unspecified version")}</Text>
                <Text>{"Publisher: " + (candidate.author || "Unknown")}</Text>
                <Text wrap={true}>{"Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")}</Text>
                <Text wrap={true}>{"Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")}</Text>
                <Row className="appearance-choices">
                    <Button onClick={() => setReview(null)}>Cancel</Button>
                    <Button id="appearance-theme-enable" disabled={!canSelect || catalog.revision !== review.revision}
                        onClick={() => {nickel.plugins.selectShell(candidate.id, review.revision); setReview(null);}}>Enable and try shell</Button>
                </Row>
            </Column> : <Text>The shell is no longer installed.</Text>}
        </Column> : null}
    </Column>;
}

registerSettingsPage({id:"shell", group:"Personalization", label:"Shell", description:"Switch the desktop, taskbar, launcher, and settings shell", component:ThemePicker});
