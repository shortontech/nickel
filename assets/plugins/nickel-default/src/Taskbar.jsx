import { taskItems, activateTask } from "./tasks.js";
// @jsx h
import "./styles/taskbar.css";
// The package groups public application/window snapshots and requests native actions.
function Task(props) {
    const item = props.item;
    const [menuOpen, setMenuOpen] = useState(false);
    const buttonId = "taskbar-item-" + item.id;
    const menuId = "taskbar-menu-" + item.id;
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
                nickel.applications.movePin(item.id, move < 0 ? -1 : 1);
            }
        }
    };
    return <div className="taskbar-item"><Button id={buttonId}
        className={item.active ? "task-button is-active" : "task-button"}
        accessibilityLabel={item.name}
        icon={item.icon || null}
        onDrag={onDrag}
        onContextMenu={() => {
            setMenuOpen(true);
            nickel.openMenu(menuId);
        }}
        onClick={() => {
            if (suppressClick.current) {
                suppressClick.current = false;
                return;
            }
            activateTask(item, nickel);
        }}>
        {label}
    </Button>
    {menuOpen ? <Menu id={menuId} anchor={buttonId} open={true} onClose={() => setMenuOpen(false)}>
        {item.windows.map(window => <MenuItem key={window.id} id={"activate-" + window.id}
            disabled={window.canActivate === false} onClick={() => { nickel.windows.activate(window.id); setMenuOpen(false); }}>
            {window.title || item.name}
        </MenuItem>)}
        {item.windows.filter(window => window.canClose).map(window => <MenuItem key={"close-" + window.id} id={"close-" + window.id}
            onClick={() => { nickel.windows.close(window.id); setMenuOpen(false); }}>
            {"Close " + (window.title || item.name)}
        </MenuItem>)}
        {(item.pinned || item.canPin) ? <MenuItem id="toggle-pin" onClick={() => { nickel.applications.togglePin(item.id); setMenuOpen(false); }}>
            {item.pinned ? 'Unpin' : 'Pin'}
        </MenuItem> : null}
    </Menu> : null}
    </div>;
}
function TrayItem(props) {
    const item = props.item;
    return <Button id={"taskbar-tray-" + item.id} className="tray-button" accessibilityLabel={item.title}
        icon={item.icon ? "tray:" + item.id : null}
        onContextMenu={() => nickel.tray.contextMenu(item.id)}
        onClick={() => nickel.tray.activate(item.id)}>
        {item.title.charAt(0).toUpperCase() || "?"}
    </Button>;
}

export function Taskbar(props) {
    const features = nickel.features?.get() || {keyboard:{},codex:{}};
    const keyboardEnabled = features.keyboard.enabled === true;
    const codexAvailable = ['enabled', 'enabling', 'rejected', 'stale'].includes(features.codex.state);
    const clock = nickel.clock.get();
    const localTime = new Date(clock.unixMilliseconds + clock.utcOffsetMinutes * 60000);
    const hours = localTime.getUTCHours();
    const clockText = (hours % 12 || 12) + ':' + String(localTime.getUTCMinutes()).padStart(2, '0') + (hours >= 12 ? ' PM' : ' AM');
    const items = taskItems(nickel.applications.list(), nickel.windows.list());
    const tray = nickel.tray.list();
    const contributions = nickel.contributions("taskbar.items");
    return <FixedWindow id="taskbar" output="all" edge="bottom"
        reserveWorkArea={true} className="taskbar">
        <div className="taskbar-content">
            <Button id="taskbar-launcher" className="launcher-button" icon="nickel-logo" showLabel={true} accessibilityLabel="Open Nickel Start"
                onClick={() => nickel.surfaces.show("launcher")}>Nickel</Button>
            {items.map(item => <Task key={item.id} item={item} />)}
            {contributions.map(entry => <entry.component key={entry.key} />)}
            <Spacer className="taskbar-spacer" />
            {keyboardEnabled ? <Button id="taskbar-keyboard" className="utility-button" accessibilityLabel="On-screen keyboard"
                onClick={() => nickel.keyboard.toggle()}>⌨</Button> : null}
            {codexAvailable ? <Button id="taskbar-codex" className="utility-button" icon="codex" accessibilityLabel="Codex projects"
                onClick={() => nickel.projects.toggle()}>Codex</Button> : null}
            {tray.map(item => <TrayItem key={item.id} item={item} />)}
            <Button id="taskbar-control" className="clock-button"
                onClick={() => nickel.surfaces.show("quick-settings")}>{clockText}</Button>
        </div>
    </FixedWindow>;
}
