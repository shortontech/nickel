// @jsx h
import "./styles/preferences.css";
function PreferenceStatus(props) {
    const snapshot = props.snapshot;
    return !snapshot.available ? h(Text, { wrap: true }, snapshot.reason || "Shell preferences are unavailable.")
        : !snapshot.writable ? h(Text, null, "Shell preferences are read only.") : null;
}
function PreferenceSwitch(props) {
    const enabled = !!props.value;
    return h(Row, { className: "preferences-row" },
        h(Column, { className: "preferences-description" },
            h(Text, null, props.label),
            props.description ? h(Text, { wrap: true }, props.description) : null),
        h(Switch, { id: props.id, accessibilityLabel: props.label, state: props.writable ? enabled ? "on" : "off" : enabled ? "disabled-on" : "disabled-off", onClick: props.writable ? () => props.onChange(!enabled) : undefined }));
}
export function Preferences() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const set = patch => nickel.preferences.set(patch);
    return h(Column, { className: "preferences-page" },
        h(PreferenceStatus, { snapshot: snapshot }),
        snapshot.available ? h(Column, { className: "preferences-card" },
            h(Text, { className: "preferences-heading" }, "Taskbar"),
            h(PreferenceSwitch, { id: "preferences-bar-all-displays", label: "Show the taskbar on every display", value: configured.barOnAllDisplays, writable: snapshot.writable, onChange: value => set({ barOnAllDisplays: value }) }),
            h(PreferenceSwitch, { id: "preferences-bar-all-windows", label: "Show windows from every display on each taskbar", value: configured.allWindowsOnEveryBar, writable: snapshot.writable, onChange: value => set({ allWindowsOnEveryBar: value }) }),
            h(Text, { className: "preferences-heading" }, "Virtual desktops"),
            h(Text, null, "Number of desktops: " + configured.desktopCount),
            h(Row, { className: "preferences-choices" }, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map(count => h(Button, { key: count, id: "preferences-desktop-count-" + count, state: configured.desktopCount === count ? "selected" : "unselected", className: configured.desktopCount === count ? "preferences-choice selected" : "preferences-choice", disabled: !snapshot.writable, onClick: () => set({ desktopCount: count }) }, count)))) : null);
}
function IdleTimeout(props) {
    const [draft, setDraft] = useState("");
    const seconds = Number(draft);
    const valid = draft.trim() !== "" && Number.isInteger(seconds) && seconds >= 30 && seconds <= 604800;
    const set = value => nickel.preferences.set({ [props.field]: value });
    return h(Column, { className: "preferences-card" },
        h(Text, { className: "preferences-heading" }, props.label),
        h(Text, null, props.value === null ? "Disabled" : "After " + props.value + " seconds"),
        h(Row, { className: "preferences-choices" }, [{ value: null, label: "Disabled" }, { value: 300, label: "5 minutes" }, { value: 900, label: "15 minutes" }, { value: 1800, label: "30 minutes" }, { value: 3600, label: "1 hour" }].map(option => h(Button, { key: String(option.value), id: "preferences-" + props.field + "-" + String(option.value), disabled: !props.writable, state: props.value === option.value ? "selected" : "unselected", className: props.value === option.value ? "preferences-choice selected" : "preferences-choice", onClick: () => set(option.value) }, option.label))),
        h(Text, null, "Custom timeout, in seconds (30 to 604800)"),
        h(Row, { className: "preferences-row" },
            h(TextField, { id: "preferences-" + props.field + "-custom", accessibilityLabel: props.label + " custom timeout in seconds", placeholder: "Seconds", value: draft, onChange: setDraft }),
            h(Button, { id: "preferences-" + props.field + "-apply", disabled: !props.writable || !valid, onClick: () => set(seconds) }, "Apply")));
}
export function IdlePreferences() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    return h(Column, { className: "preferences-page" },
        h(PreferenceStatus, { snapshot: snapshot }),
        snapshot.available ? h(Column, { className: "preferences-page" },
            h(Text, { wrap: true }, "Choose what happens when the session is idle. Changing a timeout starts a fresh idle interval."),
            h(IdleTimeout, { field: "idleDimSeconds", label: "Dim the display", value: configured.idleDimSeconds, writable: snapshot.writable }),
            h(IdleTimeout, { field: "idleLockSeconds", label: "Lock the session", value: configured.idleLockSeconds, writable: snapshot.writable }),
            h(IdleTimeout, { field: "idleSuspendSeconds", label: "Suspend the computer", value: configured.idleSuspendSeconds, writable: snapshot.writable })) : null);
}
function ApplicationPreference(props) {
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const installed = new Map(nickel.applications.list().map(application => [application.id, application.name]));
    const applications = props.snapshot.applications || [];
    const choices = applications.map(application => ({ id: application.id,
        name: (installed.get(application.id) || application.id) }));
    const selected = choices.find(application => application.id === props.value);
    const filtered = choices.filter(application => !needle || (application.id + " " + application.name).toLowerCase().includes(needle));
    const set = id => nickel.preferences.set({ [props.field]: id });
    return h(Column, { className: "preferences-card" },
        h(Text, { className: "preferences-heading" }, props.label),
        h(Text, { wrap: true }, props.unavailable ? "The configured application is unavailable. It will be retained until you choose an application or restore the system default."
            : props.value === null ? "System default" : "Current: " + (selected?.name || props.value)),
        h(Button, { id: "preferences-" + props.field + "-system", disabled: !props.snapshot.writable, state: props.value === null && !props.unavailable ? "selected" : "unselected", onClick: () => set(null) }, "Use system default"),
        h(TextField, { id: "preferences-" + props.field + "-search", accessibilityLabel: "Search " + props.label.toLowerCase(), placeholder: "Search installed applications", value: query, onChange: setQuery }),
        filtered.slice(0, 40).map((application, index) => h(Button, { key: application.id, id: "preferences-" + props.field + "-choice-" + index, disabled: !props.snapshot.writable, className: props.value === application.id ? "preferences-choice selected" : "preferences-choice", state: props.value === application.id ? "selected" : "unselected", onClick: () => set(application.id) }, application.name.slice(0, 120))),
        filtered.length > 40 ? h(Text, null, "Search to narrow the remaining applications.") : null,
        !filtered.length ? h(Text, null, "No installed applications match.") : null);
}
export function PreferredApplications() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const unavailable = snapshot.unavailableSelections || {};
    return h(Column, { className: "preferences-page" },
        h(PreferenceStatus, { snapshot: snapshot }),
        snapshot.available ? h(Column, { className: "preferences-page" },
            h(Text, { wrap: true }, "Choose which applications Nickel uses to open terminals and file windows."),
            h(ApplicationPreference, { field: "preferredTerminal", label: "Preferred terminal", value: configured.preferredTerminal, unavailable: unavailable.preferredTerminal, snapshot: snapshot }),
            h(ApplicationPreference, { field: "preferredFileManager", label: "Preferred file manager", value: configured.preferredFileManager, unavailable: unavailable.preferredFileManager, snapshot: snapshot })) : null);
}
export function FileArtwork() {
    const snapshot = nickel.preferences.get();
    const configured = snapshot.configured || {};
    const [query, setQuery] = useState("");
    const needle = query.trim().toLowerCase();
    const themes = snapshot.iconThemes || [];
    const filtered = themes.filter(theme => !needle || theme.toLowerCase().includes(needle));
    const set = patch => nickel.preferences.set(patch);
    return h(Column, { className: "preferences-page" },
        h(PreferenceStatus, { snapshot: snapshot }),
        snapshot.available ? h(Column, { className: "preferences-card" },
            h(Text, { className: "preferences-heading" }, "File artwork"),
            h(Row, { className: "preferences-choices" }, [{ value: "nickel", label: "Nickel icons" }, { value: "system", label: "System icons" }].map(option => h(Button, { key: option.value, id: "preferences-file-icons-" + option.value, disabled: !snapshot.writable, state: configured.fileIconProvider === option.value ? "selected" : "unselected", onClick: () => set({ fileIconProvider: option.value }) }, option.label))),
            h(Text, { wrap: true }, snapshot.unavailableSelections?.fileIconTheme ? "Configured theme " + configured.fileIconTheme + " is unavailable and will be retained until you select another theme."
                : configured.fileIconTheme ? "Selected theme: " + configured.fileIconTheme : "System default icon theme"),
            h(Button, { id: "preferences-file-theme-system", disabled: !snapshot.writable, onClick: () => set({ fileIconProvider: "system", fileIconTheme: null }) }, "Use system default icon theme"),
            themes.length ? h(Column, { className: "preferences-page" },
                h(TextField, { id: "preferences-file-theme-search", accessibilityLabel: "Search icon themes", placeholder: "Search installed icon themes", value: query, onChange: setQuery }),
                filtered.slice(0, 40).map((theme, index) => h(Button, { key: theme, id: "preferences-file-theme-choice-" + index, disabled: !snapshot.writable, state: configured.fileIconTheme === theme ? "selected" : "unselected", onClick: () => set({ fileIconProvider: "system", fileIconTheme: theme }) }, theme.slice(0, 120))),
                filtered.length > 40 ? h(Text, null, "Search to narrow the remaining icon themes.") : null,
                !filtered.length ? h(Text, null, "No installed icon themes match.") : null) : h(Text, null, "No installed icon themes are available on this platform.")) : null);
}
registerSettingsPage({ id: "shell-preferences", group: "Shell", label: "Desktop and taskbar", description: "Virtual desktops and taskbar behavior", component: Preferences });
registerSettingsPage({ id: "idle-preferences", group: "Power and security", label: "Idle behavior", description: "Dim, lock, and suspend timeouts", component: IdlePreferences });
registerSettingsPage({ id: "preferred-applications", group: "Applications", label: "Preferred applications", description: "Terminal and file manager choices", component: PreferredApplications });
registerSettingsPage({ id: "file-artwork", group: "Appearance", label: "File artwork", description: "File icon provider and system icon themes", component: FileArtwork });
