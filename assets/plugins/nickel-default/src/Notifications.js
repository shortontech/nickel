// @jsx h
import "./styles/notifications.css";
// Nickel supplies a bounded snapshot and checks every action against its live feed.
export function Notifications(props) {
    const data = { ...{ notification: null, history: [] }, ...nickel.notifications.get() };
    const [historyVisible, setHistoryVisible] = useState(false);
    const current = data.notification;
    let content;
    if (historyVisible) {
        content = h(Column, null,
            h(Row, null,
                h(Text, null, "Notifications"),
                h(Button, { id: "notification-close-history", onClick: () => nickel.surfaces.hide("notifications") }, "Close")),
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
                current.actions.map(action => h(Button, { key: action.key, id: "notification-action-" + action.key, onClick: () => nickel.notifications.invoke(current.id, action.key) }, action.label)),
                h(Button, { id: "notification-dismiss", onClick: () => nickel.notifications.dismiss(current.id) }, "Dismiss")));
    }
    return h(Window, { id: "notifications", placement: "fixed", anchor: "top-right", width: 420, height: 180, className: "notification-window", onEscape: historyVisible
            ? () => nickel.surfaces.hide("notifications")
            : current ? () => nickel.notifications.dismiss(current.id) : undefined },
        h("div", { className: "notification-content" },
            !historyVisible ? h(Row, null,
                h(Text, null, "Notifications"),
                h(Button, { id: "notification-history", onClick: () => setHistoryVisible(true) }, "History")) : null,
            content));
}
