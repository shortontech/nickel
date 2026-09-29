// @jsx h
function App() {
    const overlay = nickel.data.surface.id === "notice";
    return <Window width={overlay ? 300 : 420} height={overlay ? 120 : 250}
        placement={overlay ? "fixed" : "managed"} className="example-overlay-window"
        background={overlay ? 0xb0202830 : 0xff202830}>
        {overlay
            ? <Column>
                <Text>This overlay is a separate surface.</Text>
                <Button id="hide-overlay" onClick={() => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "notice",
                })}>Close overlay</Button>
            </Column>
            : <Column>
                <Text>The overlay starts closed.</Text>
                <Button id="show-overlay" onClick={() => nickel.request({
                    type: "show-plugin-surface",
                    surfaceId: "notice",
                })}>Show overlay</Button>
            </Column>}
    </Window>;
}
