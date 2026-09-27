// @jsx h
// The host executes commands and reports errors; this plugin owns the form.
function App() {
    const [command, setCommand] = useState("");
    const submit = () => {
        const trimmed = command.trim();
        if (trimmed) nickel.request({ type: "run-submit", command: trimmed });
    };
    return h(Panel, { height: 180, background: 0xf12b303c },
        h(Column, null,
            h(Text, null, nickel.data.status || "Run command"),
            h(TextField, { id: "run-command", value: command, placeholder: "Enter a command", onChange: value => setCommand(value.slice(0, 4096)) }),
            h(Row, null,
                h(Button, { id: "run-submit", onClick: submit }, "Run"),
                h(Button, { id: "run-cancel", onClick: () => nickel.request({ type: "run-dismiss" }) }, "Cancel"))));
}
