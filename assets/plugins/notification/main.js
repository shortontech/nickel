// @jsx h
// Nickel supplies a bounded snapshot and checks every action against its live feed.
function App() {
    const data = nickel.data;
    const current = data.notification;
    if (data.historyVisible) {
        return h(Panel, { height: 180 },
            h(Column, null,
                h(Row, null,
                    h(Text, null, "Notifications"),
                    h(Button, { id: "notification-close-history", onClick: () => nickel.request({ type: "notification-close-history" }) }, "Close")),
                h(ScrollView, { id: "notification-history-scroll", height: 140 },
                    data.history.length === 0 ? h(Text, null, "No notifications") : null,
                    data.history.map(item => h(Column, null,
                        h(Text, null, item.summary || item.appName),
                        h(Text, null, item.body))))));
    }
    if (!current)
        return h(Panel, { height: 180 },
            h(Text, null, "No notification"));
    return h(Panel, { height: 180 },
        h(Column, null,
            h(Text, null, current.summary || current.appName),
            h(ScrollView, { id: "notification-body-scroll", height: 82 },
                h(Text, null, current.body)),
            h(Row, null,
                current.actions.map(action => h(Button, { id: "notification-action-" + action.key, onClick: () => nickel.request({ type: "notification-invoke", id: current.id, key: action.key }) }, action.label)),
                h(Button, { id: "notification-dismiss", onClick: () => nickel.request({ type: "notification-dismiss", id: current.id }) }, "Dismiss"))));
}
