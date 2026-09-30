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
            if (direction)
                pendingDrag.current = direction;
            return;
        }
        if (gesture.phase === "end" && item.pinned) {
            const move = pendingDrag.current || direction;
            pendingDrag.current = 0;
            if (move) {
                suppressClick.current = true;
                if (item.capabilityModel) {
                    nickel.applications.movePin(item.id, move < 0 ? -1 : 1);
                    return;
                }
                nickel.request({ type: "taskbar-move-pin", index: item.index,
                    id: item.id, direction: move < 0 ? "left" : "right" });
            }
        }
    };
    return h("div", { className: "taskbar-item" },
        h(Button, { id: buttonId, className: item.active ? "task-button is-active" : "task-button", accessibilityLabel: item.name, icon: item.icon ? "task:" + item.index : null, onDrag: onDrag, onContextMenu: () => {
                if (item.capabilityModel) {
                    setMenuOpen(true);
                    nickel.openMenu(menuId);
                }
                else
                    nickel.request({ type: "taskbar-context-item", index: item.index, id: item.id });
            }, onClick: () => {
                if (suppressClick.current) {
                    suppressClick.current = false;
                    return;
                }
                if (item.capabilityModel)
                    activateTask(item, nickel);
                else
                    nickel.request({ type: "taskbar-activate-item", index: item.index, id: item.id });
            } }, label),
        menuOpen ? h(Menu, { id: menuId, anchor: buttonId, open: true, onClose: () => setMenuOpen(false) },
            item.windows.map(window => h(MenuItem, { key: window.id, id: "activate-" + window.id, disabled: window.canActivate === false, onClick: () => { nickel.windows.activate(window.id); setMenuOpen(false); } }, window.title || item.name)),
            item.windows.filter(window => window.canClose).map(window => h(MenuItem, { key: "close-" + window.id, id: "close-" + window.id, onClick: () => { nickel.windows.close(window.id); setMenuOpen(false); } }, "Close " + (window.title || item.name))),
            !item.id.startsWith('window:') ? h(MenuItem, { id: "toggle-pin", onClick: () => { nickel.applications.togglePin(item.id); setMenuOpen(false); } }, item.pinned ? 'Unpin' : 'Pin') : null) : null);
}
function TrayItem(props) {
    const item = props.item;
    return h(Button, { id: "taskbar-tray-" + item.id, className: "tray-button", accessibilityLabel: item.title, icon: item.icon ? "tray:" + item.id : null, onContextMenu: () => nickel.tray.contextMenu(item.id), onClick: () => nickel.tray.activate(item.id) }, item.title.charAt(0).toUpperCase() || "?");
}
export function Taskbar(props) {
    const data = { ...{ items: [], tray: [], slots: {}, clock: '', codexAvailable: false, keyboardEnabled: false }, ...props?.data };
    const items = props?.data?.items || taskItems(nickel.applications.list(), nickel.windows.list());
    const tray = props?.data?.tray || nickel.tray.list();
    const badges = (data.slots && data.slots["task-badge"]) || [];
    return h(FixedWindow, { id: "taskbar", output: "all", edge: "bottom", reserveWorkArea: true, className: "taskbar" },
        h("div", { className: "taskbar-content" },
            h(Button, { id: "taskbar-launcher", className: "launcher-button", icon: "logo", accessibilityLabel: "Open Nickel Start", onClick: () => nickel.surfaces.show("launcher") }, "Nickel"),
            items.flatMap(item => [
                h(Task, { key: item.id, item: item }),
                ...badges.filter(badge => badge.item === item.id).slice(0, 3).map((badge, index) => h(Badge, { key: badge.pluginId + ":" + item.id + ":" + index, className: "task-badge", label: badge.label, count: badge.count, color: badge.color }))
            ]),
            h(Spacer, { className: "taskbar-spacer" }),
            data.keyboardEnabled ? h(Button, { id: "taskbar-keyboard", className: "utility-button", accessibilityLabel: "On-screen keyboard", onClick: () => nickel.request({ type: "toggle-on-screen-keyboard" }) }, "\u2328") : null,
            data.codexAvailable ? h(Button, { id: "taskbar-codex", className: "utility-button", icon: "codex", accessibilityLabel: "Codex projects", onClick: () => nickel.request({ type: "toggle-projects-menu" }) }, "Codex") : null,
            tray.map(item => h(TrayItem, { key: item.id, item: item })),
            h(Button, { id: "taskbar-control", className: "clock-button", onClick: () => nickel.surfaces.show("quick-settings") }, data.clock || "")));
}
