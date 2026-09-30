// @jsx h
import "./styles/plugins.css";

function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}

export function Plugins() {
    const catalog = nickel.plugins.get();
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const plugins = catalog.plugins.filter(plugin => (plugin.name + " " + plugin.id).toLowerCase().includes(needle));
    return <Column className="plugins-page">
        <TextField id="plugins-search" accessibilityLabel="Search plugins" placeholder="Search plugins" value={query} onChange={setQuery} />
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Plugin inventory unavailable."}</Text> : null}
        {catalog.available && !catalog.writable ? <Text>Plugin management is read only.</Text> : null}
        {catalog.truncated ? <Text wrap={true}>This inventory is incomplete.</Text> : null}
        {catalog.lastResult ? <Text wrap={true}>{catalog.lastResult.status === "applied" ? "Plugin state updated." : catalog.lastResult.detail || "Plugin change rejected."}</Text> : null}
        {catalog.available && !plugins.length ? <Text>No matching plugins.</Text> : null}
        {plugins.map((plugin, index) => <Column key={plugin.id} className="plugin-card">
            <Row><Text className="plugin-title">{plugin.name}</Text><Spacer />
                <Button id={"plugin-toggle-" + index} disabled={!catalog.writable}
                    accessibilityLabel={(plugin.enabled ? "Disable " : "Enable ") + plugin.name}
                    onClick={() => plugin.enabled ? nickel.plugins.disable(plugin.id, catalog.revision) : nickel.plugins.enable(plugin.id, catalog.revision)}>
                    {plugin.enabled ? "Disable" : "Enable"}
                </Button>
            </Row>
            <Text wrap={true}>{plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")}</Text>
            <Text>{"Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")}</Text>
            <Text wrap={true}>{"Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")}</Text>
            <Text wrap={true}>{"Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")}</Text>
            {plugin.composition.map((entry, entryIndex) => <Text key={entryIndex} wrap={true}>{entry}</Text>)}
            <Text wrap={true}>{"JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)}</Text>
            <Text wrap={true}>{"Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions}</Text>
            <Text wrap={true}>Memory reports tracked package resources. Per component and total process memory are unavailable.</Text>
        </Column>)}
    </Column>;
}

registerSettingsPage({id:"plugins",group:"Shell",label:"Plugins",description:"Inspect plugin status, capabilities and memory; enable or disable packages",component:Plugins});
