// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const openAppMenu = (item, kind, anchor) => {
        setMenuTarget({id: item.id, index: item.index, pinned: item.pinned, kind, anchor});
        nickel.openMenu("launcher-app-actions");
    };
    return <Column>
        <Text>Nickel Launcher</Text>
        <TextField id="launcher-query" value={data.query} placeholder="Search applications"
            onChange={query => nickel.request({type: "launcher-set-query", query})} />
        {data.dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" height={580}>
            <Text>Places</Text>
            {data.places.map(place => <Row>
                <Button id={"launcher-place-" + place.index}
                    icon={"place:" + place.index} showLabel={true}
                    onContextMenu={() => openAppMenu(place, "dashboard", "launcher-place-" + place.index)}
                    onClick={() => nickel.request({type: "launcher-launch-dashboard", id: place.id})}>
                    {place.name}
                </Button>
                <Button id={"launcher-pin-place-" + place.index}
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: place.id})}>
                    {place.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.codexAvailable ? <Column>
                <Text>Recent projects</Text>
                {data.projects.map(project => <Button id={"launcher-project-" + project.id}
                    onClick={() => nickel.request({type: "launcher-open-project", id: project.id})}>
                    {project.name}
                </Button>)}
                <Button id="launcher-all-projects"
                    onClick={() => nickel.request({type: "launcher-see-all-projects"})}>
                    All projects
                </Button>
            </Column> : null}
            <Row>
                <Button id="launcher-view-favorites" onClick={() => nickel.request({type: "launcher-set-view", view: "favorites"})}>
                    Pinned &amp; recent
                </Button>
                <Button id="launcher-view-applications" onClick={() => nickel.request({type: "launcher-set-view", view: "applications"})}>
                    All applications
                </Button>
                <Button id="launcher-view-places" onClick={() => nickel.request({type: "launcher-set-view", view: "places"})}>
                    Places
                </Button>
            </Row>
            <Text>{data.view === "favorites" ? "Pinned and recent" : data.view === "applications" ? "All applications" : "Places"}</Text>
            {data.dashboard.length === 0 ? <Text>No applications in this view</Text> : null}
            {data.dashboard.map(app => <Row>
                <Button id={"launcher-dashboard-" + app.index}
                    icon={"dashboard:" + app.index} showLabel={true}
                    onContextMenu={() => openAppMenu(app, "dashboard", "launcher-dashboard-" + app.index)}
                    onClick={() => nickel.request({type: "launcher-launch-dashboard", id: app.id})}>
                    {app.name}
                </Button>
                <Button id={"launcher-pin-dashboard-" + app.index}
                    accessibilityLabel={(app.pinned ? "Unpin " : "Pin ") + app.name}
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: app.id})}>
                    {app.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.dashboardPageCount > 1 ? <Row>
                {data.dashboardPage > 0 ? <Button id="launcher-dashboard-previous" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage - 1})}>
                    Previous
                </Button> : null}
                <Text>{"Page " + (data.dashboardPage + 1) + " of " + data.dashboardPageCount}</Text>
                {data.dashboardPage + 1 < data.dashboardPageCount ? <Button id="launcher-dashboard-next" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage + 1})}>
                    Next
                </Button> : null}
            </Row> : null}
            <Button id="launcher-account" onClick={() => nickel.request({type: "launcher-open-account"})}>
                {data.accountName}
            </Button>
            <Button id="launcher-settings" onClick={() => nickel.request({type: "launcher-open-settings"})}>
                Settings
            </Button>
            {data.logoutAvailable ? <Button id="launcher-logout" onClick={() => {
                setLogoutOpen(true);
                nickel.openDialog("launcher-logout-dialog");
            }}>Log out</Button> : null}
        </ScrollView> : null}
        {!data.dashboardVisible ?
        <ScrollView id="launcher-search-scroll" height={580}>
            {data.results.length === 0 ? <Text>No applications found</Text> : null}
            {data.results.map(result => <Row>
                <Button id={"launcher-result-" + result.index}
                    icon={"search:" + result.index} showLabel={true}
                    onContextMenu={() => openAppMenu(result, "search", "launcher-result-" + result.index)}
                    onClick={() => nickel.request({type: "launcher-activate-result", index: result.index, id: result.id})}>
                    {result.name}
                </Button>
                <Button id={"launcher-pin-result-" + result.index}
                    accessibilityLabel={(result.pinned ? "Unpin " : "Pin ") + result.name}
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: result.id})}>
                    {result.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.resultPageCount > 1 ? <Row>
                {data.resultPage > 0 ? <Button id="launcher-search-previous" onClick={() => nickel.request({type: "launcher-set-page", view: "search", page: data.resultPage - 1})}>
                    Previous
                </Button> : null}
                <Text>{"Page " + (data.resultPage + 1) + " of " + data.resultPageCount}</Text>
                {data.resultPage + 1 < data.resultPageCount ? <Button id="launcher-search-next" onClick={() => nickel.request({type: "launcher-set-page", view: "search", page: data.resultPage + 1})}>
                    Next
                </Button> : null}
            </Row> : null}
        </ScrollView> : null}
        <Dialog id="launcher-logout-dialog" anchor="launcher-logout" open={logoutOpen} width={320} height={160}>
            <Column>
                <Text>Log out of this session?</Text>
                <Button id="launcher-confirm-logout" accessibilityLabel="Confirm log out" onClick={() => {
                    setLogoutOpen(false);
                    nickel.request({type: "launcher-request-logout"});
                }}>Log out</Button>
                <Button id="launcher-cancel-logout" onClick={() => setLogoutOpen(false)}>Cancel</Button>
            </Column>
        </Dialog>
        {menuTarget ? <Menu id="launcher-app-actions" anchor={menuTarget.anchor} open={true}>
            <MenuItem id="launch" onClick={() => {
                if (menuTarget.kind === "search") {
                    nickel.request({type: "launcher-activate-result", index: menuTarget.index, id: menuTarget.id});
                } else {
                    nickel.request({type: "launcher-launch-dashboard", id: menuTarget.id});
                }
            }}>Launch</MenuItem>
            <MenuItem id="toggle-pin" onClick={() => nickel.request({type: "launcher-toggle-pin", id: menuTarget.id})}>
                {menuTarget.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"}
            </MenuItem>
        </Menu> : null}
    </Column>;
}
