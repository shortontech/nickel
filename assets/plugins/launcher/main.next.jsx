// Architecture draft. The shipped main.jsx/main.js still drive the launcher.
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const openAppMenu = (item, kind, anchor) => {
        setMenuTarget({id: item.id, index: item.index, pinned: item.pinned, kind, anchor});
        nickel.openMenu("launcher-app-actions");
    };
    return <FixedWindow id="main" className="launcher-window" output="active"
        anchor="bottom-start" avoid="taskbar" width={920} height={680}
        maxWidth="output" maxHeight="work-area">
        <Column className="launcher-layout">
        <Text className="launcher-title">Nickel Launcher</Text>
        {data.status ? <Text className="status-message">{data.status}</Text> : null}
        <TextField id="launcher-query" className="search-field" value={data.query} placeholder="Search applications"
            onChange={query => nickel.request({type: "launcher-set-query", query})} />
        {data.dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" className="launcher-content" grow={true}>
            <Text className="section-heading">Places</Text>
            {data.places.map(place => <Row key={place.id} className="application-row">
                <Button id={"launcher-place-" + place.index}
                    className="application-button" icon={"place:" + place.index} showLabel={true}
                    onContextMenu={() => openAppMenu(place, "dashboard", "launcher-place-" + place.index)}
                    onClick={() => nickel.request({type: "launcher-launch-dashboard", id: place.id})}>
                    {place.name}
                </Button>
                <Button id={"launcher-pin-place-" + place.index} className="pin-button"
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: place.id})}>
                    {place.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.codexAvailable ? <Column className="projects">
                <Text className="section-heading">Recent projects</Text>
                {data.projects.map(project => <Button id={"launcher-project-" + project.id}
                    key={project.id} className="project-button"
                    onClick={() => nickel.request({type: "launcher-open-project", id: project.id})}>
                    {project.name}
                </Button>)}
                <Button id="launcher-all-projects" className="secondary-button"
                    onClick={() => nickel.request({type: "launcher-see-all-projects"})}>
                    All projects
                </Button>
            </Column> : null}
            <Row className="view-tabs">
                <Button id="launcher-view-favorites" className="view-tab" onClick={() => nickel.request({type: "launcher-set-view", view: "favorites"})}>
                    Pinned &amp; recent
                </Button>
                <Button id="launcher-view-applications" className="view-tab" onClick={() => nickel.request({type: "launcher-set-view", view: "applications"})}>
                    All applications
                </Button>
                <Button id="launcher-view-places" className="view-tab" onClick={() => nickel.request({type: "launcher-set-view", view: "places"})}>
                    Places
                </Button>
            </Row>
            <Text className="section-heading">{data.view === "favorites" ? "Pinned and recent" : data.view === "applications" ? "All applications" : "Places"}</Text>
            {data.dashboard.length === 0 ? <Text className="empty-message">No applications in this view</Text> : null}
            {data.dashboard.map(app => <Row key={app.id} className="application-row">
                <Button id={"launcher-dashboard-" + app.index}
                    className="application-button" icon={"dashboard:" + app.index} showLabel={true}
                    onContextMenu={() => openAppMenu(app, "dashboard", "launcher-dashboard-" + app.index)}
                    onClick={() => nickel.request({type: "launcher-launch-dashboard", id: app.id})}>
                    {app.name}
                </Button>
                <Button id={"launcher-pin-dashboard-" + app.index} className="pin-button"
                    accessibilityLabel={(app.pinned ? "Unpin " : "Pin ") + app.name}
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: app.id})}>
                    {app.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.dashboardPageCount > 1 ? <Row className="pager">
                {data.dashboardPage > 0 ? <Button id="launcher-dashboard-previous" className="secondary-button" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage - 1})}>
                    Previous
                </Button> : null}
                <Text className="page-label">{"Page " + (data.dashboardPage + 1) + " of " + data.dashboardPageCount}</Text>
                {data.dashboardPage + 1 < data.dashboardPageCount ? <Button id="launcher-dashboard-next" className="secondary-button" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage + 1})}>
                    Next
                </Button> : null}
            </Row> : null}
            <Button id="launcher-account" className="footer-button" onClick={() => nickel.request({type: "launcher-open-account"})}>
                {data.accountName}
            </Button>
            <Button id="launcher-settings" className="footer-button" onClick={() => nickel.request({type: "launcher-open-settings"})}>
                Settings
            </Button>
            {data.logoutAvailable ? <Button id="launcher-logout" className="footer-button" onClick={() => {
                setLogoutOpen(true);
                nickel.openDialog("launcher-logout-dialog");
            }}>Log out</Button> : null}
        </ScrollView> : null}
        {!data.dashboardVisible ?
        <ScrollView id="launcher-search-scroll" className="launcher-content" grow={true}>
            {data.results.length === 0 ? <Text className="empty-message">No applications found</Text> : null}
            {data.results.map(result => <Row key={result.id} className="application-row">
                <Button id={"launcher-result-" + result.index}
                    className="application-button" icon={"search:" + result.index} showLabel={true}
                    onContextMenu={() => openAppMenu(result, "search", "launcher-result-" + result.index)}
                    onClick={() => nickel.request({type: "launcher-activate-result", index: result.index, id: result.id})}>
                    {result.name}
                </Button>
                <Button id={"launcher-pin-result-" + result.index} className="pin-button"
                    accessibilityLabel={(result.pinned ? "Unpin " : "Pin ") + result.name}
                    onClick={() => nickel.request({type: "launcher-toggle-pin", id: result.id})}>
                    {result.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.resultPageCount > 1 ? <Row className="pager">
                {data.resultPage > 0 ? <Button id="launcher-search-previous" className="secondary-button" onClick={() => nickel.request({type: "launcher-set-page", view: "search", page: data.resultPage - 1})}>
                    Previous
                </Button> : null}
                <Text className="page-label">{"Page " + (data.resultPage + 1) + " of " + data.resultPageCount}</Text>
                {data.resultPage + 1 < data.resultPageCount ? <Button id="launcher-search-next" className="secondary-button" onClick={() => nickel.request({type: "launcher-set-page", view: "search", page: data.resultPage + 1})}>
                    Next
                </Button> : null}
            </Row> : null}
        </ScrollView> : null}
        <Dialog id="launcher-logout-dialog" className="logout-dialog" anchor="launcher-logout" open={logoutOpen} onClose={() => setLogoutOpen(false)} width={320} height={160}>
            <Column className="dialog-content">
                <Text>Log out of this session?</Text>
                <Button id="launcher-confirm-logout" className="danger-button" accessibilityLabel="Confirm log out" onClick={() => {
                    setLogoutOpen(false);
                    nickel.request({type: "launcher-request-logout"});
                }}>Log out</Button>
                <Button id="launcher-cancel-logout" className="secondary-button" onClick={() => setLogoutOpen(false)}>Cancel</Button>
            </Column>
        </Dialog>
        {menuTarget ? <Menu id="launcher-app-actions" className="app-menu" anchor={menuTarget.anchor} open={true}>
            <MenuItem id="launch" className="menu-item" onClick={() => {
                if (menuTarget.kind === "search") {
                    nickel.request({type: "launcher-activate-result", index: menuTarget.index, id: menuTarget.id});
                } else {
                    nickel.request({type: "launcher-launch-dashboard", id: menuTarget.id});
                }
            }}>Launch</MenuItem>
            <MenuItem id="toggle-pin" className="menu-item" onClick={() => nickel.request({type: "launcher-toggle-pin", id: menuTarget.id})}>
                {menuTarget.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"}
            </MenuItem>
        </Menu> : null}
        </Column>
    </FixedWindow>;
}
