// @jsx h
// Nickel supplies a bounded snapshot and checks every action against its live feed.
function App() {
    const data = nickel.data;
    const current = data.notification;
    if (data.historyVisible) {
        return <Panel height={180}>
            <Column>
                <Row>
                    <Text>Notifications</Text>
                    <Button id="notification-close-history" onClick={() => nickel.request({type: "notification-close-history"})}>Close</Button>
                </Row>
                <ScrollView id="notification-history-scroll" height={140}>
                    {data.history.length === 0 ? <Text>No notifications</Text> : null}
                    {data.history.map(item => <Column>
                        <Text>{item.summary || item.appName}</Text>
                        <Text>{item.body}</Text>
                    </Column>)}
                </ScrollView>
            </Column>
        </Panel>;
    }
    if (!current) return <Panel height={180}><Text>No notification</Text></Panel>;
    return <Panel height={180}>
        <Column>
            <Text>{current.summary || current.appName}</Text>
            <ScrollView id="notification-body-scroll" height={82}><Text>{current.body}</Text></ScrollView>
            <Row>
                {current.actions.map(action => <Button id={"notification-action-" + action.key}
                    onClick={() => nickel.request({type: "notification-invoke", id: current.id, key: action.key})}>
                    {action.label}
                </Button>)}
                <Button id="notification-dismiss" onClick={() => nickel.request({type: "notification-dismiss", id: current.id})}>Dismiss</Button>
            </Row>
        </Column>
    </Panel>;
}
