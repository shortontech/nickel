// @jsx h
import "./styles/launcher.css";
const PAGE_SIZE = 12;
function appControlId(prefix, app) { return prefix + (app.icon || app.id); }
function pageItems(items, page) { return items.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE); }
function Pages(props) {
    const count = Math.max(1, Math.ceil(props.total / PAGE_SIZE));
    return count > 1 ? h(Row, null,
        props.page > 0 ? h(Button, { id: props.id + "-previous", onClick: () => props.onChange(props.page - 1) }, "Previous") : null,
        h(Text, null, "Page " + (props.page + 1) + " of " + count),
        props.page + 1 < count ? h(Button, { id: props.id + "-next", onClick: () => props.onChange(props.page + 1) }, "Next") : null) : null;
}
// Query, tabs, paging, and menus are presentation state in this package.
// Applications retains the authoritative inventory and native fuzzy ranking.
export function Launcher() {
    const applications = nickel.applications.list();
    const search = nickel.applications.searchResults();
    const session = nickel.session?.get() || { account: { displayName: "Local session" }, support: {} };
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
    const openAppMenu = (item, anchor) => { setMenuTarget({ id: item.id, anchor }); nickel.openMenu("launcher-app-actions"); };
    const menuApplication = menuTarget && [...applications, ...results].find(app => app.id === menuTarget.id);
    const launch = id => { nickel.applications.launch(id); nickel.surfaces.hide("launcher"); };
    const submit = () => { if (!dashboardVisible && results[0])
        launch(results[0].id); };
    return h(Window, { id: "launcher", title: "Nickel Launcher", className: "launcher-window", onEscape: () => query ? changeQuery("") : nickel.surfaces.hide("launcher"), onSubmit: !dashboardVisible && results.length ? submit : undefined },
        h("div", { className: "launcher-content" },
            search.status ? h(Text, { className: "launcher-status" }, search.status) : null,
            h(TextField, { autoFocus: true, id: "launcher-query", className: "launcher-search", value: query, placeholder: "Search applications", onChange: changeQuery }),
            dashboardVisible ? h(ScrollView, { id: "launcher-dashboard-scroll", className: "launcher-scroll", grow: true },
                h(Text, { className: "launcher-section" }, "Places"),
                places.map(place => h(Row, { key: place.id, className: "launcher-app-row" },
                    h(Button, { id: appControlId("launcher-place-", place), className: "launcher-app-button", icon: place.icon, showLabel: true, onContextMenu: () => openAppMenu(place, appControlId("launcher-place-", place)), onClick: () => launch(place.id) }, place.name),
                    h(Button, { id: appControlId("launcher-pin-place-", place), className: "launcher-pin-button", onClick: () => nickel.applications.togglePin(place.id) }, place.pinned ? "Unpin" : "Pin"))),
                search.nativeProjectsAvailable ? h(Column, { className: "launcher-projects" },
                    h(Text, { className: "launcher-section" }, "Projects"),
                    h(Button, { id: "launcher-all-projects", className: "launcher-footer-button", onClick: () => nickel.projects.show() }, "Open native project overview")) : null,
                h(Row, { className: "launcher-tabs" },
                    h(Button, { id: "launcher-view-favorites", className: view === "favorites" ? "launcher-tab selected" : "launcher-tab", onClick: () => changeView("favorites") }, "Pinned & recent"),
                    h(Button, { id: "launcher-view-applications", className: view === "applications" ? "launcher-tab selected" : "launcher-tab", onClick: () => changeView("applications") }, "All applications"),
                    h(Button, { id: "launcher-view-places", className: view === "places" ? "launcher-tab selected" : "launcher-tab", onClick: () => changeView("places") }, "Places")),
                h(Text, { className: "launcher-section" }, view === "favorites" ? "Pinned and recent" : view === "applications" ? "All applications" : "Places"),
                search.catalogTruncated ? h(Text, { wrap: true }, "Search to find applications beyond this bounded catalog.") : null,
                !dashboard.length ? h(Text, null, "No applications in this view") : null,
                h("div", { className: "launcher-app-grid" }, pageItems(dashboard, safeDashboardPage).map(app => h("div", { key: app.id, className: "launcher-app-card" },
                    h(Button, { id: appControlId("launcher-dashboard-", app), className: "launcher-icon-button", icon: app.icon, accessibilityLabel: app.name, onContextMenu: () => openAppMenu(app, appControlId("launcher-dashboard-", app)), onClick: () => launch(app.id) }, app.name.charAt(0).toUpperCase()),
                    h(Text, { className: "launcher-app-name", wrap: true }, app.name)))),
                h(Pages, { id: "launcher-dashboard", page: safeDashboardPage, total: dashboard.length, onChange: setDashboardPage })) : h(ScrollView, { id: "launcher-search-scroll", className: "launcher-scroll", grow: true },
                !search.available ? h(Text, null, search.reason || "Application search is unavailable.") : search.query !== query ? h(Text, null, "Searching\u2026") : !results.length ? h(Text, null, "No applications found") : null,
                pageItems(results, safeResultPage).map(result => h(Row, { key: result.id, className: "launcher-app-row" },
                    h(Button, { id: appControlId("launcher-result-", result), className: "launcher-app-button", icon: result.icon, showLabel: true, onContextMenu: () => openAppMenu(result, appControlId("launcher-result-", result)), onClick: () => launch(result.id) }, result.name),
                    h(Button, { id: appControlId("launcher-pin-result-", result), className: "launcher-pin-button", accessibilityLabel: (result.pinned ? "Unpin " : "Pin ") + result.name, onClick: () => nickel.applications.togglePin(result.id) }, result.pinned ? "Unpin" : "Pin"))),
                search.truncated ? h(Text, { wrap: true }, "More matches are available. Refine your search to narrow the results.") : null,
                h(Pages, { id: "launcher-search", page: safeResultPage, total: results.length, onChange: setResultPage })),
            h(Row, { className: "launcher-footer" },
                h(Button, { id: "launcher-account", className: "launcher-account-button", onClick: () => nickel.surfaces.show("quick-settings") }, session.account?.displayName || "Local session"),
                session.support.logout ? h(Button, { id: "launcher-logout", className: "launcher-footer-button", onClick: () => { setLogoutOpen(true); nickel.openDialog("launcher-logout-dialog"); } }, "Log out") : null,
                h(Button, { id: "launcher-settings", className: "launcher-footer-button", onClick: () => nickel.surfaces.show("settings") }, "Settings")),
            h(Dialog, { id: "launcher-logout-dialog", anchor: "launcher-logout", open: logoutOpen, onClose: () => setLogoutOpen(false), width: 320, height: 160 },
                h(Column, null,
                    h(Text, null, "Log out of this session?"),
                    h(Button, { id: "launcher-confirm-logout", accessibilityLabel: "Confirm log out", onClick: () => { setLogoutOpen(false); nickel.session.logout(); } }, "Log out"),
                    h(Button, { id: "launcher-cancel-logout", onClick: () => setLogoutOpen(false) }, "Cancel"))),
            menuApplication ? h(Menu, { id: "launcher-app-actions", anchor: menuTarget.anchor, open: true },
                h(MenuItem, { id: "launch", onClick: () => { launch(menuApplication.id); setMenuTarget(null); } }, "Launch"),
                h(MenuItem, { id: "toggle-pin", onClick: () => nickel.applications.togglePin(menuApplication.id) }, menuApplication.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar"),
                search.pinSaveFailed ? h(MenuItem, { id: "retry-pin-save", onClick: () => nickel.applications.retryPinSave() }, "Retry saving favorites") : null) : null));
}
