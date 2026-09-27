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
    const data = nickel.data;
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
        <settings-search-index>
            <settings-search-entry id="appearance-mode-system" state={data.labels.appearance}
                label={data.search.automatic} value={data.search.mode} />
            <settings-search-entry id="appearance-hue" state={data.labels.appearance}
                label={data.search.startingHue} value={data.search.interface} />
            <settings-search-entry id="appearance-intensity" state={data.labels.appearance}
                label={data.search.colorIntensity} value={data.search.interface} />
            <settings-search-entry id="appearance-transparency" state={data.labels.appearance}
                label={data.search.reduceTransparency} value={data.search.interface} />
            <settings-search-entry id="appearance-animations" state={data.labels.appearance}
                label={data.search.animations} value={data.search.interface} />
            <settings-search-entry id="optional-feature-codex-enabled" state={data.labels['optional-features']}
                label="Use Codex projects and conversations in Nickel" value="Codex" />
            <settings-search-entry id="on-screen-keyboard-mode" state={data.labels['optional-features']}
                label="Screen keyboard · touch keyboard · virtual keyboard" value="On-screen keyboard" />
            <settings-search-entry id="plugins-page" state={data.labels.plugins}
                label="Enable or disable shell plugins and review their access" value="Plugin memory and permissions" />
        </settings-search-index>
    </settings-navigation>;
}
