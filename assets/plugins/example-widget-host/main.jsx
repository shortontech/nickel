// @jsx h
function App() {
    const widgets = nickel.data.slots.metrics || [];
    return <Panel height={280} background={0xff202830}>
        <Column>
            <Text>Widget host</Text>
            {widgets.length === 0 ? <Text>No metrics yet</Text> : null}
            {widgets.map((item, index) => <Row key={item.pluginId + ':' + index}>
                <Text>{item.label}</Text>
                <Text>{item.value}</Text>
            </Row>)}
        </Column>
    </Panel>;
}
