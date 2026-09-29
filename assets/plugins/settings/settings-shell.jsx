// @jsx h
// The native host owns Settings values and validates navigation requests.
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
