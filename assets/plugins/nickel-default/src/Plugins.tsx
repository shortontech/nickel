// @jsx h
import "./styles/plugins.css";

const pluginKey = plugin => plugin.id;
const settingKey = setting => setting.id;
const compositionKey = (_, index) => String(index);
const pluginHeight = plugin => Math.min(8192, 300 + plugin.composition.length * 28 + (plugin.settings || []).length * 128);
const settingHeight = setting => setting.description ? 140 : 100;
function settingType(setting) {
    const kind = setting.kind.kind;
    return kind === "boolean" ? "switch" : kind === "integer" ? "number" : kind === "choice" ? "select" : "text";
}

function memoryValue(value) {
    return value === null || value === undefined ? "Unavailable" : Math.round(value / 1024) + " KiB";
}

function PluginSetting({plugin, setting, revision, writable, draftScope}) {
    const Control = nickel.component("shell.settings.controls");
    const kind = setting.kind;
    const type = settingType(setting);
    return <Column className="plugin-setting">
        <Text>{setting.label}</Text>
        {setting.description ? <Text wrap={true}>{setting.description}</Text> : null}
        {writable ? <Control controlId={"plugin-setting/" + plugin.id + "/" + setting.id}
            draftState={draftScope.controls[setting.id]?.type === type ? draftScope.controls[setting.id].state : undefined}
            onDraftChange={next => {
                draftScope.controls[setting.id] = {type,state:next};
            }}
            setting={{id:setting.id,providerPackage:plugin.id,label:setting.label,type,value:setting.value,
                min:kind.min,max:kind.max,maxLength:kind.max_length,
                options:(kind.options || []).map(option => ({label:option,value:option})),
                onChange:value => nickel.plugins.setSetting(plugin.id, setting.id, value, revision)}} /> : <Text>{String(setting.value)}</Text>}
    </Column>;
}

function PluginCard({plugin, index, revision, writable, canManage, setReview, draftScope}) {
    return <Column className="plugin-card">
        <Row><Text className="plugin-title">{plugin.name}</Text><Spacer />
            <Button id={"plugin-toggle-" + index} disabled={!canManage}
                accessibilityLabel={(plugin.enabled ? "Disable " : "Enable ") + plugin.name}
                onClick={() => plugin.enabled ? nickel.plugins.disable(plugin.id, revision) : setReview({id:plugin.id,revision})}>
                {plugin.enabled ? "Disable" : "Enable"}
            </Button>
        </Row>
        <Text wrap={true}>{plugin.id + (plugin.version ? " · " + plugin.version : "") + (plugin.author ? " · " + plugin.author : "")}</Text>
        <Text wrap={true}>{"Status: " + plugin.health.state + (plugin.health.reason ? " · " + plugin.health.reason : "")}</Text>
        <Text wrap={true}>{"Authorized capabilities: " + (plugin.grants.length ? plugin.grants.join(", ") : "None")}</Text>
        <Text wrap={true}>{"Surfaces: " + (plugin.surfaces.length ? plugin.surfaces.join(", ") : "None")}</Text>
        {plugin.composition.length ? <VirtualColumn id={"plugin-composition/" + plugin.id}
            items={plugin.composition} itemKey={compositionKey} itemHeight={20} gap={8} overscan={96}
            renderItem={entry => <Text wrap={true}>{entry}</Text>} /> : null}
        <Text wrap={true}>{"JavaScript heap: " + memoryValue(plugin.memory.jsHeapBytes) + " · Native UI: " + memoryValue(plugin.memory.nativeUiBytes) + " · Textures: " + memoryValue(plugin.memory.textureBytes)}</Text>
        <Text wrap={true}>{"Tracked peak: " + memoryValue(plugin.memory.trackedPeakBytes) + " · Timers: " + plugin.memory.timers + " · Subscriptions: " + plugin.memory.subscriptions}</Text>
        {plugin.settings?.length ? <VirtualColumn id={"plugin-settings/" + plugin.id}
            items={plugin.settings} itemKey={settingKey} itemHeight={settingHeight} gap={8} overscan={96}
            renderItem={setting => <PluginSetting key={setting.id} plugin={plugin} setting={setting} revision={revision} writable={writable}
                draftScope={draftScope} />} /> : null}
        <Text wrap={true}>Memory reports tracked package resources. Per component and total process memory are unavailable.</Text>
    </Column>;
}

export function Plugins() {
    const catalog = nickel.plugins.get();
    const preview = catalog.shellPreview;
    const canManage = catalog.available && catalog.writable && !preview;
    const [query, setQuery] = useState("");
    const [review, setReview] = useState(null);
    const draftScopes = useRef(Object.create(null));
    // Draft ownership outlives materialized cards, but removed plugin/setting
    // identities and changed editor types must retire their state offscreen too.
    useMemo(() => {
        const present = Object.create(null);
        for (const plugin of catalog.plugins) {
            present[plugin.id] = true;
            const scope = draftScopes.current[plugin.id] || (draftScopes.current[plugin.id] = {
                controls:Object.create(null),
            });
            const paths = Object.create(null);
            for (const setting of plugin.settings || []) {
                const type = settingType(setting);
                paths[setting.id] = type;
            }
            for (const id of Object.keys(scope.controls)) if (paths[id] !== scope.controls[id].type) delete scope.controls[id];
        }
        for (const id of Object.keys(draftScopes.current)) if (!present[id]) delete draftScopes.current[id];
    }, [catalog.plugins]);
    const candidate = review && catalog.plugins.find(plugin => plugin.id === review.id);
    const reviewCurrent = candidate && !candidate.enabled && review.revision === catalog.revision;
    const needle = query.trim().toLowerCase();
    const plugins = useMemo(() => catalog.plugins.filter(plugin => !plugin.shell && (plugin.name + " " + plugin.id).toLowerCase().includes(needle)), [catalog.plugins, needle]);
    return <Column className="plugins-page">
        <TextField id="plugins-search" accessibilityLabel="Search plugins" placeholder="Search plugins" value={query} onChange={setQuery} />
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Plugin inventory unavailable."}</Text> : null}
        {catalog.available && !catalog.writable ? <Text>Plugin management is read only.</Text> : null}
        {catalog.truncated ? <Text wrap={true}>This inventory is incomplete.</Text> : null}
        {catalog.lastResult ? <Text wrap={true}>{catalog.lastResult.detail || ({applied:"Plugin state updated.",preview:"Shell preview started.",confirmed:"Shell selection saved.",reverted:"Previous shell restored.",rejected:"Plugin change rejected."}[catalog.lastResult.status] || "Plugin operation: " + catalog.lastResult.status)}</Text> : null}
        {preview ? <Text wrap={true}>Finish the temporary shell preview in the Shell setting before changing extensions.</Text> : null}
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
                    onClick={() => { nickel.plugins.enable(candidate.id, review.revision); setReview(null); }}>Enable plugin</Button></Row>
        </Column> : null}
        {catalog.available && !plugins.length ? <Text>No matching extensions.</Text> : null}
        <VirtualColumn id="settings-plugins" items={plugins} itemKey={pluginKey}
            itemHeight={pluginHeight} gap={12} overscan={96}
            renderItem={(plugin, index) => <PluginCard key={plugin.id} plugin={plugin} index={index}
                revision={catalog.revision} writable={catalog.writable} canManage={canManage}
                setReview={setReview} draftScope={draftScopes.current[plugin.id]} />} />
    </Column>;
}

registerSettingsPage({id:"plugins",group:"Shell",label:"Plugins",description:"Inspect extension status, capabilities and memory; enable or disable packages",component:Plugins});
