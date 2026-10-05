// Drafts belong to a logical repeated row, not to its materialized control.
export const SettingsDraftContext = createContext(null);

export function draftPaths(fields, prefix = "", paths = Object.create(null)) {
    for (const field of fields) {
        const path = prefix + field.id;
        if (field.type === "group") draftPaths(field.fields, path + "/", paths);
        else if (field.type === "text" || field.type === "number") paths[field.type + ":" + path] = true;
        else if (field.type === "repeated") paths["repeated:" + path] = true;
    }
    return paths;
}

// Only schema identity matters here; labels and current values do not invalidate drafts.
export function draftSchema(fields) {
    return fields.map(field => [field.id, field.type,
        field.fields ? draftSchema(field.fields) : null]);
}

export function pruneCollectionDrafts(identity, fields) {
    const paths = draftPaths(fields);
    function pruneControls(controls, schema) {
        if (!controls) return;
        const byId = new Map(schema.map(field => [field.id, field]));
        for (const id of Object.keys(controls)) {
            const field = byId.get(id);
            const retained = controls[id];
            if (!field || retained.type !== field.type) delete controls[id];
            else if (field.type === "group") pruneControls(retained.state, field.fields);
            else if (field.type === "repeated") pruneCollectionDrafts(retained.state, field.fields);
        }
    }
    function pruneCollections(collections, schema, prefix = "") {
        if (!collections) return;
        for (const field of schema) {
            const path = prefix + field.id;
            if (field.type === "group") pruneCollections(collections, field.fields, path + "/");
            else if (field.type === "repeated" && collections[path])
                pruneCollectionDrafts(collections[path], field.fields);
        }
    }
    for (const row of identity.rows) {
        if (row.drafts) for (const key of Object.keys(row.drafts))
            if (!paths[key]) delete row.drafts[key];
        if (row.collections) for (const key of Object.keys(row.collections))
            if (!paths["repeated:" + key]) delete row.collections[key];
        pruneCollections(row.collections, fields);
        pruneControls(row.controls, fields);
    }
}

export function rowDraftScope(row, prefix, paths) {
    // Plain null-prototype records participate in native transactional checkpoints.
    const drafts = row.drafts || (row.drafts = Object.create(null));
    const collections = row.collections || (row.collections = Object.create(null));
    for (const key of Object.keys(drafts)) if (!paths[key]) delete drafts[key];
    for (const key of Object.keys(collections)) if (!paths["repeated:" + key]) delete collections[key];
    return { drafts, collections, prefix };
}

export function useCollectionIdentity(controlId) {
    const scope = useContext(SettingsDraftContext);
    const local = useRef({rows: [], next: 0});
    if (!scope || !controlId.startsWith(scope.prefix)) return local.current;
    const key = controlId.slice(scope.prefix.length);
    if (!scope.collections[key]) scope.collections[key] = {rows: [], next: 0};
    return scope.collections[key];
}

export function useSettingDraft(source, controlId, type) {
    const scope = useContext(SettingsDraftContext);
    const key = scope && controlId.startsWith(scope.prefix)
        ? type + ":" + controlId.slice(scope.prefix.length) : null;
    const store = key === null ? null : scope.drafts;
    const local = useRef({source, key, store, type, text:source});
    const [, refresh] = useState(null);
    if (local.current.source !== source || local.current.key !== key || local.current.store !== store || local.current.type !== type)
        local.current = {source, key, store, type, text:source};
    const retained = key === null ? null : scope.drafts[key];
    if (retained && retained.source !== source) delete scope.drafts[key];
    const draft = retained && retained.source === source ? retained : local.current;
    return [draft.source === source ? draft.text : source, text => {
        const next = {source, text};
        if (key !== null) scope.drafts[key] = next;
        local.current = {...next, key, store, type};
        refresh(next);
    }];
}
