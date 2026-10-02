// @jsx h
import { reconcileRows, newRow } from "./settings-collection.js";
export function SettingsCollection({ setting, value, controlId, onChange }) {
    const identity = useRef({ rows: [], next: 0 });
    const values = Array.isArray(value) ? value : [];
    const rows = reconcileRows(identity.current, values);
    const Control = nickel.component("shell.settings.controls");
    return h(Column, { className: "settings-collection" },
        rows.map((row, index) => h(Column, { key: row.key, className: "settings-collection-row" },
            setting.fields.map(field => h(Column, { key: field.id, className: "settings-group-field" },
                h(Text, null, field.label),
                field.description ? h(Text, { wrap: true }, field.description) : null,
                h(Control, { controlId: controlId + "/" + row.key + "/" + field.id, setting: { ...field, providerPackage: setting.providerPackage,
                        value: () => row.value[field.id] === undefined ? field.defaultValue : row.value[field.id],
                        onChange: next => {
                            const updated = values.map((value, position) => position === index ? { ...value, [field.id]: next } : value);
                            identity.current.rows[index].value = updated[index];
                            onChange(updated);
                        } } }))),
            h(Button, { id: controlId + "/" + row.key + "/remove", disabled: values.length <= (setting.minItems || 0), onClick: () => {
                    if (values.length <= (setting.minItems || 0))
                        return;
                    identity.current.rows.splice(index, 1);
                    onChange(values.filter((_, position) => position !== index));
                } }, "Remove"))),
        h(Button, { id: controlId + "/add", disabled: values.length >= setting.maxItems, onClick: () => {
                if (values.length < setting.maxItems)
                    onChange([...values, newRow(setting.fields)]);
            } }, "Add"));
}
