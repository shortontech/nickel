// @jsx h
// Page order and grouping for the ordinary Settings navigation shell.
function Destination(props) {
    const data = nickel.data;
    const header = data.headers[props.id];
    return <settings-destination id={props.id} label={data.labels[props.id]}
        value={props.section ? data.sections[props.section] : ''}>
        <settings-header label={header.title} value={header.subtitle} />
    </settings-destination>;
}

function App() {
    return <settings-navigation>
        <Destination id="display" section="system" />
        <Destination id="bar" section="personalization" />
        <Destination id="appearance" />
        <Destination id="network" section="connectivity" />
        <Destination id="bluetooth" />
        <Destination id="bluetooth-pair" />
        <Destination id="default-apps" />
        <Destination id="optional-features" />
        <Destination id="plugins" />
        <Destination id="keyboard-shortcuts" section="support" />
        <Destination id="about" />
    </settings-navigation>;
}
