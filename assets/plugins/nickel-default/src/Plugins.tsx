// @jsx h
import "./styles/plugins.css";

function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}

function PluginSetting({plugin, setting, revision, writable}) {
    const Control = nickel.component("shell.settings.controls");
    const kind = setting.kind;
    const type = kind.kind === "boolean" ? "switch" : kind.kind === "integer" ? "number" : kind.kind === "choice" ? "select" : "text";
    return <Column className="plugin-setting">
        <Text>{setting.label}</Text>
        {setting.description ? <Text wrap={true}>{setting.description}</Text> : null}
        {writable ? <Control controlId={"plugin-setting/" + plugin.id + "/" + setting.id}
            setting={{id:setting.id,providerPackage:plugin.id,label:setting.label,type,value:setting.value,
                min:kind.min,max:kind.max,maxLength:kind.max_length,
                options:(kind.options || []).map(option => ({label:option,value:option})),
                onChange:value => nickel.plugins.setSetting(plugin.id, setting.id, value, revision)}} /> : <Text>{String(setting.value)}</Text>}
    </Column>;
}

export function Plugins() {
    const catalog = nickel.plugins.get();
    const preview = catalog.shellPreview;
    const canManage = catalog.available && catalog.writable && !preview;
    const shellName = id => catalog.plugins.find(plugin => plugin.id === id)?.name || id;
    const [query, setQuery] = useState("");
    const [review, setReview] = useState(null);
    const candidate = review && catalog.plugins.find(plugin => plugin.id === review.id);
    const reviewCurrent = candidate && !candidate.enabled && review.revision === catalog.revision;
    const needle = query.trim().toLowerCase();
    const plugins = catalog.plugins.filter(plugin => (plugin.name + " " + plugin.id).toLowerCase().includes(needle));
    return <Column className="plugins-page">
        <TextField id="plugins-search" accessibilityLabel="Search plugins" placeholder="Search plugins" value={query} onChange={setQuery} />
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Plugin inventory unavailable."}</Text> : null}
        {catalog.available && !catalog.writable ? <Text>Plugin management is read only.</Text> : null}
        {catalog.truncated ? <Text wrap={true}>This inventory is incomplete.</Text> : null}
        {catalog.lastResult ? <Text wrap={true}>{catalog.lastResult.detail || ({applied:"Plugin state updated.",preview:"Shell preview started.",confirmed:"Shell selection saved.",reverted:"Previous shell restored.",rejected:"Plugin change rejected."}[catalog.lastResult.status] || "Plugin operation: " + catalog.lastResult.status)}</Text> : null}
        {preview ? <Column className="plugin-card">
            <Text className="plugin-title">Temporary shell preview</Text>
            <Text wrap={true}>{"Previewing " + shellName(preview.selectedShell) + ". Previous shell: " + shellName(preview.previousShell) + "."}</Text>
            <Text wrap={true}>Keep this shell before the recovery timer expires, or restore the previous shell. Unconfirmed previews revert automatically.</Text>
            <Row>
                <Button id="plugin-shell-confirm" disabled={!catalog.available || !catalog.writable || !preview.canConfirm}
                    onClick={() => nickel.plugins.confirmShell(preview.token, catalog.revision)}>Keep this shell</Button>
                <Button id="plugin-shell-revert" disabled={!catalog.available || !catalog.writable || !preview.canRevert}
                    onClick={() => nickel.plugins.revertShell(preview.token, catalog.revision)}>Restore previous shell</Button>
            </Row>
            <Text wrap={true}>Finish this preview before changing plugin activation or selecting another shell.</Text>
        </Column> : null}
        {review ? <Column className="plugin-card">
            <Text className="plugin-title">Review plugin access</Text>
            {candidate ? <Column>
                <Text>{candidate.name + " · " + (candidate.version || "Unspecified version")}</Text>
                <Text>{"Publisher: " + (candidate.author || "Unknown")}</Text>
                <Text wrap={true}>{"Capabilities requested: " + (candidate.grants.length ? candidate.grants.join(", ") : "None")}</Text>
                <Text wrap={true}>{"Surfaces affected: " + (candidate.surfaces.length ? candidate.surfaces.join(", ") : "None")}</Text>
                <Text wrap={true}>{"Composition changes: " + (candidate.composition.length ? candidate.composition.join(", ") : "None")}</Text>
            </Column> : null}
            {!reviewCurrent ? <Text wrap={true}>Plugin access changed. Cancel and review it again before enabling.</Text> : null}
            <Row><Button id="plugin-review-cancel" onClick={() => setReview(null)}>Cancel</Button>
                <Button id="plugin-review-confirm" disabled={!canManage || !reviewCurrent}
                    onClick={() => { review.selectShell ? nickel.plugins.selectShell(candidate.id, review.revision) : nickel.plugins.enable(candidate.id, review.revision); setReview(null); }}>{review.selectShell ? "Enable and preview shell" : "Enable plugin"}</Button></Row>
        </Column> : null}
        {catalog.available && !plugins.length ? <Text>No matching plugins.</Text> : null}
        {plugins.map((plugin, index) => <Column key={plugin.id} className="plugin-card">
            <Row><Text className="plugin-title">{plugin.name}</Text><Spacer />
                <Button id={"plugin-toggle-" + index} disabled={!canManage}
                    accessibilityLabel={(plugin.enabled ? "Disable " : "Enable ") + plugin.name}
                    onClick={() => plugin.enabled ? nickel.plugins.disable(plugin.id, catalog.revision) : setReview({id:plugin.id,revision:catalog.revision})}>
                    {plugin.enabled ? "Disable" : "Enable"}
                </Button>
            </Row>
            {plugin.shell ? plugin.selected ? <Text>Selected shell</Text> : <Button id={"plugin-preview-shell-" + index}
                disabled={!canManage} onClick={() => plugin.enabled ? nickel.plugins.selectShell(plugin.id, catalog.revision) : setReview({id:plugin.id,revision:catalog.revision,selectShell:true})}>Preview shell</Button> : null}
            <Text wrap={true}>{plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")}</Text>
            <Text wrap={true}>{"Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")}</Text>
            <Text wrap={true}>{"Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")}</Text>
            <Text wrap={true}>{"Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")}</Text>
            {plugin.composition.map((entry, entryIndex) => <Text key={entryIndex} wrap={true}>{entry}</Text>)}
            <Text wrap={true}>{"JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)}</Text>
            <Text wrap={true}>{"Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions}</Text>
            {(plugin.settings || []).map(setting => <PluginSetting key={setting.id} plugin={plugin} setting={setting} revision={catalog.revision} writable={catalog.writable} />)}
            <Text wrap={true}>Memory reports tracked package resources. Per component and total process memory are unavailable.</Text>
        </Column>)}
    </Column>;
}

registerSettingsPage({id:"plugins",group:"Shell",label:"Plugins",description:"Inspect plugin status, capabilities and memory; enable or disable packages",component:Plugins});
