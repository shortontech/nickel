// @jsx h
import { reconcileRows, newRow } from "./settings-collection.js";
import { SettingsDraftContext, draftPaths, draftSchema, pruneCollectionDrafts, rowDraftScope, useCollectionIdentity } from "./SettingsDrafts.js";
const emptyValues = [];
const rowKey = row => row.key;
let nextCollection = 0;
function estimatedFieldsHeight(fields) {
    return fields.reduce((height, field) => height + 36 + (field.description ? 32 : 0)
        + (field.type === "group" ? estimatedFieldsHeight(field.fields) : 72), 0);
}
export function SettingsCollection({ setting, value, controlId, onChange, draftState, onDraftChange }) {
    const localIdentity = useCollectionIdentity(controlId);
    const identity = draftState || localIdentity;
    const [, refreshDrafts] = useState(null);
    const values = Array.isArray(value) ? value : emptyValues;
    const rows = useMemo(() => reconcileRows(identity, values), [identity, values]);
    const schema = useMemo(() => JSON.stringify(draftSchema(setting.fields)), [setting.fields]);
    // Schema changes retire drafts in unmaterialized rows too. Scrolling and
    // draft edits with the same schema must not walk the logical collection.
    useMemo(() => pruneCollectionDrafts(identity, setting.fields), [identity, schema]);
    const paths = useMemo(() => draftPaths(setting.fields), [setting.fields]);
    const height = useMemo(() => 64 + estimatedFieldsHeight(setting.fields), [setting.fields]);
    const listId = useMemo(() => {
        if (nextCollection >= Number.MAX_SAFE_INTEGER)
            throw Error("Settings collection identities exhausted");
        return "settings-collection-" + nextCollection++;
    }, []);
    const Control = nickel.component("shell.settings.controls");
    return h(Column, { className: "settings-collection" },
        h(VirtualColumn, { id: listId, items: rows, itemKey: rowKey, itemHeight: height, gap: 12, overscan: 96, renderItem: (row, index) => h(SettingsDraftContext.Provider, { key: row.key, value: rowDraftScope(row, controlId + "/" + row.key + "/", paths) },
                h(Column, { className: "settings-collection-row" },
                    setting.fields.map(field => h(Column, { key: field.id, className: "settings-group-field" },
                        h(Text, null, field.label),
                        field.description ? h(Text, { wrap: true }, field.description) : null,
                        h(Control, { controlId: controlId + "/" + row.key + "/" + field.id, draftState: row.controls?.[field.id]?.type === field.type ? row.controls[field.id].state : undefined, onDraftChange: next => {
                                const controls = row.controls || (row.controls = Object.create(null));
                                controls[field.id] = { type: field.type, state: next };
                                refreshDrafts({});
                                if (onDraftChange)
                                    onDraftChange(identity);
                            }, setting: { ...field, providerPackage: setting.providerPackage,
                                value: row.value[field.id] === undefined ? field.defaultValue : row.value[field.id],
                                onChange: next => {
                                    const updated = values.map((value, position) => position === index ? { ...value, [field.id]: next } : value);
                                    identity.rows[index].value = updated[index];
                                    onChange(updated);
                                } } }))),
                    h(Button, { id: controlId + "/" + row.key + "/remove", disabled: values.length <= (setting.minItems || 0), onClick: () => {
                            if (values.length <= (setting.minItems || 0))
                                return;
                            identity.rows = identity.rows.filter((_, position) => position !== index);
                            onChange(values.filter((_, position) => position !== index));
                        } }, "Remove"))) }),
        h(Button, { id: controlId + "/add", disabled: values.length >= setting.maxItems, onClick: () => {
                if (values.length < setting.maxItems)
                    onChange([...values, newRow(setting.fields)]);
            } }, "Add"));
}
