// @jsx h
// Window actions are projected by the host and revalidated when invoked.
function App() {
    const [page, setPage] = useState("root");
    const rows = nickel.data[page] || nickel.data.root;
    return h(Panel, { background: 0xf12b303c },
        h(Column, null,
            rows.map((row, index) => h(Button, {
                key: page + ":" + index,
                id: "window-menu-action-" + index,
                onClick: () => row.navigate
                    ? setPage(row.navigate)
                    : nickel.request({ type: "taskbar-window-menu-action", page, index })
            }, row.label))));
}
