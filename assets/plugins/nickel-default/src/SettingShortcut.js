// @jsx h
function validPart(part) {
    if (!part.trim())
        return false;
    let bytes = 0;
    for (const character of part) {
        const code = character.codePointAt(0);
        bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
    }
    return bytes <= 64;
}
export function SettingShortcut({ setting, value, controlId, onChange }) {
    const source = JSON.stringify(Array.isArray(value) ? value : []);
    const nextKey = useRef(0);
    const [draft, setDraft] = useState({ source, rows: JSON.parse(source).map((text, index) => ({ key: "saved-" + index, text })) });
    const rows = draft.source === source ? draft.rows : JSON.parse(source).map((text, index) => ({ key: "saved-" + index, text }));
    const update = rows => setDraft({ source, rows });
    const valid = rows.length <= 8 && rows.every(row => validPart(row.text));
    const apply = () => { if (valid)
        onChange(rows.map(row => row.text.trim())); };
    return h(Column, { className: "settings-shortcut" },
        h(Text, { wrap: true }, "Enter each key in the chord, such as Super and Space. An empty chord disables the shortcut."),
        rows.map((row, index) => h(Row, { key: row.key, className: "settings-shortcut-key" },
            h(TextField, { id: controlId + "/" + row.key, value: row.text, accessibilityLabel: setting.label + " key " + (index + 1), onChange: text => update(rows.map((entry, position) => position === index ? { ...entry, text } : entry)) }),
            h(Button, { id: controlId + "/" + row.key + "/remove", accessibilityLabel: "Remove key " + (index + 1), onClick: () => update(rows.filter((_, position) => position !== index)) }, "Remove"))),
        !valid ? h(Text, { wrap: true }, "Each key must contain 1\u201364 bytes, with at most eight keys.") : null,
        h(Row, { className: "settings-shortcut-actions" },
            h(Button, { id: controlId + "/add", disabled: rows.length >= 8, onClick: () => {
                    if (rows.length < 8)
                        update([...rows, { key: "draft-" + nextKey.current++, text: "" }]);
                } }, "Add key"),
            h(Button, { id: controlId + "/clear", disabled: !rows.length, onClick: () => update([]) }, "Clear"),
            h(Button, { id: controlId + "/apply", disabled: !valid, onClick: apply }, "Apply")));
}
