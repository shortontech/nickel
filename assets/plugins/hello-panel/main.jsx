/// <reference path="../nickel-plugin.d.ts" />
// @jsx h
// This is the source for the bundled development panel. Run `tsc` with
// --allowJs --jsx react --jsxFactory h to regenerate main.js.
function App() {
    const [count, setCount] = useState(0);
    const [dialogOpen, setDialogOpen] = useState(false);
    const showCount = nickel.data.settings?.["show-count"] !== false;
    return <Panel background={0xc9262b36}>
        <Row>
            <Text>Nickel plugin panel</Text>
            {showCount ? <Button onClick={() => setCount(count + 1)}>Count: {count}</Button> : null}
            <Button id="open-dialog" onClick={() => {
                setDialogOpen(true);
                nickel.openDialog("launcher-dialog");
            }}>Open dialog</Button>
        </Row>
        <Dialog id="launcher-dialog" anchor="open-dialog" open={dialogOpen} onClose={() => setDialogOpen(false)} width={320} height={120}>
            <Row>
                <Text>Show the launcher?</Text>
                <Button onClick={() => {
                    setDialogOpen(false);
                    nickel.request("show-launcher");
                }}>Show</Button>
                <Button onClick={() => setDialogOpen(false)}>Cancel</Button>
            </Row>
        </Dialog>
    </Panel>;
}
