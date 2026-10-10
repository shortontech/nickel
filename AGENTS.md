# Repository Guidelines

## Project Structure & Module Organization

Nickel is a cross-platform desktop shell targeting Windows and Linux. Its native host is a Cargo workspace, while its production shell presentation is supplied by capability-constrained JSX/CSS packages under `assets/plugins/`:

- `crates/nickel/`: application binaries, session orchestration, and platform entry points.
- `crates/nickel-core/`: platform-neutral application state and domain logic.
- Pinned upstream `twinkle` libraries: native rendering, layout, input, widgets, JSX execution and native presentation. The revision is declared in the root `Cargo.toml`.
- `crates/nickel-platform/`: narrow Windows and Linux adapters.
- `crates/nickel-jsx-host/`: package admission, composition, Settings and capability-gated host APIs.
- `crates/nickel-ui-host/`: hosted controller transport, localization and shell appearance adapters.
- `crates/nickel-session-protocol/`: typed communication between shell processes and trusted local clients.
- `assets/plugins/nickel-default/`: the production default shell, canonically authored in TSX/CSS; JS, JSX, and TS package inputs remain supported.
- `assets/plugins/nickel-cupertino-dock/`: a bundled derived shell that inherits the default shell and replaces its shell/taskbar contracts.
- `assets/`: shell packages, fonts, shaders, icons, and test fixtures with compatible licenses.
- `specs/`: active design specifications; move completed specifications to `specs/done/`.

Keep search, ranking, navigation, task-switcher policy, package validation, and capability enforcement independent of native window APIs so they remain deterministic and portable. Rust owns trusted state, policy, rendering, validation, effects, and platform integration. JSX owns shell composition and presentation through bounded native APIs; it must not become an authority bypass.

## Build, Test, and Development Commands

Use standard Cargo commands from the workspace root:

- `cargo run -p nickel`: launch Nickel with the default platform configuration.
- `cargo run -p nickel --no-default-features --features backend-winit --bin nickel-nested`: launch a nested Linux development session.
- `cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-default`: validate the default shell package.
- `cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-cupertino-dock`: validate the derived Cupertino shell package.
- `cargo build --workspace`: compile every crate.
- `cargo test --workspace`: run unit and integration tests.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: reject lint regressions.

Use `nickel-plugin dev` for an isolated live package preview. When developing a derived shell, pass its base package first, for example `target/debug/nickel-plugin dev assets/plugins/nickel-default assets/plugins/nickel-cupertino-dock`.

On Windows development hosts, prefer installed Unix-style tools such as `rg`, `cat`, and `ps`
for routine searching, reading, and process inspection. Use PowerShell-specific commands only when
the task genuinely requires Windows APIs or no suitable Unix-style tool is available.

## Coding Style & Naming Conventions

Use stable Rust and standard `rustfmt` formatting. Name Rust modules, functions, and files in `snake_case`; types and traits in `UpperCamelCase`; constants in `SCREAMING_SNAKE_CASE`. Prefer explicit platform boundaries using `cfg(target_os = "...")`. Keep unsafe code localized, documented with a `SAFETY:` justification, and covered by focused tests.

Native application code, security boundaries, platform adapters, and shipped tooling must remain Rust. Production shell packages are the intentional exception: author their presentation canonically in TSX and CSS. Nickel accepts JS, JSX, TS, and TSX, transforms changed modules with Oxc, and executes cached JavaScript through direct V8 bindings. Nickel does not load React, Node, Deno, or a browser at runtime. Do not move trusted policy, capability checks, native resource ownership, or platform operations from Rust into package code. Do not introduce external TypeScript runtime dependencies, JavaScript build steps at application startup, or application components in Lua, Go, C, or C++.

For shell-package changes:

- Treat `plugin.json` as a validated authority declaration. Request only the capabilities and surfaces the package needs.
- Use composition exports and replacements for reusable or derived shell components instead of copying the default shell wholesale.
- Prefer `.tsx` for new and migrated authoring; `.js`, `.jsx`, and `.ts` remain compatible. Oxc transformation is package-load/change work and must never occur during render or interaction. Checked-in generated `.js` may remain where packaging still requires it, and must stay synchronized until that package migrates to a source entry.
- Use the host-provided component and hook vocabulary declared in `assets/plugins/nickel-plugin.d.ts`. Do not assume browser DOM APIs.
- Put visual styling in the package CSS where supported and preserve native ownership of input, layout, hit testing, effects, and protected data.

## Testing Guidelines

Place unit tests beside their modules and integration tests in the relevant crate's `tests/` directory. Name behavior tests descriptively, for example `recent_window_ranks_above_unused_match`. Test core logic with synthetic windows, notifications, clocks, and controller events. Gate platform adapter tests by target OS and record manual coverage for focus, DPI, multiple monitors, and permissions.

For new shell interaction behavior and regressions—especially launcher, focus, task switching, input,
surface identity, effect ordering, and multi-output behavior—prefer the semantic scenario harness in
`nickel-core::scenario` over bespoke state mutation, copied coordinates, or test-only reducers. Drive
semantic input through production reducers and hit testing, and assert production-owned state and
recorded effects. This is a default for new interaction tests, not a requirement to rewrite suitable
existing tests or to force pure unit, rendering, adapter-contract, and live platform tests into the
scenario harness.

For JSX shell behavior, add focused runtime or presentation tests for package loading, composition, capabilities, callbacks, layout, and effect dispatch. Validate every changed package, regenerate its checked-in JavaScript, and use a nested live session for interaction or visual changes that package validation cannot exercise. Keep native policy assertions in Rust tests; do not rely solely on snapshots of generated JSX trees. Record manual Windows or direct-session coverage when behavior depends on native shell ownership, compositor integration, thumbnails, focus, or platform permissions.

## Commit & Pull Request Guidelines

Use concise, imperative commit subjects such as `Add fuzzy ranking pipeline`. Keep commits scoped and include tests with behavioral changes. Commit new specifications when written; archive them when completed. Pull requests should explain user-visible behavior, list platforms tested, link relevant specifications or issues, and include screenshots or recordings for visual changes.
