// @jsx h
import "./styles/notifications.css";
// Nickel supplies a bounded snapshot and checks every action against its live feed.
export function Notifications(props) {
    const data = {...{notification:null,history:[]}, ...nickel.notifications.get()};
    const [historyVisible, setHistoryVisible] = useState(false);
    const current = data.notification;
    let content;
    if (historyVisible) {
        content = <Column>
                <Row>
                    <Text>Notifications</Text>
                    <Button id="notification-close-history" onClick={() => nickel.surfaces.hide("notifications")}>Close</Button>
                </Row>
                <ScrollView id="notification-history-scroll" height={140}>
                    {data.history.length === 0 ? <Text>No notifications</Text> : null}
                    {data.history.map(item => <Column key={item.id}>
                        <Text>{item.summary || item.appName}</Text>
                        <Text>{item.body}</Text>
                    </Column>)}
                </ScrollView>
            </Column>;
    } else if (!current) {
        content = <Text>No notification</Text>;
    } else {
        content = <Column>
            <Text>{current.summary || current.appName}</Text>
            <ScrollView id="notification-body-scroll" height={82}><Text>{current.body}</Text></ScrollView>
            <Row>
                {current.actions.map(action => <Button key={action.key} id={"notification-action-" + action.key}
                    onClick={() => nickel.notifications.invoke(current.id, action.key)}>
                    {action.label}
                </Button>)}
                <Button id="notification-dismiss" onClick={() => nickel.notifications.dismiss(current.id)}>Dismiss</Button>
            </Row>
        </Column>;
    }
    return <Window id="notifications" placement="fixed" anchor="top-right" width={420} height={180} className="notification-window"
        onEscape={historyVisible
            ? () => nickel.surfaces.hide("notifications")
            : current ? () => nickel.notifications.dismiss(current.id) : undefined}>
        <div className="notification-content">
            {!historyVisible ? <Row>
                <Text>Notifications</Text>
                <Button id="notification-history" onClick={() => setHistoryVisible(true)}>History</Button>
            </Row> : null}
            {content}
        </div>
    </Window>;
}
