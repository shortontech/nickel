// @jsx h
import "./styles/quick-settings.css";
// Nickel owns status snapshots and validates every requested system action.
export function QuickSettings(props) {
    const data = {...{scrollHeight:552,network:{available:false,enabled:false,networks:[]},bluetooth:{available:false,powered:false,discovering:false,devices:[]},audio:{muted:false,percent:0,devices:[]},workspaces:[],projectionModes:[],slots:{}}, ...props?.data, audio:props?.data?.audio || nickel.audio.get(), network:nickel.wifi.get(), bluetooth:nickel.bluetooth.get()};
    const sections = (data.slots && data.slots["control-section"]) || [];
    const [wifiOpen, setWifiOpen] = useState(false);
    const [bluetoothOpen, setBluetoothOpen] = useState(false);
    const [audioOpen, setAudioOpen] = useState(false);
    const [confirming, setConfirming] = useState(null);
    const request = (action, value) => nickel.request({type: "control-action", action, value});
    const prepare = action => {
        setConfirming(action);
        request("session-prepare", action);
        nickel.openDialog("session-confirm-dialog");
    };
    return <FixedWindow id="quick-settings" edge="right" width={420} height="100%" className="control-center"
        onEscape={() => nickel.surfaces.hide("quick-settings")}>
        <Column className="control-content">
            <Text className="control-title">Control Center</Text>
            <ScrollView id="control-center-scroll" height={data.scrollHeight}>
                <Column className="control-sections">
                    <Row>
                        <Text>{"Wi-Fi: " + (data.network.available ? data.network.enabled ? "On" : "Off" : "Unavailable")}</Text>
                        {data.network.operations.setEnabled ? <Button id="wifi-power" onClick={() => nickel.wifi.setEnabled(!data.network.enabled)}>{data.network.enabled ? "Turn off" : "Turn on"}</Button> : null}
                        <Button id="wifi-section" onClick={() => setWifiOpen(!wifiOpen)}>{wifiOpen ? "Less" : "More"}</Button>
                    </Row>
                    {wifiOpen ? data.network.networks.map(network => <Button key={network.id}
                        id={"wifi-" + network.id} disabled={!network.canConnect} onClick={() => nickel.wifi.connect(network.id)}>
                        {network.name + (network.connected ? " · Connected" : network.saved ? " · Saved" : "")}
                    </Button>) : null}
                    <Row>
                        <Text>{"Bluetooth: " + (data.bluetooth.available ? data.bluetooth.powered ? "On" : "Off" : "Unavailable")}</Text>
                        {data.bluetooth.operations.setPowered ? <Button id="bluetooth-power" onClick={() => nickel.bluetooth.setPowered(!data.bluetooth.powered)}>{data.bluetooth.powered ? "Turn off" : "Turn on"}</Button> : null}
                        <Button id="bluetooth-section" onClick={() => setBluetoothOpen(!bluetoothOpen)}>{bluetoothOpen ? "Less" : "More"}</Button>
                    </Row>
                    {bluetoothOpen ? <Column>
                        {data.bluetooth.operations.setDiscovery && data.bluetooth.powered ? <Button id="bluetooth-scan"
                            onClick={() => nickel.bluetooth.setDiscovery(!data.bluetooth.discovering)}>{data.bluetooth.discovering ? "Stop scan" : "Scan nearby"}</Button> : null}
                        {data.bluetooth.devices.map(device => <Button key={device.id} id={"bluetooth-" + device.id}
                            disabled={!data.bluetooth.powered || !(device.paired ? data.bluetooth.operations[device.connected ? "disconnect" : "connect"] : data.bluetooth.operations.pair)}
                            onClick={() => device.paired ? device.connected ? nickel.bluetooth.disconnect(device.id) : nickel.bluetooth.connect(device.id) : nickel.bluetooth.pair(device.id)}>
                            {device.name + (device.connected ? " · Connected" : device.paired ? " · Paired" : "")}
                        </Button>)}
                    </Column> : null}
                    <Row>
                        <Text>{"Audio: " + (data.audio.muted ? "Muted" : data.audio.percent + "%")}</Text>
                        <Button id="audio-mute" onClick={() => nickel.audio.setMuted(!data.audio.muted)}>{data.audio.muted ? "Unmute" : "Mute"}</Button>
                        <Button id="audio-section" onClick={() => setAudioOpen(!audioOpen)}>{audioOpen ? "Less" : "More"}</Button>
                    </Row>
                    <Row>
                        <Button id="audio-down" onClick={() => nickel.audio.setVolume(Math.max(0, data.audio.percent - 10))}>−</Button>
                        <Progress className="audio-progress" percent={data.audio.percent} width={220} height={8} />
                        <Button id="audio-up" onClick={() => nickel.audio.setVolume(Math.min(100, data.audio.percent + 10))}>+</Button>
                    </Row>
                    {audioOpen ? data.audio.devices.map(device => <Button key={device.id} id={"audio-" + device.id}
                        onClick={() => nickel.audio.selectOutput(device.id)}>{device.name + (device.isDefault ? " · Default" : "")}</Button>) : null}
                    <Text className="control-section-title">Workspaces</Text>
                    <Row>
                        {data.workspaces.map((workspace, index) => <Button key={workspace.id} id={"workspace-" + workspace.id}
                            onClick={() => request("workspace-switch", workspace.id)}>{(index + 1) + (workspace.active ? " ●" : "")}</Button>)}
                        <Button id="workspace-create" onClick={() => request("workspace-create")}>+</Button>
                        {data.workspaces.length > 1 ? <Button id="workspace-remove"
                            onClick={() => request("workspace-remove", data.activeWorkspace)}>−</Button> : null}
                    </Row>
                    <Row>
                        <Button id="show-desktop" onClick={() => request("show-desktop")}>Show desktop</Button>
                        <Button id="show-notifications" onClick={() => nickel.surfaces.show("notifications")}>Notifications</Button>
                    </Row>
                    {sections.length ? <Text className="control-section-title">Extensions</Text> : null}
                    {sections.map((section, index) => <Row key={`${section.pluginId}:${section.id}`}>
                        <Text>{section.label + ": " + section.value}</Text>
                        <Button id={`control-extension-${index}`} onClick={() => nickel.request({
                            type: "invoke-plugin-slot-section", slot: "control-section",
                            pluginId: section.pluginId, id: section.id
                        })}>Open</Button>
                    </Row>)}
                    <Text className="control-section-title">Displays</Text>
                    {data.pendingProjection ? <Row>
                        <Text>Keep display settings?</Text>
                        <Button id="projection-revert" onClick={() => request("projection-cancel")}>Revert</Button>
                        <Button id="projection-keep" onClick={() => request("projection-confirm")}>Keep</Button>
                    </Row> : <Row>
                        {data.projectionModes.map(mode => <Button key={mode.id} id={"projection-" + mode.id}
                            onClick={() => request("projection-preview", mode.id)}>{mode.label}</Button>)}
                    </Row>}
                    <Text className="control-section-title">Session</Text>
                    <Row>
                        <Button id="session-lock" onClick={() => request("session-lock")}>Lock</Button>
                        <Button id="session-suspend" onClick={() => prepare("suspend")}>Suspend</Button>
                        <Button id="session-logout" onClick={() => prepare("logout")}>Log out</Button>
                    </Row>
                    <Row>
                        <Button id="session-restart-shell" onClick={() => prepare("restart-shell")}>Restart Nickel</Button>
                        <Button id="session-reboot" onClick={() => prepare("reboot")}>Restart PC</Button>
                        <Button id="session-poweroff" onClick={() => prepare("poweroff")}>Shut down</Button>
                    </Row>
                </Column>
            </ScrollView>
            <Dialog id="session-confirm-dialog" anchor={"session-" + confirming}
                open={confirming !== null} onClose={() => setConfirming(null)} width={320} height={128}>
                <Column>
                    <Text>{"Confirm " + (confirming || "action") + "?"}</Text>
                    <Row>
                        <Button id="session-cancel" onClick={() => { setConfirming(null); request("session-cancel"); }}>Cancel</Button>
                        <Button id="session-confirm" onClick={() => { request("session-confirm"); setConfirming(null); }}>Confirm</Button>
                    </Row>
                </Column>
            </Dialog>
        </Column>
    </FixedWindow>;
}
