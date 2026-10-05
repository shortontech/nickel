// @jsx h
import "./styles/default-apps.css";
const associationKey = item => item.id;
const associationHeight = target => 66 + Math.max(0, target.handlers.length * 40 - 8)
    + (target.detail ? 32 : 0) + (target.protected ? 28 : 0)
    + (target.capability === "nativeConsent" ? 48 : 0) + (target.handlersTruncated ? 28 : 0);
let nextHandlerList = 0;
function AssociationCard({ target, catalog }) {
    // Compact mount-local names avoid embedding long target IDs in collection IDs.
    // Native source revisions, not these presentation names, own authority.
    const listId = useMemo(() => {
        if (nextHandlerList >= Number.MAX_SAFE_INTEGER)
            throw Error("Association list identities exhausted");
        return "default-app-handlers-" + nextHandlerList++;
    }, []);
    return h(Column, { className: "default-app-card" },
        h(Text, { className: "default-app-title" }, target.family + " · " + target.id),
        target.detail ? h(Text, { wrap: true }, target.detail) : null,
        target.protected ? h(Text, null, "This association is protected.") : null,
        target.capability === "nativeConsent" ? h(Text, { wrap: true }, "Changing this association requires confirmation in system settings.") : null,
        target.handlersTruncated ? h(Text, null, "More applications are available in system settings.") : null,
        h(VirtualColumn, { id: listId, items: target.handlers, itemKey: associationKey, itemHeight: 32, gap: 8, overscan: 96, renderItem: handler => h(Button, { key: handler.id, id: listId + "/" + handler.id, className: handler.id === target.effectiveHandlerId ? "default-app-handler selected" : "default-app-handler", state: handler.id === target.effectiveHandlerId ? "selected" : "unselected", disabled: catalog.writable === false || !catalog.operations.setDefault || !target.canSetDefault || target.protected || handler.protected || handler.id === target.effectiveHandlerId, onClick: () => nickel.associations.setDefault(target.id, handler.id, catalog.revision) }, handler.name + (handler.id === target.effectiveHandlerId ? " · Current" : "")) }));
}
export function DefaultApps() {
    const catalog = nickel.associations.get();
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const [family, setFamily] = useState(null);
    const families = Array.from(new Set(catalog.targets.map(target => target.family)));
    const targets = catalog.targets.filter(target => (!family || target.family === family) && (!needle || (target.id + " " + target.family + " " + target.handlers.map(handler => handler.name).join(" ")).toLowerCase().includes(needle)));
    const result = catalog.lastResult;
    return h(Column, { className: "default-apps-page" },
        h(TextField, { id: "default-app-search", accessibilityLabel: "Search default applications", placeholder: "Search applications or file types", value: query, onChange: setQuery }),
        h(Row, null,
            h(Button, { id: "default-app-family-all", state: family === null ? "selected" : "unselected", onClick: () => setFamily(null) }, "All types"),
            families.map(value => h(Button, { key: value, id: "default-app-family/" + value, state: family === value ? "selected" : "unselected", onClick: () => setFamily(value) }, value))),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Default applications are unavailable.") : null,
        catalog.writable === false ? h(Text, null, "Default applications are read only.") : null,
        catalog.truncated ? h(Text, { wrap: true }, "This catalog is incomplete. Open system settings to see all associations.") : null,
        result ? h(Column, { className: "default-apps-result" },
            h(Text, null, result.status === "applied" ? "Default application updated." : result.status === "nativeConsentRequired" ? "Confirm this change in system settings." : result.status === "rejected" ? "The change was rejected." : result.status === "opened" ? "System settings opened." : result.status),
            result.detail ? h(Text, { wrap: true }, result.detail) : null) : null,
        catalog.operations.openSystemSettings ? h(Button, { id: "default-app-system-settings", disabled: catalog.writable === false, onClick: () => nickel.associations.openSystemSettings() }, "Open system default applications") : null,
        catalog.available && !targets.length ? h(Text, null, "No matching associations.") : null,
        h(VirtualColumn, { id: "default-app-targets", items: targets, itemKey: associationKey, itemHeight: associationHeight, gap: 16, overscan: 96, renderItem: target => h(AssociationCard, { key: target.id, target: target, catalog: catalog }) }));
}
registerSettingsPage({ id: "default-apps", group: "Applications", label: "Default applications", description: "Choose applications for file types and links", component: DefaultApps });
