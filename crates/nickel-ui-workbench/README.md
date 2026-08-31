# Nickel UI workbench authoring

Run `cargo run -p nickel-ui-workbench` for the native catalog. The headless loop is:

```text
cargo run -p nickel-ui-workbench -- list
cargo run -p nickel-ui-workbench -- metadata-json shared.primitives
cargo run -p nickel-ui-workbench -- semantic-json shared.primitives high-contrast
cargo run -p nickel-ui-workbench -- headless render-variant shared.primitives high-contrast /tmp/primitives.png
cargo run -p nickel-ui-workbench -- reachability shared.primitives --modality controller
cargo run -p nickel-ui-workbench -- feedback-evidence --full-comparison
```

## Add a component fixture

Implement `nickel_ui_testkit::Fixture` beside the component or surface. Give it a stable ID,
source location, at least one `FixtureVariant`, deterministic assets with license and SHA-256
metadata, and any potentially destructive `SimulatedEffectKind`s. Export a `FixtureProvider` from
the component crate; the workbench consumes that provider, so the component never depends on the
workbench binary or shell.

A variant declares its viewport, theme, locale/direction, scale, controller family, and
accessibility preferences. Add separate variants for meaningful narrow/wide, RTL, high-contrast,
reduced-effect, empty/loading/error, and overlay states. `create_variant` must construct those states
from injected data; fixtures must not read native services, user identity, the network, or ambient
filesystem contents.

The native workbench exposes variants, deterministic reset, modality routing, comparison, semantic
and accessibility inspection, runtime diagnostics, retained resources, and explicit simulation
labeling. Independent controls can change viewport, scale, theme, locale/direction, controller
family, contrast, and reduced effects without adding another named preset. Semantic-node selection
draws a non-interactive outline from the production semantic bounds. Admitted visual references can
be viewed side by side or as a deterministic difference overlay. The workbench never performs
declared external effects. Use semantic IDs/actions rather than copied
coordinates; pointer and controller routes go through production `UiHost` geometry/navigation.

## Baselines and acceptance

`validate` rejects duplicate metadata, nondeterministic rasters, unnamed semantics, incomplete asset
provenance, unmanifested or checksum-mismatched references, runtime diagnostics, and invalid
inventories. `feedback-evidence` measures incremental compilation and a selected focused test
against the versioned feedback manifest. `feedback-evidence --full-comparison` creates a fresh
isolated Cargo target, separately measures its bootstrap and clean incremental check, runs the old
launcher raster matrix, enforces both budgets, and requires at least a fourfold measured advantage.
A visual baseline is admitted only
after recording its asset path, compatible license, SHA-256, toolchain/profile, viewport, scale,
theme, locale, and accessibility variant.

Headless semantic behavior, raster inspection, native adapter behavior, and live desktop acceptance
are separate evidence. A screenshot or workbench scenario does not replace live acceptance.
