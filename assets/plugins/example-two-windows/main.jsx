// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return <Window width={home ? 400 : 450} height={home ? 240 : 260} className="example-window">
        {home
            ? <Button id="reopen-details" onClick={() => nickel.request({
                type: "surface.show",
                id: "details",
            })}>Reopen details</Button>
            : <div className="details-content">
                <Text>Details window</Text>
                <Button id="close-details" onClick={() => nickel.request({
                    type: "surface.hide",
                    id: "details",
                })}>Close details</Button>
            </div>}
    </Window>;
}
