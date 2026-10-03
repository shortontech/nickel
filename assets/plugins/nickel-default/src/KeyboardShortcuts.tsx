// @jsx h
import "./styles/features.css";

export function KeyboardShortcuts() {
    const snapshot = nickel.shortcuts.get();
    return <Column className="features-page">
        <Text wrap={true}>{snapshot.reason || "Keyboard shortcut reference"}</Text>
        {snapshot.globalReason ? <Text wrap={true}>{snapshot.globalReason}</Text> : null}
        {snapshot.shortcuts.map(shortcut => <Column key={shortcut.id} className="feature-card">
            <Text className="feature-title">{shortcut.action}</Text>
            <Text wrap={true}>{shortcut.keys}</Text>
            <Text>{shortcut.scope + (shortcut.available ? "" : " · Unavailable")}</Text>
        </Column>)}
    </Column>;
}

registerSettingsPage({id:"keyboard-shortcuts",group:"Input",label:"Keyboard shortcuts",description:"Shell shortcuts and navigation",component:KeyboardShortcuts});
