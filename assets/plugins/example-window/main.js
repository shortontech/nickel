// @jsx h
function App() {
    const [open, setOpen] = useState(false);
    return h(Window, { title: "Window plugin", width: 520, height: 340, className: "example-window", background: 0xff202830 },
        h(Layer, { className: "example-layout" },
            h(Box, { x: 0, y: 0, width: 504, height: 324, background: 0xff202830 },
                h(Column, null,
                    h(Text, null, "Window plugin"),
                    h(Button, { id: "open-dialog", onClick: () => {
                            setOpen(true);
                            nickel.openDialog("example-window-dialog");
                        } }, "Open dialog"),
                    h(Image, { asset: "nickel-icon", width: 48, height: 48, accessibilityLabel: "Nickel icon" })))),
        h(Dialog, { id: "example-window-dialog", anchor: "open-dialog", open: open, onClose: () => setOpen(false), width: 340, height: 140 },
            h(Column, null,
                h(Text, null, "This dialog belongs to a component window."),
                h(Row, null,
                    h(Button, { id: "show-settings", onClick: () => {
                            setOpen(false);
                            nickel.request({ type: "show-settings" });
                        } }, "Open Settings"),
                    h(Button, { id: "dismiss", onClick: () => setOpen(false) }, "Dismiss")))));
}
