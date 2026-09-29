// @jsx h
// Nickel supplies a bounded snapshot and checks every action against its live feed.
function App() {
    const data = nickel.data;
    const current = data.notification;
    let content;
    if (data.historyVisible) {
        content = h(Column, null,
            h(Row, null,
                h(Text, null, "Notifications"),
                h(Button, { id: "notification-close-history", onClick: () => nickel.request({ type: "notification-close-history" }) }, "Close")),
            h(ScrollView, { id: "notification-history-scroll", height: 140 },
                data.history.length === 0 ? h(Text, null, "No notifications") : null,
                data.history.map(item => h(Column, { key: item.id },
                    h(Text, null, item.summary || item.appName),
                    h(Text, null, item.body)))));
    }
    else if (!current) {
        content = h(Text, null, "No notification");
    }
    else {
        content = h(Column, null,
            h(Text, null, current.summary || current.appName),
            h(ScrollView, { id: "notification-body-scroll", height: 82 },
                h(Text, null, current.body)),
            h(Row, null,
                current.actions.map(action => h(Button, { key: action.key, id: "notification-action-" + action.key, onClick: () => nickel.request({ type: "notification-invoke", id: current.id, key: action.key }) }, action.label)),
                h(Button, { id: "notification-dismiss", onClick: () => nickel.request({ type: "notification-dismiss", id: current.id }) }, "Dismiss")));
    }
    return h(Window, { id: "main", placement: "fixed", anchor: "top-right", width: 420, height: 180, className: "notification-window" },
        h("div", { className: "notification-content" }, content));
}
