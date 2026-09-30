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
        return h(Column, { className: "settings-group" }, setting.fields.map(field => h(Column, { key: field.id, className: "settings-group-field" },
            h(Text, null, field.label),
            field.description ? h(Text, { wrap: true }, field.description) : null,
            h(Control, { controlId: controlId + "/" + field.id, setting: { ...field,
                    providerPackage: setting.providerPackage,
                    value: () => values[field.id] === undefined ? field.defaultValue : values[field.id],
                    onChange: next => change({ ...values, [field.id]: next }) } }))));
    }
    if (setting.type === "repeated") {
        return h(SettingsCollection, { setting: setting, value: value, controlId: controlId, onChange: change });
    }
    if (setting.type === "switch") {
        return h(Switch, { id: controlId, state: value ? "on" : "off", accessibilityLabel: setting.label, onClick: () => change(!value) });
    }
    if (setting.type === "slider") {
        return h(Slider, { id: controlId, value: value, min: setting.min, max: setting.max, step: setting.step, accessibilityLabel: setting.label, onChange: change });
    }
    if (setting.type === "select") {
        return h(SettingSelect, { setting: setting, value: value, controlId: controlId, onChange: change });
    }
    if (setting.type === "color") {
        const Picker = nickel.component("shell.settings.colorPicker");
        return h(Picker, { id: controlId, label: setting.label, value: value, allowAlpha: setting.allowAlpha, onChange: change });
    }
    if (setting.type === "text" || setting.type === "number") {
        return h(TextField, { id: controlId, value: String(value), accessibilityLabel: setting.label, onChange: next => {
                if (setting.type === "number") {
                    const number = Number(next);
                    if (Number.isFinite(number) && number >= setting.min && number <= setting.max)
                        change(number);
                }
                else {
                    change(next);
                }
            } });
    }
    if (setting.type === "shortcut") {
        return h(SettingShortcut, { setting: setting, value: value, controlId: controlId, onChange: change });
    }
    if (setting.type === "action") {
        return h(Button, { id: controlId, onClick: () => change(null) }, setting.actionLabel || setting.label);
    }
    return h(Text, null, value === undefined || value === null ? "" : String(value));
}
function SettingSelect({ setting, value, controlId, onChange }) {
    const [open, setOpen] = useState(false);
    const selected = setting.options.find(option => option.value === value);
    return h(Select, { id: controlId, accessibilityLabel: setting.label, value: selected ? selected.label : String(value || ""), open: open, onClick: () => setOpen(!open) }, setting.options.map((option, index) => h(Option, { key: option.value, id: controlId + "/option-" + index, onClick: () => {
            onChange(option.value);
            setOpen(false);
        } }, option.label)));
}
export function SettingsNavigation(props) {
    return h(Column, { className: "settings-destinations" }, props.groups.map(group => h(Column, { key: group.id },
        h(Text, { className: "settings-section" }, group.label),
        group.entries.map(entry => h(Button, { key: entry.providerPackage + "/" + entry.id, id: "settings-navigation/destination/" + entry.providerPackage + "/" + entry.id, state: entry.providerPackage + "/" + entry.id === props.activeId ? "selected" : "unselected", className: entry.providerPackage + "/" + entry.id === props.activeId ? "settings-destination active" : "settings-destination", onClick: () => props.onSelect(entry.providerPackage + "/" + entry.id) }, entry.label)))));
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
    const [activeId, setActiveId] = useState(entries.length ? entries[0].providerPackage + "/" + entries[0].id : null);
    const active = entries.find(entry => entry.providerPackage + "/" + entry.id === activeId) || entries[0];
    const ActivePage = active && typeof active.component === "function" ? active.component : null;
    const groups = groupEntries(settings, pages);
    return h(Window, { id: "settings", title: "Nickel Settings", width: 1100, height: 800, className: "settings-window" },
        h("div", { className: "settings-shell wide" },
            h("div", { className: "settings-sidebar" },
                h(ScrollView, { id: "settings-sidebar-scroll", height: 736 },
                    h(SettingsNavigation, { groups: groups, activeId: active && active.providerPackage + "/" + active.id, onSelect: setActiveId }))),
            h("div", { className: "settings-detail" },
                h(Row, { className: "settings-heading" },
                    h(Column, null,
                        h(Text, { className: "settings-title" }, active ? active.label : "Settings"),
                        active && active.description ? h(Text, { className: "settings-subtitle", wrap: true }, active.description) : null)),
                h(ScrollView, { id: "settings-content", className: "settings-content", height: 704 },
                    !active ? h(Text, null, "No settings are registered.") : null,
                    active && active.component && !ActivePage ? h(Text, null, "Settings page component is not resolved.") : null,
                    ActivePage ? h(ActivePage, null) : active && !active.component ? h(Column, { className: "settings-registered-control" },
                        h(Text, null, active.label),
                        h(SettingControl, { setting: active })) : null))));
}
