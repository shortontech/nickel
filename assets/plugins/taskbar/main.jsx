// @jsx h
// The host supplies grouped tasks and performs all window and launch actions.
function Task(props) {
    const item = props.item;
    const label = (item.active ? "●" : "") + (item.name.charAt(0).toUpperCase() || "?");
    const pendingDrag = useRef(0);
    const suppressClick = useRef(false);
    const onDrag = gesture => {
        if (gesture.phase === "start" || gesture.phase === "cancel") {
            pendingDrag.current = 0;
            suppressClick.current = false;
            return;
        }
        const direction = gesture.x < gesture.bounds.x ? -1
            : gesture.x > gesture.bounds.x + gesture.bounds.width ? 1 : 0;
        if (gesture.phase === "move") {
            if (direction) pendingDrag.current = direction;
            return;
        }
        if (gesture.phase === "end" && item.pinned) {
            const move = pendingDrag.current || direction;
            pendingDrag.current = 0;
            if (move) {
                suppressClick.current = true;
                nickel.request({type: "taskbar-move-pin", index: item.index,
                    id: item.id, direction: move < 0 ? "left" : "right"});
            }
        }
    };
    return <Button id={"taskbar-item-" + item.index}
        className={item.active ? "task-button is-active" : "task-button"}
        accessibilityLabel={item.name}
        icon={item.icon ? "task:" + item.index : null}
        onDrag={onDrag}
        onContextMenu={() => nickel.request({type: "taskbar-context-item", index: item.index, id: item.id})}
        onClick={() => {
            if (suppressClick.current) {
                suppressClick.current = false;
                return;
            }
            nickel.request({type: "taskbar-activate-item", index: item.index, id: item.id});
        }}>
        {label}
    </Button>;
}

function TrayItem(props) {
    const item = props.item;
    return <Button id={"taskbar-tray-" + item.id} className="tray-button" accessibilityLabel={item.title}
        icon={item.icon ? "tray:" + item.id : null}
        onContextMenu={() => nickel.request({type: "taskbar-context-tray", id: item.id})}
        onClick={() => nickel.request({type: "taskbar-activate-tray", id: item.id})}>
        {item.title.charAt(0).toUpperCase() || "?"}
    </Button>;
}

function App() {
    const data = nickel.data;
    const items = data.items || [];
    const tray = data.tray || [];
    const badges = (data.slots && data.slots["task-badge"]) || [];
    return <FixedWindow output="all" edge="bottom"
        reserveWorkArea={true} className="taskbar">
        <div className="taskbar-content">
            <Button id="taskbar-launcher" className="launcher-button" icon="logo" accessibilityLabel="Open Nickel Start"
                onClick={() => nickel.request({type: "toggle-launcher"})}>Nickel</Button>
            {items.flatMap(item => [
                <Task key={item.id} item={item} />,
                ...badges.filter(badge => badge.item === item.id).slice(0, 3).map((badge, index) =>
                    <Badge key={badge.pluginId + ":" + item.id + ":" + index} className="task-badge"
                        label={badge.label} count={badge.count} color={badge.color} />)
            ])}
            <Spacer className="taskbar-spacer" />
            {data.keyboardEnabled ? <Button id="taskbar-keyboard" className="utility-button" accessibilityLabel="On-screen keyboard"
                onClick={() => nickel.request({type: "toggle-on-screen-keyboard"})}>⌨</Button> : null}
            {data.codexAvailable ? <Button id="taskbar-codex" className="utility-button" icon="codex" accessibilityLabel="Codex projects"
                onClick={() => nickel.request({type: "toggle-projects-menu"})}>Codex</Button> : null}
            {tray.map(item => <TrayItem key={item.id} item={item} />)}
            <Button id="taskbar-control" className="clock-button"
                onClick={() => nickel.request({type: "toggle-control-center"})}>{data.clock || ""}</Button>
        </div>
    </FixedWindow>;
}
