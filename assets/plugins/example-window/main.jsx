// @jsx h
function App() {
    const [open, setOpen] = useState(false);
    return <Window title="Window plugin" width={520} height={340}
        className="example-window" background={0xff202830}>
        <Box x={0} y={0} width={504} height={324} background={0xff202830}>
            <Column>
                <Text>Window plugin</Text>
                <Button id="open-dialog" onClick={() => {
                    setOpen(true);
                    nickel.openDialog("example-window-dialog");
                }}>Open dialog</Button>
                <Image asset="nickel-icon" width={48} height={48} accessibilityLabel="Nickel icon" />
            </Column>
        </Box>
        <Dialog id="example-window-dialog" anchor="open-dialog" open={open}
            onClose={() => setOpen(false)} width={340} height={140}>
            <Column>
                <Text>This dialog belongs to a component window.</Text>
                <Row>
                    <Button id="show-settings" onClick={() => {
                        setOpen(false);
                        nickel.request({type: "show-settings"});
                    }}>Open Settings</Button>
                    <Button id="dismiss" onClick={() => setOpen(false)}>Dismiss</Button>
                </Row>
            </Column>
        </Dialog>
    </Window>;
}
