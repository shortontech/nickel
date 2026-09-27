// @jsx h
// This is the source for the bundled development panel. Run `tsc` with
// --allowJs --jsx react --jsxFactory h to regenerate main.js.
function App() {
    const [count, setCount] = useState(0);
    return h(Panel, { background: 0xc9262b36 },
        h(Row, null,
            h(Text, null, "Nickel plugin panel"),
            h(Button, { onClick: () => setCount(count + 1) },
                "Count: ",
                count)));
}
