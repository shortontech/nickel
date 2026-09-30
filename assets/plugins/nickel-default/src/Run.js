// @jsx h
import "./styles/run.css";
export function Run() {
    const [command, setCommand] = useState("");
    const state = nickel.run.get();
    const submit = () => { if (command.trim() && state.available)
        nickel.run.execute(command); };
    return h(Window, { id: "run", width: 620, height: 180, className: "run-window", onSubmit: submit, onEscape: () => nickel.surfaces.hide("run") },
        h(Column, { className: "run-content" },
            h(Text, null, state.status || "Run command"),
            h(TextField, { autoFocus: true, id: "run-command", value: command, placeholder: "Program, file, folder, or command", onChange: value => setCommand(String(value).slice(0, 4096)) }),
            h(Row, { spacing: 8 },
                h(Button, { id: "run-execute", disabled: !state.available || !command.trim(), onClick: submit }, "Run"),
                h(Button, { id: "run-cancel", onClick: () => nickel.surfaces.hide("run") }, "Cancel"))));
}
