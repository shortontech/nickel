// @jsx h
import "./styles/run.css";
export function Run() {
    const [command, setCommand] = useState("");
    const state = nickel.run.get();
    const submit = () => { if (command.trim() && state.available) nickel.run.execute(command); };
    return <Window id="run" width={620} height={180} className="run-window" onSubmit={submit} onEscape={() => nickel.surfaces.hide("run")}>
        <Column className="run-content">
            <Text>{state.status || "Run command"}</Text>
            <TextField autoFocus={true} id="run-command" value={command} placeholder="Program, file, folder, or command" onChange={value => setCommand(String(value).slice(0, 4096))} />
            <Row spacing={8}>
                <Button id="run-execute" disabled={!state.available || !command.trim()} onClick={submit}>Run</Button>
                <Button id="run-cancel" onClick={() => nickel.surfaces.hide("run")}>Cancel</Button>
            </Row>
        </Column>
    </Window>;
}
