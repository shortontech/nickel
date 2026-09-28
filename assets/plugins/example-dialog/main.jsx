// @jsx h
function App() {
    const [open, setOpen] = useState(false);
    const openCount = nickel.data.settings?.["open-count"] ?? 0;
    return <Panel height={120} background={0xdd202830}>
        <Button id="settings-example" onClick={() => {
            setOpen(true);
            nickel.openDialog("settings-example-dialog");
        }}>Open a dialog</Button>
        <Dialog id="settings-example-dialog" anchor="settings-example" open={open}
            onClose={() => setOpen(false)} width={320} height={120}>
            <Column>
                <Text>Opened {openCount} times</Text>
                <Row>
                    <Button id="save-count" onClick={() => {
                        nickel.request({
                            type: "set-plugin-setting",
                            key: "open-count",
                            value: Math.min(99, openCount + 1),
                        });
                    }}>Save count</Button>
                    <Button id="confirm-settings" onClick={() => {
                        setOpen(false);
                        nickel.request({type: "show-settings"});
                    }}>Open Settings</Button>
                    <Button id="cancel-settings" onClick={() => setOpen(false)}>Cancel</Button>
                </Row>
            </Column>
        </Dialog>
    </Panel>;
}
