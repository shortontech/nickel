// @jsx h
import "./styles/launcher.css";

const PAGE_SIZE = 12;
const CATEGORIES = [
    {id:"all", label:"All"}, {id:"application", label:"Applications"},
    {id:"setting", label:"Settings"}, {id:"place", label:"Files"}, {id:"action", label:"Actions"}
];
function appControlId(prefix, app) { return prefix + (app.icon || app.id); }
function pageItems(items, page) { return items.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE); }
function kindLabel(item) {
    return item.kind === "place" ? "Folder" : item.kind === "setting" ? "Setting" : item.kind === "action" ? "Action" : "Application";
}
function lastUsed(item, now) {
    if (!Number.isFinite(item.lastUsedUnixSeconds)) return "";
    const minutes = Math.max(0, Math.floor((now - item.lastUsedUnixSeconds) / 60));
    return minutes < 1 ? "Just now" : minutes < 60 ? minutes + "m ago"
        : minutes < 1440 ? Math.floor(minutes / 60) + "h ago" : Math.floor(minutes / 1440) + "d ago";
}
function Pages(props) {
    const count = Math.max(1, Math.ceil(props.total / PAGE_SIZE));
    return count > 1 ? <Row className="launcher-pages">
        {props.page > 0 ? <Button id={props.id + "-previous"} onClick={() => props.onChange(props.page - 1)}>Previous</Button> : null}
        <Text>{"Page " + (props.page + 1) + " of " + count}</Text>
        {props.page + 1 < count ? <Button id={props.id + "-next"} onClick={() => props.onChange(props.page + 1)}>Next</Button> : null}
    </Row> : null;
}

export function Launcher() {
    const availableSize = useSurface(surface => surface.availableSize);
    const applications = nickel.applications.list().filter(app => app.canLaunch !== false);
    const search = nickel.applications.searchResults();
    const session = nickel.session?.get() || {account:{displayName:"Local session"},support:{}};
    const [query, setQuery] = useState("");
    const [queryFocused, setQueryFocused] = useState(true);
    const [view, setView] = useState("favorites");
    const [category, setCategory] = useState("all");
    const [selectedId, setSelectedId] = useState(null);
    const [detailOpen, setDetailOpen] = useState(false);
    const [resultPage, setResultPage] = useState(0);
    const [dashboardPage, setDashboardPage] = useState(0);
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const dashboardVisible = !query.length;
    const results = search.query === query ? [...search.results, ...(search.settingsResults || []), ...(search.actionResults || [])] : [];
    const filtered = category === "all" ? results : results.filter(item => (item.kind || "application") === category);
    const safeResultPage = Math.min(resultPage, Math.max(0, Math.ceil(filtered.length / PAGE_SIZE) - 1));
    const visibleResults = pageItems(filtered, safeResultPage);
    const selected = visibleResults.find(item => item.id === selectedId) || visibleResults[0];
    const places = applications.filter(app => app.kind === "place");
    const allApps = applications.filter(app => app.kind !== "place").slice().sort((a, b) => a.name.localeCompare(b.name));
    const pinned = applications.filter(app => app.pinned).slice().sort((a,b) => a.pinOrder - b.pinOrder);
    const recent = applications.filter(app => app.recentOrder !== null && app.recentOrder !== undefined)
        .slice().sort((a,b) => a.recentOrder - b.recentOrder);
    const dashboard = view === "places" ? places : view === "applications" ? allApps : view === "recent" ? recent : pinned;
    const safeDashboardPage = Math.min(dashboardPage, Math.max(0, Math.ceil(dashboard.length / PAGE_SIZE) - 1));
    const now = search.nowUnixSeconds || Math.floor(Date.now() / 1000);
    const viewport = availableSize || {};
    const windowWidth = Math.min(608,
        Number.isFinite(viewport.width) ? Math.max(1, viewport.width - 16) : 608);
    const windowHeight = Math.min(628,
        Number.isFinite(viewport.height) ? Math.max(1, viewport.height - 16) : 628);
    const compact = windowWidth < 550;
    const short = windowHeight < 420;
    const changeQuery = next => {
        const bounded = Array.from(next).slice(0, 512).join("");
        setQuery(bounded); setResultPage(0); setSelectedId(null); setDetailOpen(false); setMenuTarget(null);
        nickel.applications.search(bounded);
    };
    const changeView = next => { setView(next); setDashboardPage(0); setMenuTarget(null); };
    const changeCategory = next => { setCategory(next); setResultPage(0); setSelectedId(null); setDetailOpen(false); };
    const openAppMenu = (item, anchor) => { setMenuTarget({id:item.id, anchor}); nickel.openMenu("launcher-app-actions"); };
    const menuApplication = menuTarget && [...applications, ...results].find(app => app.id === menuTarget.id);
    const activate = item => {
        if (item.kind === "setting") nickel.request({type:"show-settings", screen:item.destination});
        else if (item.kind === "action") {
            if (item.destination === "projects") nickel.projects.show();
            else nickel.surfaces.show(item.destination);
        } else nickel.applications.launch(item.id);
        nickel.surfaces.hide("launcher");
    };
    const canPin = item => item && (!item.kind || item.kind === "application" || item.kind === "place") && item.canPin !== false;
    const recentRow = item => <Row key={item.id} className={item.kind === "place" && item.path ? "launcher-recent-row with-path" : "launcher-recent-row"}>
        <Column className="launcher-recent-identity">
            <Button id={appControlId("launcher-recent-", item)} className="launcher-recent-button" icon={item.icon} iconSize={28} showLabel={true}
                description={item.kind === "place" ? item.path : undefined}
                onContextMenu={() => openAppMenu(item, appControlId("launcher-recent-", item))}
                onClick={() => activate(item)}>{item.name}</Button>
        </Column>
        <Text className="launcher-time">{lastUsed(item, now)}</Text>
    </Row>;

    return <FixedWindow id="launcher" title="Nickel Launcher" className={(dashboardVisible ? "launcher-window dashboard" : "launcher-window search") + (compact ? " compact" : "") + (short ? " short" : "")}
        width={windowWidth} height={windowHeight} anchor="bottom-left"
        onEscape={() => detailOpen ? setDetailOpen(false) : query ? changeQuery("") : view !== "favorites" ? changeView("favorites") : nickel.surfaces.hide("launcher")}
        onSubmit={queryFocused && !dashboardVisible && selected ? () => activate(selected) : undefined}>
      <div className="launcher-content">
        <div className="launcher-search-header">
            <TextField autoFocus={true} id="launcher-query" className="launcher-search" value={query}
                accessibilityLabel="Search apps, files, settings, or commands"
                placeholder="Search apps, files, settings, or commands…" onChange={changeQuery}
                onFocus={() => setQueryFocused(true)} onBlur={() => setQueryFocused(false)} />
        </div>
        {search.status ? <Text className="launcher-status">{search.status}</Text> : null}
        {dashboardVisible ? <ScrollView id="launcher-dashboard-scroll" className="launcher-dashboard-scroll" grow={true}>
            <Row className="launcher-section-heading">
                <Text className="launcher-section">{view === "favorites" ? "Pinned" : view === "applications" ? "All apps" : view === "places" ? "Places" : "Recent"}</Text>
                <Button id={view === "favorites" ? "launcher-view-applications" : "launcher-view-favorites"} className="launcher-all-apps"
                    onClick={() => changeView(view === "favorites" ? "applications" : "favorites")}>{view === "favorites" ? "All apps  ›" : "‹  Back"}</Button>
            </Row>
            {search.catalogTruncated && view === "applications" ? <Text wrap={true}>Search to find applications beyond this catalog.</Text> : null}
            {view === "recent" || view === "places" ? pageItems(dashboard, safeDashboardPage).map(recentRow)
                : view === "applications" ? <Column className="launcher-catalog">
                    {pageItems(dashboard, safeDashboardPage).map(app => <Button key={app.id}
                        id={appControlId("launcher-dashboard-", app)} className="launcher-catalog-button"
                        icon={app.icon} iconSize={28} showLabel={true}
                        onContextMenu={() => openAppMenu(app, appControlId("launcher-dashboard-", app))}
                        onClick={() => activate(app)}>{app.name}</Button>)}
                  </Column> : <div className="launcher-app-grid">
                {pageItems(dashboard, safeDashboardPage).map(app => <Button key={app.id}
                    id={appControlId("launcher-dashboard-", app)} className="launcher-pin-tile"
                    icon={app.icon} iconSize={32} iconPlacement="top" showLabel={true} accessibilityLabel={app.name}
                    onContextMenu={() => openAppMenu(app, appControlId("launcher-dashboard-", app))}
                    onClick={() => activate(app)}>{app.name}</Button>)}
                {view === "favorites" && pinned.length < PAGE_SIZE ? <div key="add" className="launcher-app-card">
                    <Button id="launcher-add-pin" className="launcher-add-button" accessibilityLabel="Add pinned application" onClick={() => changeView("applications")}>+</Button>
                    <Text className="launcher-app-name">Add</Text>
                </div> : null}
            </div>}
            {!pinned.length && view === "favorites" ? <Text className="launcher-empty" wrap={true}>Keep your favorites here. Right-click an app in All apps to pin it.</Text> : null}
            {!dashboard.length && view !== "favorites" ? <Text className="launcher-empty">No items in this view</Text> : null}
            <Pages id="launcher-dashboard" page={safeDashboardPage} total={dashboard.length} onChange={setDashboardPage} />
            {view === "favorites" ? <Column className="launcher-recents">
                <Row className="launcher-section-heading">
                    <Text className="launcher-section">Recent</Text>
                    <Button id="launcher-view-recent" className="launcher-link" onClick={() => changeView("recent")}>More  ›</Button>
                </Row>
                {recent.slice(0,5).map(recentRow)}
                {!recent.length ? <Text className="launcher-empty" wrap={true}>Recently opened applications will appear here.</Text> : null}
            </Column> : null}
            {view === "places" && search.nativeProjectsAvailable ? <Button id="launcher-all-projects" className="launcher-link" onClick={() => nickel.projects.show()}>All projects  ›</Button> : null}
        </ScrollView> : <div className="launcher-search-body">
            {!detailOpen ? <div className="launcher-categories">
                {CATEGORIES.map(item => <Button key={item.id} id={"launcher-category-" + item.id}
                    className={category === item.id ? "launcher-category selected" : "launcher-category"}
                    state={category === item.id ? "selected" : "unselected"}
                    onClick={() => changeCategory(item.id)}>{compact && item.id === "application" ? "Apps" : item.label}</Button>)}
            </div> : null}
            {!detailOpen ? <ScrollView id="launcher-search-scroll" className="launcher-results" grow={true}>
                {!search.available ? <Text wrap={true}>{search.reason || "Application search is unavailable."}</Text> : search.query !== query ? <Text>Searching…</Text> : !filtered.length ? <Text>No results found</Text> : null}
                {CATEGORIES.filter(group => group.id !== "all").map(group => {
                    const items = visibleResults.filter(item => (item.kind || "application") === group.id);
                    return items.length ? <Column key={group.id} className="launcher-result-group">
                        <Text className="launcher-group-title">{group.label}</Text>
                        {items.map(result => <Button key={result.id} id={appControlId("launcher-result-", result)}
                                className={selected?.id === result.id ? "launcher-result-button selected" : "launcher-result-button"}
                                icon={result.icon} iconSize={24} showLabel={true} description={result.description || kindLabel(result)}
                                state={selected?.id === result.id ? "selected" : "unselected"}
                                onFocus={() => setSelectedId(result.id)}
                                onContextMenu={canPin(result) ? () => openAppMenu(result, appControlId("launcher-result-", result)) : undefined}
                                onClick={() => {setSelectedId(result.id); activate(result);}}>{result.name}</Button>)}
                    </Column> : null;
                })}
                {search.truncated ? <Text wrap={true}>More matches are available. Refine your search.</Text> : null}
                <Pages id="launcher-search" page={safeResultPage} total={filtered.length} onChange={page => {setResultPage(page); setSelectedId(null);}} />
            </ScrollView> : null}
            {selected && detailOpen ? <ScrollView id="launcher-detail-scroll" className="launcher-detail" grow={true}>
                <Button id="launcher-back-results" className="launcher-link" onClick={() => setDetailOpen(false)}>‹  Results</Button>
                <Row className="launcher-detail-heading">
                    {selected.icon ? <Image asset={selected.icon} width={64} height={64} accessibilityLabel={selected.name} /> : null}
                    <Column className="launcher-detail-identity">
                        <Text className="launcher-detail-title" wrap={true}>{selected.name}</Text>
                        <Text className="launcher-detail-type">{kindLabel(selected)}</Text>
                    </Column>
                </Row>
                {!short && selected.description ? <Text className="launcher-detail-description" wrap={true}>{selected.description}</Text> : null}
                <Button id="launcher-open-selected" className="launcher-primary" onClick={() => activate(selected)}>Open</Button>
                {short && selected.description ? <Text className="launcher-detail-description" wrap={true}>{selected.description}</Text> : null}
                {canPin(selected) ? <Button id="launcher-pin-selected" className="launcher-detail-action" onClick={() => nickel.applications.togglePin(selected.id)}>{selected.pinned ? "Unpin from launcher" : "Pin to launcher"}</Button> : null}
                <div className="launcher-horizontal-divider" />
                <Column className="launcher-metadata">
                    {selected.path ? <Text wrap={true}>{"Path   " + selected.path}</Text> : null}
                    <Text>{"Type   " + kindLabel(selected)}</Text>
                    {lastUsed(selected, now) ? <Text>{"Last used   " + lastUsed(selected, now)}</Text> : null}
                </Column>
            </ScrollView> : null}
        </div>}
        {!dashboardVisible && selected && !detailOpen ? <Row className="launcher-selection-actions">
            <Text className="launcher-selection-name">{selected.name}</Text>
            <Button id="launcher-open-selected" className="launcher-link" onClick={() => activate(selected)}>Open</Button>
            {canPin(selected) ? <Button id="launcher-pin-selected" className="launcher-link"
                accessibilityLabel={selected.pinned ? "Unpin from launcher" : "Pin to launcher"}
                onClick={() => nickel.applications.togglePin(selected.id)}>{selected.pinned ? "Unpin" : "Pin"}</Button> : null}
            <Button id="launcher-more-selected" className="launcher-link" onClick={() => setDetailOpen(true)}>More…</Button>
        </Row> : null}
        {dashboardVisible || recent.length ? <div className="launcher-horizontal-divider" /> : null}
        {!dashboardVisible ? recent.length ? <Row className="launcher-recent-strip">
            <Text className="launcher-strip-label">Recent</Text>
            {recent.slice(0,compact ? 2 : 4).map(item => <Column key={item.id} className="launcher-shortcut-identity">
                <Button id={appControlId("launcher-shortcut-", item)} className="launcher-recent-shortcut" icon={item.icon} iconSize={24} showLabel={true} onClick={() => activate(item)}>{item.name}</Button>
                {!short && lastUsed(item, now) ? <Text className="launcher-shortcut-time">{lastUsed(item, now)}</Text> : null}
            </Column>)}
            {!recent.length ? <Text className="launcher-empty">No recent applications</Text> : null}
        </Row> : null : <Row className="launcher-footer">
            <Button id="launcher-account" className="launcher-account-button" onClick={() => nickel.surfaces.show("quick-settings")}>{session.account?.displayName || "Local session"}</Button>
            <Button id="launcher-view-places" className="launcher-link" onClick={() => changeView("places")}>Places</Button>
            {session.support.logout ? <Button id="launcher-logout" className="launcher-link" onClick={() => {setLogoutOpen(true); nickel.openDialog("launcher-logout-dialog");}}>Log out</Button> : null}
            <Button id="launcher-settings" className="launcher-link" onClick={() => nickel.surfaces.show("settings")}>Settings</Button>
        </Row>}
        <Dialog id="launcher-logout-dialog" anchor="launcher-logout" open={logoutOpen} onClose={() => setLogoutOpen(false)} width={320} height={160}>
            <Column>
                <Text>Log out of this session?</Text>
                <Button id="launcher-confirm-logout" accessibilityLabel="Confirm log out" onClick={() => {setLogoutOpen(false); nickel.session.logout();}}>Log out</Button>
                <Button id="launcher-cancel-logout" onClick={() => setLogoutOpen(false)}>Cancel</Button>
            </Column>
        </Dialog>
        {menuApplication ? <Menu id="launcher-app-actions" className="launcher-app-menu" anchor={menuTarget.anchor} open={true}>
            <MenuItem id="launch" onClick={() => {activate(menuApplication); setMenuTarget(null);}}>Open</MenuItem>
            <MenuItem id="toggle-pin" onClick={() => nickel.applications.togglePin(menuApplication.id)}>{menuApplication.pinned ? "Unpin from launcher" : "Pin to launcher"}</MenuItem>
            {search.pinSaveFailed ? <MenuItem id="retry-pin-save" onClick={() => nickel.applications.retryPinSave()}>Retry saving favorites</MenuItem> : null}
        </Menu> : null}
      </div>
    </FixedWindow>;
}
