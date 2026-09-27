/// <reference path="../nickel-plugin.d.ts" />
// @jsx h
// This is the source for the bundled development panel. Run `tsc` with
// --allowJs --jsx react --jsxFactory h to regenerate main.js.
function App() {
    const [count, setCount] = useState(0);
    const [dialogOpen, setDialogOpen] = useState(false);
    const showCount = nickel.data.settings?.["show-count"] !== false;
    return h(Panel, { background: 0xc9262b36 },
        h(Row, null,
            h(Text, null, "Nickel plugin panel"),
            showCount ? h(Button, { onClick: () => setCount(count + 1) },
                "Count: ",
                count) : null,
            h(Button, { id: "open-dialog", onClick: () => {
                    setDialogOpen(true);
                    nickel.openDialog("launcher-dialog");
                } }, "Open dialog")),
        h(Dialog, { id: "launcher-dialog", anchor: "open-dialog", open: dialogOpen, onClose: () => setDialogOpen(false), width: 320, height: 120 },
            h(Row, null,
                h(Text, null, "Show the launcher?"),
                h(Button, { onClick: () => {
                        setDialogOpen(false);
                        nickel.request("show-launcher");
                    } }, "Show"),
                h(Button, { onClick: () => setDialogOpen(false) }, "Cancel"))));
}
