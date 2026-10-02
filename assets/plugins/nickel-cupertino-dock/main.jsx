// @jsx h
// Floating Cupertino-style dock theme for Nickel's native renderer.

function taskItems(applications, windows) {
    const groups = new Map();
    const sorted = applications.slice().sort((left, right) =>
        (left.pinOrder ?? Number.MAX_SAFE_INTEGER) - (right.pinOrder ?? Number.MAX_SAFE_INTEGER));
    for (const application of sorted) {
        if (application.pinned) groups.set(application.id, {
            id: application.id,
            name: application.name,
            icon: application.icon,
            pinned: true,
            canLaunch: application.canLaunch !== false,
            windows: [],
            active: false
        });
    }
    for (const window of windows) {
        const id = window.applicationId || `window:${window.id}`;
        if (!groups.has(id)) {
            const application = applications.find(candidate => candidate.id === id);
            groups.set(id, {
                id,
                name: application?.name || window.title || "Window",
                icon: application?.icon,
                pinned: false,
                canLaunch: !!application && application.canLaunch !== false,
                windows: [],
                active: false
            });
        }
        const group = groups.get(id);
        group.windows.push(window);
        group.active ||= window.active;
    }
    return Array.from(groups.values()).slice(0, 5);
}

function activateTask(item) {
    if (!item.windows.length) {
        if (item.canLaunch) nickel.applications.launch(item.id);
        return;
    }
    const active = item.windows.findIndex(window => window.active);
    const window = item.windows[(active + 1) % item.windows.length];
    if (window.canActivate !== false) nickel.windows.activate(window.id);
}

function DockItem({item}) {
    const label = item.name.charAt(0).toUpperCase() || "?";
    return <div className="dock-item">
        <Button id={"cupertino-dock-item-" + item.id}
            className={item.active ? "dock-button active" : "dock-button"}
            width={58} height={58}
            accessibilityLabel={item.name}
            icon={item.icon || null}
            iconSize={44}
            showLabel={false}
            onClick={() => activateTask(item)}>{label}</Button>
        <Text color={item.windows.length ? "#17202bcc" : "transparent"}>•</Text>
    </div>;
}

function TrayItem({item}) {
    return <Button id={"cupertino-dock-tray-" + item.id}
        className="dock-utility"
        width={44} height={52}
        accessibilityLabel={item.title}
        icon={item.icon ? "tray:" + item.id : null}
        iconSize={30}
        showLabel={false}
        onContextMenu={() => nickel.tray.contextMenu(item.id)}
        onClick={() => nickel.tray.activate(item.id)}>{item.title.charAt(0).toUpperCase() || "?"}</Button>;
}

export function Taskbar() {
    const features = nickel.features?.get() || {keyboard: {}, codex: {}};
    const keyboardEnabled = features.keyboard.enabled === true;
    const codexAvailable = ["enabled", "enabling", "rejected", "stale"].includes(features.codex.state);
    const items = taskItems(nickel.applications.list(), nickel.windows.list());
    const tray = nickel.tray.list().slice(0, 4);
    const contributions = nickel.contributions("taskbar.items");
    const clock = nickel.clock.get();
    const localTime = new Date(clock.unixMilliseconds + clock.utcOffsetMinutes * 60000);
    const hours = localTime.getUTCHours();
    const clockText = (hours % 12 || 12) + ":" + String(localTime.getUTCMinutes()).padStart(2, "0");

    return <FixedWindow id="taskbar" output="all" edge="bottom" bottomOffset={10} width="max-content"
        reserveWorkArea={false} className="cupertino-dock">
        <div className="dock-shelf">
                <Button id="cupertino-dock-launcher" className="dock-button launcher"
                    width={58} height={58}
                    icon="nickel-logo" iconSize={44} showLabel={false}
                    accessibilityLabel="Open Nickel Launcher"
                    onClick={() => nickel.surfaces.show("launcher")}>Nickel</Button>
                {items.map(item => <DockItem key={item.id} item={item} />)}
                {contributions.map(entry => <entry.component key={entry.key} />)}
                <Text color="#26313d70">│</Text>
                {keyboardEnabled ? <Button id="cupertino-dock-keyboard" className="dock-utility"
                    width={44} height={52}
                    accessibilityLabel="On-screen keyboard" onClick={() => nickel.keyboard.toggle()}>⌨</Button> : null}
                {codexAvailable ? <Button id="cupertino-dock-codex" className="dock-utility"
                    width={44} height={52}
                    icon="codex" iconSize={30} showLabel={false} accessibilityLabel="Codex projects"
                    onClick={() => nickel.projects.toggle()}>C</Button> : null}
                {tray.map(item => <TrayItem key={item.id} item={item} />)}
                <Button id="cupertino-dock-control" className="dock-clock"
                    width={70} height={52}
                    accessibilityLabel="Open Quick Settings"
                    onClick={() => nickel.surfaces.show("quick-settings")}>{clockText}</Button>
        </div>
    </FixedWindow>;
}

export function Shell() {
    const id = nickel.data.surface?.id || "taskbar";
    const contract = id === "taskbar" ? "shell.taskbar"
        : id === "launcher" ? "shell.launcher"
        : id === "quick-settings" ? "shell.quickSettings"
        : id === "notifications" ? "shell.notifications"
        : id === "settings" ? "shell.settings"
        : id === "window-menu" ? "shell.windowMenu"
        : id === "volume-osd" ? "shell.volumeOSD"
        : id === "window-preview" ? "shell.window-preview"
        : id === "run" ? "shell.run"
        : id === "keyboard" ? "shell.keyboard"
        : "shell.taskbar";
    const Component = nickel.component(contract);
    return <Component />;
}

export default Shell;
