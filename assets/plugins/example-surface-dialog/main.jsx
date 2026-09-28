// @jsx h
function App() {
    const dialog = nickel.data.surface.id === "confirm";
    return <Panel height={dialog ? 180 : 280} background={0xff202830}>
        {dialog
            ? <Column>
                <Text>This is a separate plugin dialog.</Text>
                <Button id="open-settings" onClick={() => nickel.request({type: "show-settings"})}>Open Settings</Button>
                <Button id="dismiss-dialog" onClick={() => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "confirm",
                })}>Dismiss</Button>
            </Column>
            : <Column>
                <Text>The dialog starts closed.</Text>
                <Button id="open-dialog" onClick={() => nickel.request({
                    type: "show-plugin-surface",
                    surfaceId: "confirm",
                })}>Open dialog</Button>
                <Button id="close-home" onClick={() => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "home",
                })}>Close home</Button>
            </Column>}
    </Panel>;
}
