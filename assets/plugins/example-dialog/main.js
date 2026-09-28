// @jsx h
function App() {
    const [open, setOpen] = useState(false);
    return h(Panel, { height: 120, background: 0xdd202830 },
        h(Button, { id: "settings-example", onClick: () => {
                setOpen(true);
                nickel.openDialog("settings-example-dialog");
            } }, "Open a dialog"),
        h(Dialog, { id: "settings-example-dialog", anchor: "settings-example", open: open, onClose: () => setOpen(false), width: 320, height: 120 },
            h(Column, null,
                h(Text, null, "Open Nickel Settings?"),
                h(Row, null,
                    h(Button, { id: "confirm-settings", onClick: () => {
                            setOpen(false);
                            nickel.request({ type: "show-settings" });
                        } }, "Open Settings"),
                    h(Button, { id: "cancel-settings", onClick: () => setOpen(false) }, "Cancel")))));
}
