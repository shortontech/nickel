// @jsx h
let showTitle = true;
registerSetting({id:"show-title",group:"Example shell",label:"Show shell title",type:"switch",defaultValue:true,
    value:() => showTitle,
    onChange:value => showTitle = value});

export function Taskbar() {
    const windows = nickel.windows.list();
    return <Panel className="example-taskbar">
        <Row>
            {showTitle !== false ? <Text>Example shell</Text> : null}
            {windows.map(window => <Button key={window.id} id={"example-window-" + window.id}
                disabled={window.canActivate === false} onClick={() => nickel.windows.activate(window.id)}>{window.title || "Window"}</Button>)}
            <Spacer />
            <Button id="example-quick-settings" onClick={() => nickel.surfaces.show("quick-settings")}>Quick Settings</Button>
            <Button id="example-settings" onClick={() => nickel.surfaces.show("settings")}>Settings</Button>
        </Row>
    </Panel>;
}

export function Shell() {
    const surface = nickel.data.surface || {};
    const Component = nickel.component(surface.id === "settings" ? "shell.settings"
        : surface.id === "quick-settings" ? "shell.quickSettings" : "shell.taskbar");
    return <Component />;
}
export default Shell;
