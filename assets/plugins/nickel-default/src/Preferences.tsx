// @jsx h
import "./styles/preferences.css";

function PreferenceStatus(props) {
    const snapshot = props.snapshot;
    return !snapshot.available ? <Text wrap={true}>{snapshot.reason || "Shell preferences are unavailable."}</Text>
        : !snapshot.writable ? <Text>Shell preferences are read only.</Text> : null;
}

function PreferenceSwitch(props) {
    const enabled = !!props.value;
    return <Row className="preferences-row">
        <Column className="preferences-description"><Text>{props.label}</Text>
            {props.description ? <Text wrap={true}>{props.description}</Text> : null}
        </Column>
        <Switch id={props.id} accessibilityLabel={props.label}
            state={props.writable ? enabled ? "on" : "off" : enabled ? "disabled-on" : "disabled-off"}
            onClick={props.writable ? () => props.onChange(!enabled) : undefined} />
    </Row>;
}

export function Preferences() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const set = patch => nickel.preferences.set(patch);
    return <Column className="preferences-page">
        <PreferenceStatus snapshot={snapshot} />
        {snapshot.available ? <Column className="preferences-card">
            <Text className="preferences-heading">Taskbar</Text>
            <PreferenceSwitch id="preferences-bar-all-displays" label="Show the taskbar on every display"
                value={configured.barOnAllDisplays} writable={snapshot.writable}
                onChange={value => set({barOnAllDisplays:value})} />
            <PreferenceSwitch id="preferences-bar-all-windows" label="Show windows from every display on each taskbar"
                value={configured.allWindowsOnEveryBar} writable={snapshot.writable}
                onChange={value => set({allWindowsOnEveryBar:value})} />
            <Text className="preferences-heading">Virtual desktops</Text>
            <Text>{"Number of desktops: " + configured.desktopCount}</Text>
            <Row className="preferences-choices">
                {[1,2,3,4,5,6,7,8,9,10].map(count => <Button key={count} id={"preferences-desktop-count-"+count}
                    state={configured.desktopCount === count ? "selected" : "unselected"}
                    className={configured.desktopCount === count ? "preferences-choice selected" : "preferences-choice"}
                    disabled={!snapshot.writable} onClick={() => set({desktopCount:count})}>{count}</Button>)}
            </Row>
        </Column> : null}
    </Column>;
}

function IdleTimeout(props) {
    const [draft, setDraft] = useState("");
    const seconds = Number(draft);
    const valid = draft.trim() !== "" && Number.isInteger(seconds) && seconds >= 30 && seconds <= 604800;
    const set = value => nickel.preferences.set({[props.field]:value});
    return <Column className="preferences-card">
        <Text className="preferences-heading">{props.label}</Text>
        <Text>{props.value === null ? "Disabled" : "After " + props.value + " seconds"}</Text>
        <Row className="preferences-choices">
            {[{value:null,label:"Disabled"},{value:300,label:"5 minutes"},{value:900,label:"15 minutes"},{value:1800,label:"30 minutes"},{value:3600,label:"1 hour"}].map(option => <Button key={String(option.value)}
                id={"preferences-"+props.field+"-"+String(option.value)} disabled={!props.writable}
                state={props.value === option.value ? "selected" : "unselected"}
                className={props.value === option.value ? "preferences-choice selected" : "preferences-choice"}
                onClick={() => set(option.value)}>{option.label}</Button>)}
        </Row>
        <Text>Custom timeout, in seconds (30 to 604800)</Text>
        <Row className="preferences-row">
            <TextField id={"preferences-"+props.field+"-custom"} accessibilityLabel={props.label+" custom timeout in seconds"}
                placeholder="Seconds" value={draft} onChange={setDraft} />
            <Button id={"preferences-"+props.field+"-apply"} disabled={!props.writable || !valid} onClick={() => set(seconds)}>Apply</Button>
        </Row>
    </Column>;
}

export function IdlePreferences() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    return <Column className="preferences-page">
        <PreferenceStatus snapshot={snapshot} />
        {snapshot.available ? <Column className="preferences-page">
            <Text wrap={true}>Choose what happens when the session is idle. Changing a timeout starts a fresh idle interval.</Text>
            <IdleTimeout field="idleDimSeconds" label="Dim the display" value={configured.idleDimSeconds} writable={snapshot.writable} />
            <IdleTimeout field="idleLockSeconds" label="Lock the session" value={configured.idleLockSeconds} writable={snapshot.writable} />
            <IdleTimeout field="idleSuspendSeconds" label="Suspend the computer" value={configured.idleSuspendSeconds} writable={snapshot.writable} />
        </Column> : null}
    </Column>;
}

function ApplicationPreference(props) {
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const installed = new Map(nickel.applications.list().map(application => [application.id, application.name]));
    const applications = props.snapshot.applications || [];
    const choices = applications.map(application => ({id:application.id,
        name:(installed.get(application.id) || application.id)}));
    const selected = choices.find(application => application.id === props.value);
    const filtered = choices.filter(application => !needle || (application.id+" "+application.name).toLowerCase().includes(needle));
    const set = id => nickel.preferences.set({[props.field]:id});
    return <Column className="preferences-card">
        <Text className="preferences-heading">{props.label}</Text>
        <Text wrap={true}>{props.unavailable ? "The configured application is unavailable. It will be retained until you choose an application or restore the system default."
            : props.value === null ? "System default" : "Current: " + (selected?.name || props.value)}</Text>
        <Button id={"preferences-"+props.field+"-system"} disabled={!props.snapshot.writable}
            state={props.value === null && !props.unavailable ? "selected" : "unselected"} onClick={() => set(null)}>Use system default</Button>
        <TextField id={"preferences-"+props.field+"-search"} accessibilityLabel={"Search "+props.label.toLowerCase()}
            placeholder="Search installed applications" value={query} onChange={setQuery} />
        {filtered.slice(0,40).map((application,index) => <Button key={application.id} id={"preferences-"+props.field+"-choice-"+index}
            disabled={!props.snapshot.writable} className={props.value === application.id ? "preferences-choice selected" : "preferences-choice"}
            state={props.value === application.id ? "selected" : "unselected"} onClick={() => set(application.id)}>{application.name.slice(0,120)}</Button>)}
        {filtered.length > 40 ? <Text>Search to narrow the remaining applications.</Text> : null}
        {!filtered.length ? <Text>No installed applications match.</Text> : null}
    </Column>;
}

export function PreferredApplications() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const unavailable = snapshot.unavailableSelections || {};
    return <Column className="preferences-page">
        <PreferenceStatus snapshot={snapshot} />
        {snapshot.available ? <Column className="preferences-page">
            <Text wrap={true}>Choose which applications Nickel uses to open terminals and file windows.</Text>
            <ApplicationPreference field="preferredTerminal" label="Preferred terminal" value={configured.preferredTerminal}
                unavailable={unavailable.preferredTerminal} snapshot={snapshot} />
            <ApplicationPreference field="preferredFileManager" label="Preferred file manager" value={configured.preferredFileManager}
                unavailable={unavailable.preferredFileManager} snapshot={snapshot} />
        </Column> : null}
    </Column>;
}

export function FileArtwork() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const themes = snapshot.iconThemes || [];
    const filtered = themes.filter(theme => !needle || theme.toLowerCase().includes(needle));
    const usesSystemIcons = configured.fileIconProvider === "system";
    const set = patch => nickel.preferences.set(patch);
    return <Column className="preferences-page">
        <PreferenceStatus snapshot={snapshot} />
        {snapshot.available ? <Column className="preferences-card">
            <Text className="preferences-heading">Icon provider</Text>
            <Row className="preferences-choices">
                {[{value:"nickel",label:"Nickel icons"},{value:"system",label:"System icons"}].map(option => <Button key={option.value}
                    id={"preferences-file-icons-"+option.value} disabled={!snapshot.writable}
                    className={configured.fileIconProvider === option.value ? "preferences-choice selected" : "preferences-choice"}
                    state={configured.fileIconProvider === option.value ? "selected" : "unselected"}
                    onClick={() => set({fileIconProvider:option.value})}>{option.label}</Button>)}
            </Row>
            <Text wrap={true}>{!usesSystemIcons ? "Nickel artwork"+(configured.fileIconTheme ? ". Saved system theme: "+configured.fileIconTheme : "")
                : snapshot.unavailableSelections?.fileIconTheme ? "Configured theme "+configured.fileIconTheme+" is unavailable; Nickel artwork is used until you select another theme."
                : configured.fileIconTheme ? "Selected system theme: "+configured.fileIconTheme : "System default icon theme"}</Text>
            {usesSystemIcons ? <Button id="preferences-file-theme-system" disabled={!snapshot.writable}
                className={configured.fileIconTheme === null ? "preferences-choice selected" : "preferences-choice"}
                state={configured.fileIconTheme === null ? "selected" : "unselected"}
                onClick={() => set({fileIconProvider:"system",fileIconTheme:null})}>Use system default icon theme</Button> : null}
            {usesSystemIcons && themes.length ? <Column className="preferences-page">
                <TextField id="preferences-file-theme-search" accessibilityLabel="Search icon themes" placeholder="Search installed icon themes" value={query} onChange={setQuery} />
                {filtered.slice(0,40).map((theme,index) => <Button key={theme} id={"preferences-file-theme-choice-"+index}
                    disabled={!snapshot.writable} className={configured.fileIconTheme === theme ? "preferences-choice selected" : "preferences-choice"}
                    state={configured.fileIconTheme === theme ? "selected" : "unselected"}
                    onClick={() => set({fileIconProvider:"system",fileIconTheme:theme})}>{theme.slice(0,120)}</Button>)}
                {filtered.length > 40 ? <Text>Search to narrow the remaining icon themes.</Text> : null}
                {!filtered.length ? <Text>No installed icon themes match.</Text> : null}
            </Column> : usesSystemIcons ? <Text>No installed icon themes are available on this platform.</Text> : null}
        </Column> : null}
    </Column>;
}

registerSettingsPage({id:"shell-preferences",group:"Shell",label:"Desktop and taskbar",description:"Virtual desktops and taskbar behavior",component:Preferences});
registerSettingsPage({id:"idle-preferences",group:"Power and security",label:"Idle behavior",description:"Dim, lock, and suspend timeouts",component:IdlePreferences});
registerSettingsPage({id:"preferred-applications",group:"Applications",label:"Preferred applications",description:"Terminal and file manager choices",component:PreferredApplications});
registerSettingsPage({id:"file-artwork",group:"Appearance",label:"File artwork",description:"File icon provider and system icon themes",component:FileArtwork});
