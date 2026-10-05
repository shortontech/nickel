// Keys live in the component, never in persisted user settings.
export function reconcileRows(state, values) {
    const available = state.rows;
    const used = new Set();
    const references = new Map();
    let encodings;
    const append = (map, key, index) => {
        let bucket = map.get(key);
        if (!bucket) map.set(key, bucket = { indices: [], next: 0 });
        bucket.indices.push(index);
    };
    const take = (map, key) => {
        const bucket = map.get(key);
        if (!bucket) return -1;
        while (bucket.next < bucket.indices.length) {
            const index = bucket.indices[bucket.next++];
            if (!used.has(index)) return index;
        }
        return -1;
    };
    available.forEach((row, index) => append(references, row.value, index));
    const rows = values.map(value => {
        let index = take(references, value);
        if (index < 0) {
            // Build the value index only when reference matching is insufficient.
            // Queues retain the original first-unused match for duplicate values.
            if (!encodings) {
                encodings = new Map();
                available.forEach((row, index) => {
                    if (!used.has(index)) append(encodings, JSON.stringify(row.value), index);
                });
            }
            index = take(encodings, JSON.stringify(value));
        }
        const row = index < 0 ? { key: "row-" + state.next++, value } : available[index];
        if (index >= 0) used.add(index);
        row.value = value;
        return row;
    });
    // Transport may copy equal values. Keep the logical sequence identity when
    // matching found the same rows in the same order, so viewport feedback does
    // not replace its own virtual source indefinitely.
    state.rows = rows.length === available.length && rows.every((row, index) => row === available[index])
        ? available : rows;
    return state.rows;
}

export function newRow(fields) {
    const row = {};
    fields.forEach(field => {
        if (field.defaultValue !== undefined) row[field.id] = JSON.parse(JSON.stringify(field.defaultValue));
        else if (field.type === "group") row[field.id] = newRow(field.fields);
    });
    return row;
}
