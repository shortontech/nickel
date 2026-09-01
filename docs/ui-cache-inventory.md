# UI cache inventory validation

`assets/ui-caches.tsv` is the machine-readable inventory for UI-path caches and retained derived
resources. Its bounds describe cache-owned payloads only. An `opaque_dependency` byte bound means
Nickel can identify the owner and lifetime but the dependency does not expose trustworthy retained
byte accounting; it never means zero bytes. Rows whose evidence distinguishes a known source-pixel
payload from opaque wrapper or renderer storage must keep those quantities separate.

`assets/ui-cache-lifecycle.tsv` is the executable owner-by-boundary companion matrix. It has exactly
one row for every inventory ID and an explicit action for hide, suspend, close, output reconnect,
topology shrink, theme, locale, font, application replacement, and fixture teardown. `na` is an
explicit declaration that a boundary does not apply to that owner's lifetime; it is not an omitted
decision. Workbench validation fails when either file gains or loses a resource independently or a
boundary action is unknown.

Run routine validation while admission measurements and lifecycle work are in progress:

```text
cargo run -p nickel-ui-workbench -- validate
```

Routine validation checks the schema, unique IDs, allowed statuses, measured-cache bounds, and a
fail-closed set of native compositor, frame, presenter, font, and renderer resources that must remain
inventoried. Adding a retained UI resource requires adding its ID to that required set and its full
inventory row.

Final specification completion uses the stricter gate:

```text
cargo run -p nickel-ui-workbench -- validate --final-completion
```

That command accepts only `removed`, `admitted_measured`, and `measured_admitted`. In particular,
`pending_measure` and `lifecycle_fixed` remain honest provisional states and cannot satisfy final
completion merely because bounds or lifecycle behavior have improved.
