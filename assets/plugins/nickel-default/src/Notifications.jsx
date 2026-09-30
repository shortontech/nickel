// @jsx h
import "./styles/notifications.css";
// Nickel supplies a bounded snapshot and checks every action against its live feed.
export function Notifications() {
    const data = nickel.data;
    const current = data.notification;
    let content;
    if (data.historyVisible) {
        content = <Column>
                <Row>
                    <Text>Notifications</Text>
                    <Button id="notification-close-history" onClick={() => nickel.request({type: "notification-close-history"})}>Close</Button>
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
                    onClick={() => nickel.request({type: "notification-invoke", id: current.id, key: action.key})}>
                    {action.label}
                </Button>)}
                <Button id="notification-dismiss" onClick={() => nickel.request({type: "notification-dismiss", id: current.id})}>Dismiss</Button>
            </Row>
        </Column>;
    }
    return <Window id="notifications" placement="fixed" anchor="top-right" width={420} height={180} className="notification-window"
        onEscape={data.historyVisible
            ? () => nickel.request({type: "notification-close-history"})
            : current ? () => nickel.request({type: "notification-dismiss", id: current.id}) : undefined}>
        <div className="notification-content">{content}</div>
    </Window>;
}
