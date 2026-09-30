// @jsx h
import "./styles/default-apps.css";

export function DefaultApps() {
    const catalog = nickel.associations.get();
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const targets = catalog.targets.filter(target => !needle || (target.id + " " + target.family + " " + target.handlers.map(handler => handler.name).join(" ")).toLowerCase().includes(needle));
    const result = catalog.lastResult;
    return <Column className="default-apps-page">
        <TextField id="default-app-search" accessibilityLabel="Search default applications" placeholder="Search applications or file types"
            value={query} onChange={setQuery} />
        {!catalog.available ? <Text wrap={true}>{catalog.reason || "Default applications are unavailable."}</Text> : null}
        {catalog.writable === false ? <Text>Default applications are read only.</Text> : null}
        {catalog.truncated ? <Text wrap={true}>This catalog is incomplete. Open system settings to see all associations.</Text> : null}
        {result ? <Column className="default-apps-result">
            <Text>{result.status === "applied" ? "Default application updated." : result.status === "nativeConsentRequired" ? "Confirm this change in system settings." : result.status === "rejected" ? "The change was rejected." : result.status === "opened" ? "System settings opened." : result.status}</Text>
            {result.detail ? <Text wrap={true}>{result.detail}</Text> : null}
        </Column> : null}
        {catalog.operations.openSystemSettings ? <Button id="default-app-system-settings" disabled={catalog.writable === false}
            onClick={() => nickel.associations.openSystemSettings()}>Open system default applications</Button> : null}
        {catalog.available && !targets.length ? <Text>No matching associations.</Text> : null}
        {targets.map((target, targetIndex) => <Column key={target.id} className="default-app-card">
            <Text className="default-app-title">{target.family + " · " + target.id}</Text>
            {target.detail ? <Text wrap={true}>{target.detail}</Text> : null}
            {target.protected ? <Text>This association is protected.</Text> : null}
            {target.capability === "nativeConsent" ? <Text wrap={true}>Changing this association requires confirmation in system settings.</Text> : null}
            {target.handlersTruncated ? <Text>More applications are available in system settings.</Text> : null}
            {target.handlers.map((handler, handlerIndex) => <Button key={handler.id}
                id={"default-app-handler-" + targetIndex + "-" + handlerIndex}
                className={handler.id === target.effectiveHandlerId ? "default-app-handler selected" : "default-app-handler"}
                state={handler.id === target.effectiveHandlerId ? "selected" : "unselected"}
                disabled={catalog.writable === false || !catalog.operations.setDefault || !target.canSetDefault || target.protected || handler.protected || handler.id === target.effectiveHandlerId}
                onClick={() => nickel.associations.setDefault(target.id, handler.id, catalog.revision)}>
                {handler.name + (handler.id === target.effectiveHandlerId ? " · Current" : "")}
            </Button>)}
        </Column>)}
    </Column>;
}

registerSettingsPage({id:"default-apps",group:"Applications",label:"Default applications",description:"Choose applications for file types and links",component:DefaultApps});
