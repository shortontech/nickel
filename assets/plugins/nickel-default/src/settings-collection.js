// Keys live in the component, never in persisted user settings.
export function reconcileRows(state, values) {
    const available = state.rows.slice();
    const rows = values.map(value => {
        const encoded = JSON.stringify(value);
        let index = available.findIndex(row => row.value === value);
        if (index < 0) index = available.findIndex(row => JSON.stringify(row.value) === encoded);
        const row = index < 0 ? { key: "row-" + state.next++, value } : available.splice(index, 1)[0];
        row.value = value;
        return row;
    });
    state.rows = rows;
    return rows;
}

export function newRow(fields) {
    const row = {};
    fields.forEach(field => {
        if (field.defaultValue !== undefined) row[field.id] = JSON.parse(JSON.stringify(field.defaultValue));
        else if (field.type === "group") row[field.id] = newRow(field.fields);
    });
    return row;
}
