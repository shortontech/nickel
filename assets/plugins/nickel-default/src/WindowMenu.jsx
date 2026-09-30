// @jsx h
import "./styles/window-menu.css";

// Native window facts and permitted operations remain owned by the window service.
export function WindowMenu() {
    const target = nickel.windows.menu().targetId;
    const window = nickel.windows.list().find(item => item.id === target);
    const destinations = nickel.windows.destinations();
    const dismiss = () => nickel.windows.dismissMenu();
    const act = operation => { operation(window.id); dismiss(); };
    return <FixedWindow id="window-menu" width={320} height={400} className="window-menu" onEscape={dismiss}>
        <ScrollView id="window-menu-scroll" height={368}><Column>
            <Text className="window-menu-title">{window?.title || "Window unavailable"}</Text>
            {window ? <Column>
                {window.canActivate ? <Button id="window-activate" onClick={() => act(nickel.windows.activate)}>{window.minimized ? "Restore" : "Activate"}</Button> : null}
                {window.canMinimize && !window.minimized ? <Button id="window-minimize" onClick={() => act(nickel.windows.minimize)}>Minimize</Button> : null}
                {window.canMaximize ? <Button id="window-maximize" onClick={() => act(nickel.windows.toggleMaximize)}>{window.maximized ? "Restore size" : "Maximize"}</Button> : null}
                {window.canSnap ? <Row><Button id="window-snap-leading" onClick={() => act(nickel.windows.snapLeading)}>Snap left</Button><Button id="window-snap-trailing" onClick={() => act(nickel.windows.snapTrailing)}>Snap right</Button></Row> : null}
                {window.canFullscreen ? <Button id="window-fullscreen" onClick={() => act(nickel.windows.toggleFullscreen)}>{window.fullscreen ? "Leave fullscreen" : "Fullscreen"}</Button> : null}
                {window.canMoveToWorkspace ? destinations.workspaces.filter(item => item.id !== String(window.workspace)).map(item => <Button key={item.id} id={"window-workspace-" + item.id} onClick={() => { nickel.windows.moveToWorkspace(window.id, item.id); dismiss(); }}>{"Move to " + item.name}</Button>) : null}
                {window.canMoveToOutput ? destinations.outputs.filter(output => output !== window.output).map(output => <Button key={output} id={"window-output-" + output} onClick={() => { nickel.windows.moveToOutput(window.id, output); dismiss(); }}>{"Move to " + output}</Button>) : null}
                {window.canClose ? <Button id="window-close" onClick={() => act(nickel.windows.close)}>Close window</Button> : null}
            </Column> : null}
            <Button id="window-menu-dismiss" onClick={dismiss}>Cancel</Button>
        </Column></ScrollView>
    </FixedWindow>;
}
