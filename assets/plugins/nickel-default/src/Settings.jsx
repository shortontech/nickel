// @jsx h
import "./styles/settings.css";

function settingValue(setting) {
    const value = typeof setting.value === "function" ? setting.value() : setting.value;
    return value === undefined ? setting.defaultValue : value;
}

export function SettingControl(props) {
    const setting = props.setting;
    const controlId = props.controlId || setting.providerPackage + "/" + setting.id;
    const value = settingValue(setting);
    const change = next => setting.onChange && setting.onChange(next);

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
        return <Column className="settings-options">
            {setting.options.map(option => <Button key={option.value}
                id={controlId + "/" + option.value}
                className={value === option.value ? "settings-option active" : "settings-option"}
                onClick={() => change(option.value)}>{option.label}</Button>)}
        </Column>;
    }
    if (setting.type === "text" || setting.type === "color" || setting.type === "number") {
        return <TextField id={controlId} value={String(value)}
            accessibilityLabel={setting.label} onChange={next => {
                if (setting.type === "number") {
                    const number = Number(next);
                    if (Number.isFinite(number) && number >= setting.min && number <= setting.max) change(number);
                } else {
                    change(next);
                }
            }} />;
    }
    if (setting.type === "action") {
        return <Button id={controlId} onClick={() => change(true)}>{setting.actionLabel || setting.label}</Button>;
    }
    return <Text>{value === undefined || value === null ? "" : String(value)}</Text>;
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
    const settings = readPluginSettings().settings;
    const pages = readPluginSettingsPages().pages;
    const entries = [...settings, ...pages];
    const [activeId, setActiveId] = useState(entries.length ? entries[0].providerPackage + "/" + entries[0].id : null);
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
