// @jsx h
function App() {
    const dialog = nickel.data.surface.id === "confirm";
    return <Window width={dialog ? 360 : 420} height={dialog ? 180 : 280}
        className="example-dialog-window" background={0xff202830}>
        {dialog
            ? <Column>
                <Text>This is a separate plugin dialog.</Text>
                <Button id="open-settings" onClick={() => nickel.request({type: "show-settings"})}>Open Settings</Button>
                <Button id="dismiss-dialog" onClick={() => nickel.request({
                    type: "surface.hide",
                    id: "confirm",
                })}>Dismiss</Button>
            </Column>
            : <Column>
                <Text>The dialog starts closed.</Text>
                <Button id="open-dialog" onClick={() => nickel.request({
                    type: "surface.show",
                    id: "confirm",
                })}>Open dialog</Button>
                <Button id="close-home" onClick={() => nickel.request({
                    type: "surface.hide",
                    id: "home",
                })}>Close home</Button>
            </Column>}
    </Window>;
}
