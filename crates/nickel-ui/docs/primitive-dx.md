# Primitive composition DX record

The compile-time fixtures in `primitives::tests::product_fixtures_compose_from_the_same_item_and_surface_primitives`
construct Settings, launcher, file-grid, and chat-list-shaped surfaces through the same five public
types: `SurfaceScaffold`, `ItemPresentation`, `ToolRegion`, `ActionRegion`, and `StatusRegion`.
They compile with typed primary and contextual messages and are resolved through `UiFrame` semantics.
Artwork and fallback are declared through `ArtworkPresentation`; tool and action regions own locale
ordering and bounded overflow instead of requiring product-side child reversal or truncation.

Compared with the product-local compositions which motivated Spec 0127, the fixture declarations own
no paint-command vectors, hit rectangles, overlay child insertion, focus-return reducer, mirrored narrow
page enum, or selected-item color table. Product code supplies content, stable IDs, typed actions, and a
semantic theme. The framework supplies layout, hit authority, accessibility state, focus/controller
styling, and transient policy. This state-ownership reduction is the acceptance signal; source line count
is deliberately not used as the sole measurement.

The collection compile-time fixtures additionally replace separate list/grid/virtual reducers with one
keyed `Collection` declaration. Its public configuration covers presentation, selection, disabled state,
reveal, bounded virtual scrolling, lifecycle states, RTL position reporting, activation, and context
activation without exposing renderer storage.
Loading, empty, and error states accept ordinary declarative slots as well as concise default labels.

Verification command:

```text
cargo test -p nickel-ui primitives::tests --lib
cargo test -p nickel-ui ui::collection::tests --lib
cargo test -p nickel-ui overlay::tests --lib
```
