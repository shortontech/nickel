// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    return <Column>
        <Text>Nickel Launcher</Text>
        <TextField id="launcher-query" value={data.query} placeholder="Search applications"
            onChange={query => nickel.request({type: "launcher-set-query", query})} />
        {data.dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" height={580}>
            <Text>Places</Text>
            {data.places.map(place => <Button id={"launcher-place-" + place.index}
                onClick={() => nickel.request({type: "launcher-launch-dashboard", id: place.id})}>
                {place.name}
            </Button>)}
            <Text>Pinned and recent</Text>
            {data.dashboard.map(app => <Button id={"launcher-dashboard-" + app.index}
                onClick={() => nickel.request({type: "launcher-launch-dashboard", id: app.id})}>
                {app.name}
            </Button>)}
        </ScrollView> : null}
        {!data.dashboardVisible ?
        <ScrollView id="launcher-search-scroll" height={580}>
            {data.results.length === 0 ? <Text>No applications found</Text> : null}
            {data.results.map(result => <Button id={"launcher-result-" + result.index}
                onClick={() => nickel.request({type: "launcher-activate-result", index: result.index, id: result.id})}>
                {result.name}
            </Button>)}
        </ScrollView> : null}
    </Column>;
}
