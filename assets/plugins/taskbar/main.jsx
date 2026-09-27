// @jsx h
// The host supplies grouped tasks and performs all window and launch actions.
function Task(props) {
    const item = props.item;
    const label = (item.active ? "●" : "") + (item.name.charAt(0).toUpperCase() || "?");
    return <Button id={"taskbar-item-" + item.index}
        accessibilityLabel={item.name}
        icon={item.icon ? "task:" + item.index : null}
        onContextMenu={() => nickel.request({type: "taskbar-context-item", index: item.index, id: item.id})}
        onClick={() => nickel.request({type: "taskbar-activate-item", index: item.index, id: item.id})}>
        {label}
    </Button>;
}

function TrayItem(props) {
    const item = props.item;
    return <Button id={"taskbar-tray-" + item.id} accessibilityLabel={item.title}
        icon={item.icon ? "tray:" + item.id : null}
        onContextMenu={() => nickel.request({type: "taskbar-context-tray", id: item.id})}
        onClick={() => nickel.request({type: "taskbar-activate-tray", id: item.id})}>
        {item.title.charAt(0).toUpperCase() || "?"}
    </Button>;
}

function App() {
    const data = nickel.data;
    return <Panel height={56} background={0xf1222730}>
        <Row>
            <Button id="taskbar-launcher" icon="logo" accessibilityLabel="Open Nickel Start"
                onClick={() => nickel.request({type: "taskbar-toggle-launcher"})}>Nickel</Button>
            {data.items.map(item => <Task key={item.id} item={item} />)}
            <Spacer />
            {data.tray.map(item => <TrayItem key={item.id} item={item} />)}
            <Button id="taskbar-control"
                onClick={() => nickel.request({type: "taskbar-toggle-control"})}>{data.clock}</Button>
        </Row>
    </Panel>;
}
