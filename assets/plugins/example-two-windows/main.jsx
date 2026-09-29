// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return <Window width={home ? 400 : 450} height={home ? 240 : 260} className="example-window">
        {home
            ? <Button id="reopen-details" onClick={() => nickel.request({
                type: "show-plugin-surface",
                surfaceId: "details",
            })}>Reopen details</Button>
            : <div className="details-content">
                <Text>Details window</Text>
                <Button id="close-details" onClick={() => nickel.request({
                    type: "hide-plugin-surface",
                    surfaceId: "details",
                })}>Close details</Button>
            </div>}
    </Window>;
}
