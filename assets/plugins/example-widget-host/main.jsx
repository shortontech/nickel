// @jsx h
function App() {
    const widgets = nickel.data.slots.metrics || [];
    const actions = nickel.data.slots.commands || [];
    return <Window width={420} height={280} className="widget-host">
        <Column>
            <Text>Widget host</Text>
            {actions.map(item => <Button key={item.pluginId + ':' + item.id}
                id={'slot-action-' + item.id}
                onClick={() => nickel.request({type: 'invoke-plugin-slot-action',
                    slot: 'commands', pluginId: item.pluginId, id: item.id})}>
                {item.label}
            </Button>)}
            {widgets.length === 0 ? <Text>No metrics yet</Text> : null}
            {widgets.map((item, index) => <Row key={item.pluginId + ':' + index}>
                <Text>{item.label}</Text>
                <Text>{item.value}</Text>
            </Row>)}
        </Column>
    </Window>;
}
