// @jsx h
import "./styles/launcher.css";
// The host owns search ranking and launches. This plugin owns the view.
export function Launcher() {
    const data = nickel.data;
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const openAppMenu = (item, kind, anchor) => {
        setMenuTarget({id: item.id, index: item.index, pinned: item.pinned, kind, anchor});
        nickel.openMenu("launcher-app-actions");
    };
    const escape = () => data.query
        ? nickel.request({type: "launcher-set-query", query: ""})
        : nickel.request({type: "dismiss-launcher"});
    const submit = () => {
        const first = !data.dashboardVisible && data.results[0];
        if (first) nickel.request({type: "launcher-activate-result", index: first.index, id: first.id});
    };
    return <Window id="launcher" title="Nickel Launcher" className="launcher-window"
        onEscape={escape} onSubmit={!data.dashboardVisible && data.results.length ? submit : undefined}>
      <div className="launcher-content">
        {data.status ? <Text className="launcher-status">{data.status}</Text> : null}
        <TextField id="launcher-query" className="launcher-search" value={data.query} placeholder="Search applications"
            onChange={query => nickel.request({type: "launcher-set-query", query})} />
        {data.dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" className="launcher-scroll" grow={true}>
            <Text className="launcher-section">Places</Text>
            {data.places.map(place => <Row key={place.id} className="launcher-app-row">
                <Button id={"launcher-place-" + place.index}
                    className="launcher-app-button"
                    icon={"place:" + place.index} showLabel={true}
                    onContextMenu={() => openAppMenu(place, "dashboard", "launcher-place-" + place.index)}
                    onClick={() => nickel.request({type: "applications.launch", id: place.id})}>
                    {place.name}
                </Button>
                <Button id={"launcher-pin-place-" + place.index} className="launcher-pin-button"
                    onClick={() => nickel.request({type: "applications.togglePin", id: place.id})}>
                    {place.pinned ? "Unpin" : "Pin"}
                </Button>
            </Row>)}
            {data.codexAvailable ? <Column className="launcher-projects">
                <Text className="launcher-section">Recent projects</Text>
                {data.projects.map(project => <Button key={project.id} id={"launcher-project-" + project.id}
                    className="launcher-app-button"
                    onClick={() => nickel.request({type: "launcher-open-project", id: project.id})}>
                    {project.name}
                </Button>)}
                <Button id="launcher-all-projects" className="launcher-footer-button"
                    onClick={() => nickel.request({type: "launcher-see-all-projects"})}>
                    All projects
                </Button>
            </Column> : null}
            <Row className="launcher-tabs">
                <Button id="launcher-view-favorites" className={data.view === 'favorites' ? 'launcher-tab selected' : 'launcher-tab'} onClick={() => nickel.request({type: "launcher-set-view", view: "favorites"})}>
                    Pinned &amp; recent
                </Button>
                <Button id="launcher-view-applications" className={data.view === 'applications' ? 'launcher-tab selected' : 'launcher-tab'} onClick={() => nickel.request({type: "launcher-set-view", view: "applications"})}>
                    All applications
                </Button>
                <Button id="launcher-view-places" className={data.view === 'places' ? 'launcher-tab selected' : 'launcher-tab'} onClick={() => nickel.request({type: "launcher-set-view", view: "places"})}>
                    Places
                </Button>
            </Row>
            <Text className="launcher-section">{data.view === "favorites" ? "Pinned and recent" : data.view === "applications" ? "All applications" : "Places"}</Text>
            {data.dashboard.length === 0 ? <Text>No applications in this view</Text> : null}
            <div className="launcher-app-grid">
            {data.dashboard.map(app => <div key={app.id} className="launcher-app-card">
                <Button id={"launcher-dashboard-" + app.index}
                    className="launcher-icon-button"
                    icon={"dashboard:" + app.index} accessibilityLabel={app.name}
                    onContextMenu={() => openAppMenu(app, "dashboard", "launcher-dashboard-" + app.index)}
                    onClick={() => nickel.request({type: "applications.launch", id: app.id})}>
                    {app.name.charAt(0).toUpperCase()}
                </Button>
                <Text className="launcher-app-name" wrap={true}>{app.name}</Text>
            </div>)}
            </div>
            {data.dashboardPageCount > 1 ? <Row>
                {data.dashboardPage > 0 ? <Button id="launcher-dashboard-previous" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage - 1})}>
                    Previous
                </Button> : null}
                <Text>{"Page " + (data.dashboardPage + 1) + " of " + data.dashboardPageCount}</Text>
                {data.dashboardPage + 1 < data.dashboardPageCount ? <Button id="launcher-dashboard-next" onClick={() => nickel.request({type: "launcher-set-page", view: "dashboard", page: data.dashboardPage + 1})}>
                    Next
                </Button> : null}
            </Row> : null}
        </ScrollView> : null}
        {!data.dashboardVisible ?
        <ScrollView id="launcher-search-scroll" className="launcher-scroll" grow={true}>
            {data.results.length === 0 ? <Text>No applications found</Text> : null}
            {data.results.map(result => <Row key={result.id} className="launcher-app-row">
                <Button id={"launcher-result-" + result.index}
                    className="launcher-app-button"
                    icon={"search:" + result.index} showLabel={true}
                    onContextMenu={() => openAppMenu(result, "search", "launcher-result-" + result.index)}
                    onClick={() => nickel.request({type: "launcher-activate-result", index: result.index, id: result.id})}>
                    {result.name}
                </Button>
                <Button id={"launcher-pin-result-" + result.index} className="launcher-pin-button"
                    accessibilityLabel={(result.pinned ? "Unpin " : "Pin ") + result.name}
                    onClick={() => nickel.request({type: "applications.togglePin", id: result.id})}>
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
        <Row className="launcher-footer">
            <Button id="launcher-account" className="launcher-account-button" onClick={() => nickel.request({type: "show-control-center"})}>
                {data.accountName}
            </Button>
            {data.logoutAvailable ? <Button id="launcher-logout" className="launcher-footer-button" onClick={() => {
                setLogoutOpen(true);
                nickel.openDialog("launcher-logout-dialog");
            }}>Log out</Button> : null}
            <Button id="launcher-settings" className="launcher-footer-button" onClick={() => nickel.request({type: "show-settings", screen: "appearance"})}>
                Settings
            </Button>
        </Row>
        <Dialog id="launcher-logout-dialog" anchor="launcher-logout" open={logoutOpen} onClose={() => setLogoutOpen(false)} width={320} height={160}>
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
                    nickel.request({type: "applications.launch", id: menuTarget.id});
                }
            }}>Launch</MenuItem>
            <MenuItem id="toggle-pin" onClick={() => nickel.request({type: "applications.togglePin", id: menuTarget.id})}>
                {menuTarget.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"}
            </MenuItem>
            {data.pinSaveFailed ? <MenuItem id="retry-pin-save" onClick={() => nickel.request({type: "applications-retry-pin-save"})}>
                Retry saving favorites
            </MenuItem> : null}
        </Menu> : null}
      </div>
    </Window>;
}
