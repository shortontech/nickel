// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    const [logoutOpen, setLogoutOpen] = useState(false);
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
            <Text>Pinned and recent</Text>
            {data.dashboard.map(app => <Button id={"launcher-dashboard-" + app.index}
                onClick={() => nickel.request({type: "launcher-launch-dashboard", id: app.id})}>
                {app.name}
            </Button>)}
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
            {data.results.map(result => <Button id={"launcher-result-" + result.index}
                onClick={() => nickel.request({type: "launcher-activate-result", index: result.index, id: result.id})}>
                {result.name}
            </Button>)}
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
    </Column>;
}
