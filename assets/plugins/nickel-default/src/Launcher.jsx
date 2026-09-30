// @jsx h
import "./styles/launcher.css";

const PAGE_SIZE = 12;
function appControlId(prefix, app) { return prefix + (app.icon || app.id); }
function pageItems(items, page) { return items.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE); }

function Pages(props) {
    const count = Math.max(1, Math.ceil(props.total / PAGE_SIZE));
    return count > 1 ? <Row>
        {props.page > 0 ? <Button id={props.id + "-previous"} onClick={() => props.onChange(props.page - 1)}>Previous</Button> : null}
        <Text>{"Page " + (props.page + 1) + " of " + count}</Text>
        {props.page + 1 < count ? <Button id={props.id + "-next"} onClick={() => props.onChange(props.page + 1)}>Next</Button> : null}
    </Row> : null;
}

// Query, tabs, paging, and menus are presentation state in this package.
// Applications retains the authoritative inventory and native fuzzy ranking.
export function Launcher() {
    const applications = nickel.applications.list();
    const search = nickel.applications.searchResults();
    const session = nickel.session?.get() || {account:{displayName:"Local session"},support:{}};
    const [query, setQuery] = useState("");
    const [view, setView] = useState("favorites");
    const [resultPage, setResultPage] = useState(0);
    const [dashboardPage, setDashboardPage] = useState(0);
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const dashboardVisible = !query.length;
    const results = search.query === query ? search.results : [];
    const places = applications.filter(app => app.kind === "place");
    const allApps = applications.filter(app => app.kind !== "place");
    const favoriteApps = applications.filter(app => app.pinned || app.recentOrder !== null && app.recentOrder !== undefined)
        .slice().sort((left, right) => (left.pinOrder ?? Number.MAX_SAFE_INTEGER) - (right.pinOrder ?? Number.MAX_SAFE_INTEGER)
            || (left.recentOrder ?? Number.MAX_SAFE_INTEGER) - (right.recentOrder ?? Number.MAX_SAFE_INTEGER));
    const dashboard = view === "places" ? places : view === "applications" ? allApps : favoriteApps.length ? favoriteApps : allApps;
    const safeDashboardPage = Math.min(dashboardPage, Math.max(0, Math.ceil(dashboard.length / PAGE_SIZE) - 1));
    const safeResultPage = Math.min(resultPage, Math.max(0, Math.ceil(results.length / PAGE_SIZE) - 1));
    const changeQuery = next => {
        const bounded = Array.from(next).slice(0, 512).join("");
        setQuery(bounded);
        setResultPage(0);
        setMenuTarget(null);
        nickel.applications.search(bounded);
    };
    const changeView = next => { setView(next); setDashboardPage(0); setMenuTarget(null); };
    const openAppMenu = (item, anchor) => { setMenuTarget({id:item.id, anchor}); nickel.openMenu("launcher-app-actions"); };
    const menuApplication = menuTarget && [...applications, ...results].find(app => app.id === menuTarget.id);
    const launch = id => { nickel.applications.launch(id); nickel.surfaces.hide("launcher"); };
    const submit = () => { if (!dashboardVisible && results[0]) launch(results[0].id); };

    return <Window id="launcher" title="Nickel Launcher" className="launcher-window"
        onEscape={() => query ? changeQuery("") : nickel.surfaces.hide("launcher")}
        onSubmit={!dashboardVisible && results.length ? submit : undefined}>
      <div className="launcher-content">
        {search.status ? <Text className="launcher-status">{search.status}</Text> : null}
        <TextField id="launcher-query" className="launcher-search" value={query} placeholder="Search applications"
            onChange={changeQuery} />
        {dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" className="launcher-scroll" grow={true}>
            <Text className="launcher-section">Places</Text>
            {places.map(place => <Row key={place.id} className="launcher-app-row">
                <Button id={appControlId("launcher-place-", place)} className="launcher-app-button"
                    icon={place.icon} showLabel={true}
                    onContextMenu={() => openAppMenu(place, appControlId("launcher-place-", place))}
                    onClick={() => launch(place.id)}>{place.name}</Button>
                <Button id={appControlId("launcher-pin-place-", place)} className="launcher-pin-button"
                    onClick={() => nickel.applications.togglePin(place.id)}>{place.pinned ? "Unpin" : "Pin"}</Button>
            </Row>)}
            {search.nativeProjectsAvailable ? <Column className="launcher-projects">
                <Text className="launcher-section">Projects</Text>
                <Button id="launcher-all-projects" className="launcher-footer-button"
                    onClick={() => nickel.request({type:"launcher-see-all-projects"})}>Open native project overview</Button>
            </Column> : null}
            <Row className="launcher-tabs">
                <Button id="launcher-view-favorites" className={view === "favorites" ? "launcher-tab selected" : "launcher-tab"} onClick={() => changeView("favorites")}>Pinned &amp; recent</Button>
                <Button id="launcher-view-applications" className={view === "applications" ? "launcher-tab selected" : "launcher-tab"} onClick={() => changeView("applications")}>All applications</Button>
                <Button id="launcher-view-places" className={view === "places" ? "launcher-tab selected" : "launcher-tab"} onClick={() => changeView("places")}>Places</Button>
            </Row>
            <Text className="launcher-section">{view === "favorites" ? "Pinned and recent" : view === "applications" ? "All applications" : "Places"}</Text>
            {search.catalogTruncated ? <Text wrap={true}>Search to find applications beyond this bounded catalog.</Text> : null}
            {!dashboard.length ? <Text>No applications in this view</Text> : null}
            <div className="launcher-app-grid">
                {pageItems(dashboard, safeDashboardPage).map(app => <div key={app.id} className="launcher-app-card">
                    <Button id={appControlId("launcher-dashboard-", app)} className="launcher-icon-button"
                        icon={app.icon} accessibilityLabel={app.name}
                        onContextMenu={() => openAppMenu(app, appControlId("launcher-dashboard-", app))}
                        onClick={() => launch(app.id)}>{app.name.charAt(0).toUpperCase()}</Button>
                    <Text className="launcher-app-name" wrap={true}>{app.name}</Text>
                </div>)}
            </div>
            <Pages id="launcher-dashboard" page={safeDashboardPage} total={dashboard.length} onChange={setDashboardPage} />
        </ScrollView> : <ScrollView id="launcher-search-scroll" className="launcher-scroll" grow={true}>
            {!search.available ? <Text>{search.reason || "Application search is unavailable."}</Text> : search.query !== query ? <Text>Searching…</Text> : !results.length ? <Text>No applications found</Text> : null}
            {pageItems(results, safeResultPage).map(result => <Row key={result.id} className="launcher-app-row">
                <Button id={appControlId("launcher-result-", result)} className="launcher-app-button" icon={result.icon} showLabel={true}
                    onContextMenu={() => openAppMenu(result, appControlId("launcher-result-", result))}
                    onClick={() => launch(result.id)}>{result.name}</Button>
                <Button id={appControlId("launcher-pin-result-", result)} className="launcher-pin-button"
                    accessibilityLabel={(result.pinned ? "Unpin " : "Pin ") + result.name}
                    onClick={() => nickel.applications.togglePin(result.id)}>{result.pinned ? "Unpin" : "Pin"}</Button>
            </Row>)}
            {search.truncated ? <Text wrap={true}>More matches are available. Refine your search to narrow the results.</Text> : null}
            <Pages id="launcher-search" page={safeResultPage} total={results.length} onChange={setResultPage} />
        </ScrollView>}
        <Row className="launcher-footer">
            <Button id="launcher-account" className="launcher-account-button" onClick={() => nickel.surfaces.show("quick-settings")}>{session.account.displayName || "Local session"}</Button>
            {session.support.logout ? <Button id="launcher-logout" className="launcher-footer-button" onClick={() => {setLogoutOpen(true); nickel.openDialog("launcher-logout-dialog");}}>Log out</Button> : null}
            <Button id="launcher-settings" className="launcher-footer-button" onClick={() => nickel.surfaces.show("settings")}>Settings</Button>
        </Row>
        <Dialog id="launcher-logout-dialog" anchor="launcher-logout" open={logoutOpen} onClose={() => setLogoutOpen(false)} width={320} height={160}>
            <Column>
                <Text>Log out of this session?</Text>
                <Button id="launcher-confirm-logout" accessibilityLabel="Confirm log out" onClick={() => {setLogoutOpen(false); nickel.session.logout();}}>Log out</Button>
                <Button id="launcher-cancel-logout" onClick={() => setLogoutOpen(false)}>Cancel</Button>
            </Column>
        </Dialog>
        {menuApplication ? <Menu id="launcher-app-actions" anchor={menuTarget.anchor} open={true}>
            <MenuItem id="launch" onClick={() => {launch(menuApplication.id); setMenuTarget(null);}}>Launch</MenuItem>
            <MenuItem id="toggle-pin" onClick={() => nickel.applications.togglePin(menuApplication.id)}>{menuApplication.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"}</MenuItem>
            {search.pinSaveFailed ? <MenuItem id="retry-pin-save" onClick={() => nickel.applications.retryPinSave()}>Retry saving favorites</MenuItem> : null}
        </Menu> : null}
      </div>
    </Window>;
}
