// @jsx h
import "./styles/settings.css";
import { SettingsCollection } from "./SettingsCollection.js";
import { SettingShortcut } from "./SettingShortcut.js";

function settingValue(setting) {
    const value = typeof setting.value === "function" ? setting.value() : setting.value;
    return value === undefined ? setting.defaultValue : value;
}

export function SettingControl(props) {
    const setting = props.setting;
    const controlId = props.controlId || setting.providerPackage + "/" + setting.id;
    const value = settingValue(setting);
    const change = next => setting.onChange && setting.onChange(next);

    if (setting.type === "group") {
        const Control = nickel.component("shell.settings.controls");
        const values = value || {};
        return <Column className="settings-group">
            {setting.fields.map(field => <Column key={field.id} className="settings-group-field">
                <Text>{field.label}</Text>
                {field.description ? <Text wrap={true}>{field.description}</Text> : null}
                <Control controlId={controlId + "/" + field.id} setting={{...field,
                    providerPackage:setting.providerPackage,
                    value:() => values[field.id] === undefined ? field.defaultValue : values[field.id],
                    onChange:next => change({...values, [field.id]:next})}} />
            </Column>)}
        </Column>;
    }
    if (setting.type === "repeated") {
        return <SettingsCollection setting={setting} value={value} controlId={controlId} onChange={change} />;
    }
    if (setting.type === "switch") {
        return <Switch id={controlId} state={value ? "on" : "off"}
            accessibilityLabel={setting.label} onClick={() => change(!value)} />;
    }
    if (setting.type === "slider") {
        return <Slider id={controlId} value={value}
            min={setting.min} max={setting.max} step={setting.step}
            accessibilityLabel={setting.label} onChange={change} />;
    }
    if (setting.type === "select") {
        return <SettingSelect setting={setting} value={value} controlId={controlId} onChange={change} />;
    }
    if (setting.type === "color") {
        const Picker = nickel.component("shell.settings.colorPicker");
        return <Picker id={controlId} label={setting.label} value={value}
            allowAlpha={setting.allowAlpha} onChange={change} />;
    }
    if (setting.type === "text" || setting.type === "number") {
        return <SettingInput setting={setting} value={value} controlId={controlId} onChange={change} />;
    }
    if (setting.type === "shortcut") {
        return <SettingShortcut setting={setting} value={value} controlId={controlId} onChange={change} />;
    }
    if (setting.type === "action") {
        return <Button id={controlId} onClick={() => change(null)}>{setting.actionLabel || setting.label}</Button>;
    }
    return <Text>{value === undefined || value === null ? "" : String(value)}</Text>;
}

function SettingInput({setting, value, controlId, onChange}) {
    const source = String(value === undefined || value === null ? "" : value);
    const [draft, setDraft] = useState({source, text:source});
    const text = draft.source === source ? draft.text : source;
    const number = Number(text);
    const valid = setting.type === "number"
        ? text.trim() !== "" && Number.isFinite(number) && number >= setting.min && number <= setting.max
        : [...text].length <= (setting.maxLength || 1024);
    return <Column className="settings-input">
        <Row className="settings-input-actions">
            <TextField id={controlId} value={text} accessibilityLabel={setting.label}
                onChange={next => setDraft({source, text:next})} />
            <Button id={controlId + "/apply"} disabled={!valid} onClick={() => {
                if (valid) onChange(setting.type === "number" ? number : text);
            }}>Apply</Button>
        </Row>
        {!valid ? <Text wrap={true}>{setting.type === "number"
            ? "Enter a number between " + setting.min + " and " + setting.max + "."
            : "Enter at most " + (setting.maxLength || 1024) + " characters."}</Text> : null}
    </Column>;
}

function SettingSelect({ setting, value, controlId, onChange }) {
    const [open, setOpen] = useState(false);
    const selected = setting.options.find(option => option.value === value);
    return <Select id={controlId} accessibilityLabel={setting.label}
        value={selected ? selected.label : String(value || "")} open={open}
        onClick={() => setOpen(!open)}>
        {setting.options.map((option, index) => <Option key={option.value}
            id={controlId + "/option-" + index} onClick={() => {
                onChange(option.value);
                setOpen(false);
            }}>{option.label}</Option>)}
    </Select>;
}

export function SettingsNavigation(props) {
    return <Column className="settings-destinations">
        {props.groups.map(group => <Column key={group.id}>
            <Text className="settings-section">{group.label}</Text>
            {group.entries.map(entry => <Button key={entry.providerPackage + "/" + entry.id}
                id={"settings-navigation/destination/" + entry.providerPackage + "/" + entry.id}
                state={entry.providerPackage + "/" + entry.id === props.activeId ? "selected" : "unselected"}
                className={entry.providerPackage + "/" + entry.id === props.activeId ? "settings-destination active" : "settings-destination"}
                onClick={() => props.onSelect(entry.providerPackage + "/" + entry.id)}>{entry.label}</Button>)}
        </Column>)}
    </Column>;
}

function groupEntries(settings, pages) {
    const groups = [];
    const byId = new Map();
    [...settings, ...pages].forEach(entry => {
        const label = entry.group || "General";
        if (!byId.has(label)) {
            const group = { id: label, label, entries: [] };
            byId.set(label, group);
            groups.push(group);
        }
        byId.get(label).entries.push(entry);
    });
    return groups;
}

export function Settings() {
    const SettingsNavigation = nickel.component("shell.settings.navigation");
    const SettingControl = nickel.component("shell.settings.controls");
    const settings = readPluginSettings().settings;
    const pages = readPluginSettingsPages().pages;
    const entries = [...settings, ...pages];
    const navigation = nickel.data.navigation || {};
    const revision = String(navigation.revision || "");
    const requested = entries.find(entry => entry.providerPackage + "/" + entry.id === navigation.destination)
        || entries.find(entry => entry.id === navigation.destination);
    const requestedId = requested ? requested.providerPackage + "/" + requested.id : null;
    const [selection, setSelection] = useState({revision, id:requestedId || (entries.length ? entries[0].providerPackage + "/" + entries[0].id : null)});
    const acceptedNavigation = useRef({revision, id:selection.id});
    if (acceptedNavigation.current.revision !== revision) {
        const previous = selection.revision === acceptedNavigation.current.revision ? selection.id : acceptedNavigation.current.id;
        acceptedNavigation.current = {revision, id:requestedId || previous};
    }
    const activeId = selection.revision === revision ? selection.id : acceptedNavigation.current.id;
    const setActiveId = id => setSelection({revision, id});
    const active = entries.find(entry => entry.providerPackage + "/" + entry.id === activeId) || entries[0];
    const ActivePage = active && typeof active.component === "function" ? active.component : null;
    const groups = groupEntries(settings, pages);

    return <Window id="settings" title="Nickel Settings" width={1100} height={800}
        className="settings-window">
        <div className="settings-shell wide">
            <div className="settings-sidebar">
                <ScrollView id="settings-sidebar-scroll" height={736}>
                    <SettingsNavigation groups={groups} activeId={active && active.providerPackage + "/" + active.id}
                        onSelect={setActiveId} />
                </ScrollView>
            </div>
            <div className="settings-detail">
                <Row className="settings-heading">
                    <Column>
                        <Text className="settings-title">{active ? active.label : "Settings"}</Text>
                        {active && active.description ? <Text className="settings-subtitle" wrap={true}>{active.description}</Text> : null}
                    </Column>
                </Row>
                <ScrollView id="settings-content" className="settings-content" height={704}>
                    {!active ? <Text>No settings are registered.</Text> : null}
                    {active && active.component && !ActivePage ? <Text>Settings page component is not resolved.</Text> : null}
                    {ActivePage ? <ActivePage /> : active && !active.component ? <Column className="settings-registered-control">
                        <Text>{active.label}</Text>
                        <SettingControl setting={active} />
                    </Column> : null}
                </ScrollView>
            </div>
        </div>
    </Window>;
}
