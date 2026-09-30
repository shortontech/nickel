// @jsx h
import "./styles/default-apps.css";
export function DefaultApps() {
    const catalog = nickel.associations.get();
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const targets = catalog.targets.filter(target => !needle || (target.id + " " + target.family + " " + target.handlers.map(handler => handler.name).join(" ")).toLowerCase().includes(needle));
    const result = catalog.lastResult;
    return h(Column, { className: "default-apps-page" },
        h(TextField, { id: "default-app-search", accessibilityLabel: "Search default applications", placeholder: "Search applications or file types", value: query, onChange: setQuery }),
        !catalog.available ? h(Text, { wrap: true }, catalog.reason || "Default applications are unavailable.") : null,
        catalog.writable === false ? h(Text, null, "Default applications are read only.") : null,
        catalog.truncated ? h(Text, { wrap: true }, "This catalog is incomplete. Open system settings to see all associations.") : null,
        result ? h(Column, { className: "default-apps-result" },
            h(Text, null, result.status === "applied" ? "Default application updated." : result.status === "nativeConsentRequired" ? "Confirm this change in system settings." : result.status === "rejected" ? "The change was rejected." : result.status === "opened" ? "System settings opened." : result.status),
            result.detail ? h(Text, { wrap: true }, result.detail) : null) : null,
        catalog.operations.openSystemSettings ? h(Button, { id: "default-app-system-settings", disabled: catalog.writable === false, onClick: () => nickel.associations.openSystemSettings() }, "Open system default applications") : null,
        catalog.available && !targets.length ? h(Text, null, "No matching associations.") : null,
        targets.map((target, targetIndex) => h(Column, { key: target.id, className: "default-app-card" },
            h(Text, { className: "default-app-title" }, target.family + " · " + target.id),
            target.detail ? h(Text, { wrap: true }, target.detail) : null,
            target.protected ? h(Text, null, "This association is protected.") : null,
            target.capability === "nativeConsent" ? h(Text, { wrap: true }, "Changing this association requires confirmation in system settings.") : null,
            target.handlersTruncated ? h(Text, null, "More applications are available in system settings.") : null,
            target.handlers.map((handler, handlerIndex) => h(Button, { key: handler.id, id: "default-app-handler-" + targetIndex + "-" + handlerIndex, className: handler.id === target.effectiveHandlerId ? "default-app-handler selected" : "default-app-handler", state: handler.id === target.effectiveHandlerId ? "selected" : "unselected", disabled: catalog.writable === false || !catalog.operations.setDefault || !target.canSetDefault || target.protected || handler.protected || handler.id === target.effectiveHandlerId, onClick: () => nickel.associations.setDefault(target.id, handler.id, catalog.revision) }, handler.name + (handler.id === target.effectiveHandlerId ? " · Current" : ""))))));
}
registerSettingsPage({ id: "default-apps", group: "Applications", label: "Default applications", description: "Choose applications for file types and links", component: DefaultApps });
