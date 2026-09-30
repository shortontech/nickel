// @jsx h
function App() {
    const home = nickel.data.surface.id === "home";
    return <Window width={home ? 400 : 450} height={home ? 240 : 260} className="example-window">
        {home
            ? <div className="details-content">
                <Button id="reopen-details" onClick={() => nickel.request({
                    type: "surface.show",
                    id: "details",
                })}>Reopen details</Button>
                <Button id="focus-details" onClick={() => nickel.request({
                    type: "surface.focus",
                    id: "details",
                })}>Focus details</Button>
            </div>
            : <div className="details-content">
                <Text>Details window</Text>
                <Button id="close-details" onClick={() => nickel.request({
                    type: "surface.hide",
                    id: "details",
                })}>Close details</Button>
            </div>}
    </Window>;
}
