// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return <Panel height={home ? 240 : 260} background={0xff202830}>
        {home
            ? <Button id="reopen-details" onClick={() => nickel.request({
                type: "show-plugin-surface",
                surfaceId: "details",
            })}>Reopen details</Button>
            : <Text>Details window</Text>}
    </Panel>;
}
