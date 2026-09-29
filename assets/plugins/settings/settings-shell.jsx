// @jsx h
// The native host owns Settings values and validates navigation requests.

// Page order and grouping for the ordinary Settings navigation shell.
function SettingsDestination(props) {
    const data = nickel.data;
    const header = data.headers[props.id];
    return <settings-destination id={props.id} label={data.labels[props.id]}
        value={props.section ? data.sections[props.section] : ''}>
        <settings-header label={header.title} value={header.subtitle} />
    </settings-destination>;
}

function SettingsNavigation() {
    const data = nickel.data;
    return <settings-navigation>
        <SettingsDestination id="display" section="system" />
        <SettingsDestination id="bar" section="personalization" />
        <SettingsDestination id="appearance" />
        <SettingsDestination id="network" section="connectivity" />
        <SettingsDestination id="bluetooth" />
        <SettingsDestination id="bluetooth-pair" />
        <SettingsDestination id="default-apps" />
        <SettingsDestination id="optional-features" />
        <SettingsDestination id="plugins" />
        <SettingsDestination id="keyboard-shortcuts" section="support" />
        <SettingsDestination id="about" />
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

function App() {
    const data = nickel.data || {};
    const wide = (data.width || 1100) >= 720;
    const showNavigation = wide || !data.active;
    const showContent = wide || data.active;
    const request = (type, fields) => nickel.request(Object.assign({type}, fields || {}));
    return <Window id="main" title="Nickel Settings" width="100%" height="100%"
        className="settings-window">
        <div className={wide ? "settings-shell wide" : "settings-shell narrow"}>
            {showNavigation ? <div className={wide ? "settings-sidebar" : "settings-sidebar narrow"}>
                <TextField id="settings-sidebar-search" className="settings-search"
                    value={data.query || ""} placeholder={data.searchPlaceholder || "Search Settings"}
                    onChange={value => request("search", {value})} />
                <ScrollView id="settings-sidebar-scroll" height={Math.max(1, (data.height || 800) - 64)}>
                    <Column className="settings-destinations">
                        {data.query ? (data.results || []).map(result => <Button key={result.target}
                            id={"search-result-" + result.target} disabled={!result.available}
                            className="settings-destination"
                            onClick={() => request("navigate-target", {target: result.target})}>
                            {result.label}
                        </Button>) : (data.destinations || []).map(destination => <Column key={destination.id}>
                            {destination.section ? <Text className="settings-section">{destination.section}</Text> : null}
                            <Button id={"settings-navigation/destination/" + destination.id}
                                state={destination.active ? "selected" : "unselected"}
                                className={destination.active ? "settings-destination active" : "settings-destination"}
                                onClick={() => request("navigate", {page: destination.id})}>
                                {destination.label}
                            </Button>
                        </Column>)}
                        {data.query && !(data.results || []).length ? <Text className="settings-empty">
                            {data.noResults || "No results"}
                        </Text> : null}
                    </Column>
                </ScrollView>
            </div> : null}
            {showContent ? <div className="settings-detail">
                <Row className="settings-heading">
                    {!wide ? <Button id="settings-show-navigation" className="settings-back"
                        onClick={() => request("show-navigation")}>‹</Button> : null}
                    <Column>
                        <Text className="settings-title">{data.title || "Settings"}</Text>
                        {data.subtitle ? <Text className="settings-subtitle" wrap={true}>{data.subtitle}</Text> : null}
                    </Column>
                </Row>
                <Slot id="settings-content" className="settings-content" />
            </div> : null}
        </div>
    </Window>;
}
