// @jsx h
import "./styles/preview.css";

export function Preview() {
    const data = useWindowPreviews();
    const windows = data.windows || [];
    const switcher = data.taskSwitcher;
    return <Window id="window-preview" placement="fixed" width="100%" height="100%" className="window-preview">
        <div className="preview-content">{windows.map((window, index) => {
            const title = window.title || window.applicationName || "Untitled window";
            const label = switcher ? `${title}, ${index + 1} of ${windows.length}${window.selected ? ", selected" : ""}` : title;
            return <Column key={window.id}>
                <Row><Text>{window.selected ? "● " + title : title}</Text>
                    {window.canClose ? <Button id={"preview-close-" + window.id} accessibilityLabel={"Close " + title}
                        onClick={() => nickel.windowPreviews.close(window.id, data.revision)}>×</Button> : null}</Row>
                <ImageButton id={"preview-window-" + window.id} asset={window.image} width={switcher ? 188 : 244} height={116}
                    fit="contain" accessibilityLabel={label}
                    onClick={() => nickel.windowPreviews.activate(window.id, data.revision)}
                    onContextMenu={() => nickel.windowPreviews.openMenu(window.id, data.revision)}/>
            </Column>;
        })}</div>
    </Window>;
}
