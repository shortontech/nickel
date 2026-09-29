// @jsx h
function App() {
    const [open, setOpen] = useState(false);
    const openCount = nickel.data.settings?.["open-count"] ?? 0;
    return h(FixedWindow, { width: 320, height: 120, className: "dialog-example" },
        h(Button, { id: "settings-example", onClick: () => {
                setOpen(true);
                nickel.openDialog("settings-example-dialog");
            } }, "Open a dialog"),
        h(Dialog, { id: "settings-example-dialog", anchor: "settings-example", open: open, onClose: () => setOpen(false), width: 320, height: 120 },
            h(Column, null,
                h(Text, null,
                    "Opened ",
                    openCount,
                    " times"),
                h(Row, null,
                    h(Button, { id: "save-count", onClick: () => {
                            nickel.request({
                                type: "set-plugin-setting",
                                key: "open-count",
                                value: Math.min(99, openCount + 1),
                            });
                        } }, "Save count"),
                    h(Button, { id: "confirm-settings", onClick: () => {
                            setOpen(false);
                            nickel.request({ type: "show-settings" });
                        } }, "Open Settings"),
                    h(Button, { id: "cancel-settings", onClick: () => setOpen(false) }, "Cancel")))));
}
