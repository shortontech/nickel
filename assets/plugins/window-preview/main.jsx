// @jsx h
// Window identity, thumbnails, and action authority come from the host.
function PreviewCard({window}) {
    const request = action => nickel.request({ type: "preview-action", action, window: window.id });
    return <Column>
        <Row>
            <Text>{window.selected ? "● " + window.title : window.title}</Text>
            {window.closable ? <Button id={"preview-close-" + window.id}
                accessibilityLabel={"Close " + window.title}
                onClick={() => request("close")}>×</Button> : null}
        </Row>
        <ImageButton id={"preview-window-" + window.id}
            asset={"window:" + window.id} width={window.imageWidth} height={116} fit="contain"
            accessibilityLabel={window.accessibleName}
            onClick={() => request("activate")}
            onContextMenu={() => request("menu")} />
    </Column>;
}
function App() {
    return <Window id="main" placement="fixed" width="100%" height="100%" className="window-preview">
        <div className="preview-content">
            {(nickel.data.windows || []).map(window => <PreviewCard key={window.id} window={window} />)}
        </div>
    </Window>;
}
