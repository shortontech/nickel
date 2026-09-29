// @jsx h
// Window actions are projected by the host and revalidated when invoked.
function App() {
    const [page, setPage] = useState("root");
    const rows = nickel.data[page] || nickel.data.root;
    return h(FixedWindow, { width: "100%", height: "100%", className: "taskbar-menu" },
        h(Column, null, rows.map((row, index) => h(Button, { key: page + ":" + index, id: "window-menu-action-" + index, className: "taskbar-menu-button", onClick: () => row.navigate
                ? setPage(row.navigate)
                : nickel.request({ type: "taskbar-window-menu-action", page, index }) }, row.label))));
}
