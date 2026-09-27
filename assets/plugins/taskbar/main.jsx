// @jsx h
// The host supplies grouped tasks and performs all window and launch actions.
function Task(props) {
    const item = props.item;
    const label = (item.active ? "●" : "") + (item.name.charAt(0).toUpperCase() || "?");
    return <Button id={"taskbar-item-" + item.index}
        accessibilityLabel={item.name}
        onClick={() => nickel.request({type: "taskbar-activate-item", index: item.index, id: item.id})}>
        {label}
    </Button>;
}

function App() {
    const data = nickel.data;
    return <Panel height={56} background={0xf1222730}>
        <Row>
            <Button id="taskbar-launcher"
                onClick={() => nickel.request({type: "taskbar-toggle-launcher"})}>Nickel</Button>
            {data.items.map(item => <Task key={item.id} item={item} />)}
            <Spacer />
            <Button id="taskbar-control"
                onClick={() => nickel.request({type: "taskbar-toggle-control"})}>{data.clock}</Button>
        </Row>
    </Panel>;
}
