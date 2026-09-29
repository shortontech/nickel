// @jsx h
// Key meanings and the text recipient remain in Nickel's host.
function App() {
    const data = nickel.data || {};
    const rows = data.rows || [];
    const generation = data.generation || 0;
    const request = (type, fields) => nickel.request(Object.assign({type, generation}, fields || {}));
    const control = (id, label, type, fields) =>
        <Button id={id} onClick={() => request(type, fields)}>{label}</Button>;
    return <FixedWindow width="100%" height="100%" className="keyboard-window"
        onEscape={() => request("keyboard-hide")}>
        <Column className="keyboard-content">
            <Row className="keyboard-toolbar">
                <Text>{data.recipientAvailable ? "English (US)" : "Select a text field"}</Text>
                {control("osk-plugin-hold", "Hold modifiers", "keyboard-hold")}
                {control("osk-plugin-dock", data.dockTop ? "Move down" : "Move up", "keyboard-dock")}
                {control("osk-plugin-hide", "Hide", "keyboard-hide")}
            </Row>
            {rows.map((row, rowIndex) => <Row key={"row-" + rowIndex} className="keyboard-key-row">
                {row.map(key => <Button key={key.id} id={key.id} className="keyboard-key" width={key.quarters * 18}
                    onClick={() => key.enabled && request("keyboard-key", {id: key.id})}>
                    {key.label}
                </Button>)}
            </Row>)}
            <Row className="keyboard-footer">
                {control("osk-plugin-smaller", "Smaller", "keyboard-resize", {delta: -32})}
                {control("osk-plugin-larger", "Larger", "keyboard-resize", {delta: 32})}
            </Row>
        </Column>
    </FixedWindow>;
}
