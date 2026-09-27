// @jsx h
// The host owns search ranking and launches. This plugin owns the view.
function App() {
    const data = nickel.data;
    const [logoutOpen, setLogoutOpen] = useState(false);
    const [menuTarget, setMenuTarget] = useState(null);
    const openAppMenu = (item, kind, anchor) => {
        setMenuTarget({ id: item.id, index: item.index, pinned: item.pinned, kind, anchor });
        nickel.openMenu("launcher-app-actions");
    };
    return h(Column, null,
        h(Text, null, "Nickel Launcher"),
        data.status ? h(Text, null, data.status) : null,
        h(TextField, { id: "launcher-query", value: data.query, placeholder: "Search applications", onChange: query => nickel.request({ type: "launcher-set-query", query }) }),
        data.dashboardVisible ? h(ScrollView, { id: "launcher-dashboard-scroll", height: 580 },
            h(Text, null, "Places"),
            data.places.map(place => h(Row, null,
                h(Button, { id: "launcher-place-" + place.index, icon: "place:" + place.index, showLabel: true, onContextMenu: () => openAppMenu(place, "dashboard", "launcher-place-" + place.index), onClick: () => nickel.request({ type: "launcher-launch-dashboard", id: place.id }) }, place.name),
                h(Button, { id: "launcher-pin-place-" + place.index, onClick: () => nickel.request({ type: "launcher-toggle-pin", id: place.id }) }, place.pinned ? "Unpin" : "Pin"))),
            data.codexAvailable ? h(Column, null,
                h(Text, null, "Recent projects"),
                data.projects.map(project => h(Button, { id: "launcher-project-" + project.id, onClick: () => nickel.request({ type: "launcher-open-project", id: project.id }) }, project.name)),
                h(Button, { id: "launcher-all-projects", onClick: () => nickel.request({ type: "launcher-see-all-projects" }) }, "All projects")) : null,
            h(Row, null,
                h(Button, { id: "launcher-view-favorites", onClick: () => nickel.request({ type: "launcher-set-view", view: "favorites" }) }, "Pinned & recent"),
                h(Button, { id: "launcher-view-applications", onClick: () => nickel.request({ type: "launcher-set-view", view: "applications" }) }, "All applications"),
                h(Button, { id: "launcher-view-places", onClick: () => nickel.request({ type: "launcher-set-view", view: "places" }) }, "Places")),
            h(Text, null, data.view === "favorites" ? "Pinned and recent" : data.view === "applications" ? "All applications" : "Places"),
            data.dashboard.length === 0 ? h(Text, null, "No applications in this view") : null,
            data.dashboard.map(app => h(Row, null,
                h(Button, { id: "launcher-dashboard-" + app.index, icon: "dashboard:" + app.index, showLabel: true, onContextMenu: () => openAppMenu(app, "dashboard", "launcher-dashboard-" + app.index), onClick: () => nickel.request({ type: "launcher-launch-dashboard", id: app.id }) }, app.name),
                h(Button, { id: "launcher-pin-dashboard-" + app.index, accessibilityLabel: (app.pinned ? "Unpin " : "Pin ") + app.name, onClick: () => nickel.request({ type: "launcher-toggle-pin", id: app.id }) }, app.pinned ? "Unpin" : "Pin"))),
            data.dashboardPageCount > 1 ? h(Row, null,
                data.dashboardPage > 0 ? h(Button, { id: "launcher-dashboard-previous", onClick: () => nickel.request({ type: "launcher-set-page", view: "dashboard", page: data.dashboardPage - 1 }) }, "Previous") : null,
                h(Text, null, "Page " + (data.dashboardPage + 1) + " of " + data.dashboardPageCount),
                data.dashboardPage + 1 < data.dashboardPageCount ? h(Button, { id: "launcher-dashboard-next", onClick: () => nickel.request({ type: "launcher-set-page", view: "dashboard", page: data.dashboardPage + 1 }) }, "Next") : null) : null,
            h(Button, { id: "launcher-account", onClick: () => nickel.request({ type: "launcher-open-account" }) }, data.accountName),
            h(Button, { id: "launcher-settings", onClick: () => nickel.request({ type: "launcher-open-settings" }) }, "Settings"),
            data.logoutAvailable ? h(Button, { id: "launcher-logout", onClick: () => {
                    setLogoutOpen(true);
                    nickel.openDialog("launcher-logout-dialog");
                } }, "Log out") : null) : null,
        !data.dashboardVisible ?
            h(ScrollView, { id: "launcher-search-scroll", height: 580 },
                data.results.length === 0 ? h(Text, null, "No applications found") : null,
                data.results.map(result => h(Row, null,
                    h(Button, { id: "launcher-result-" + result.index, icon: "search:" + result.index, showLabel: true, onContextMenu: () => openAppMenu(result, "search", "launcher-result-" + result.index), onClick: () => nickel.request({ type: "launcher-activate-result", index: result.index, id: result.id }) }, result.name),
                    h(Button, { id: "launcher-pin-result-" + result.index, accessibilityLabel: (result.pinned ? "Unpin " : "Pin ") + result.name, onClick: () => nickel.request({ type: "launcher-toggle-pin", id: result.id }) }, result.pinned ? "Unpin" : "Pin"))),
                data.resultPageCount > 1 ? h(Row, null,
                    data.resultPage > 0 ? h(Button, { id: "launcher-search-previous", onClick: () => nickel.request({ type: "launcher-set-page", view: "search", page: data.resultPage - 1 }) }, "Previous") : null,
                    h(Text, null, "Page " + (data.resultPage + 1) + " of " + data.resultPageCount),
                    data.resultPage + 1 < data.resultPageCount ? h(Button, { id: "launcher-search-next", onClick: () => nickel.request({ type: "launcher-set-page", view: "search", page: data.resultPage + 1 }) }, "Next") : null) : null) : null,
        h(Dialog, { id: "launcher-logout-dialog", anchor: "launcher-logout", open: logoutOpen, onClose: () => setLogoutOpen(false), width: 320, height: 160 },
            h(Column, null,
                h(Text, null, "Log out of this session?"),
                h(Button, { id: "launcher-confirm-logout", accessibilityLabel: "Confirm log out", onClick: () => {
                        setLogoutOpen(false);
                        nickel.request({ type: "launcher-request-logout" });
                    } }, "Log out"),
                h(Button, { id: "launcher-cancel-logout", onClick: () => setLogoutOpen(false) }, "Cancel"))),
        menuTarget ? h(Menu, { id: "launcher-app-actions", anchor: menuTarget.anchor, open: true },
            h(MenuItem, { id: "launch", onClick: () => {
                    if (menuTarget.kind === "search") {
                        nickel.request({ type: "launcher-activate-result", index: menuTarget.index, id: menuTarget.id });
                    }
                    else {
                        nickel.request({ type: "launcher-launch-dashboard", id: menuTarget.id });
                    }
                } }, "Launch"),
            h(MenuItem, { id: "toggle-pin", onClick: () => nickel.request({ type: "launcher-toggle-pin", id: menuTarget.id }) }, menuTarget.pinned ? "Unpin from Nickel Bar" : "Pin to Nickel Bar")) : null);
}
