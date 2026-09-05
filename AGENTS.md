# Repository Guidelines

## Project Structure & Module Organization

Nickel is an all-Rust, cross-platform desktop shell targeting Windows and Linux. Organize it as a Cargo workspace with small crates under `crates/`:

- `crates/nickel-core/`: platform-neutral application state and domain logic.
- `crates/nickel-search/`: indexing, fuzzy matching, and result ranking.
- `crates/nickel-ui/`: `winit` event handling, `wgpu` rendering, and widgets.
- `crates/nickel-platform/`: narrow Windows and Linux adapters.
- `assets/`: fonts, shaders, icons, and test fixtures with compatible licenses.
- `tests/`: workspace-level integration and platform contract tests.
- `specs/`: active design specifications; move completed specifications to `specs/done/`.

Keep search, ranking, navigation, and task-switcher policy independent of native window APIs so they remain deterministic and portable.

## Build, Test, and Development Commands

Use standard Cargo commands from the workspace root:

- `cargo run`: launch the development shell.
- `cargo build --workspace`: compile every crate.
- `cargo test --workspace`: run unit and integration tests.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: reject lint regressions.

Add platform-specific commands here when packaging and test harnesses are introduced.

On Windows development hosts, prefer installed Unix-style tools such as `rg`, `cat`, and `ps`
for routine searching, reading, and process inspection. Use PowerShell-specific commands only when
the task genuinely requires Windows APIs or no suitable Unix-style tool is available.

## Coding Style & Naming Conventions

Use stable Rust and standard `rustfmt` formatting. Name modules, functions, and files in `snake_case`; types and traits in `UpperCamelCase`; constants in `SCREAMING_SNAKE_CASE`. Prefer explicit platform boundaries using `cfg(target_os = "...")`. Keep unsafe code localized, documented with a `SAFETY:` justification, and covered by focused tests.

Nickel application code and shipped tooling must remain Rust: do not introduce JavaScript, TypeScript, Lua, Go, C, or C++ application components.

## Testing Guidelines

Place unit tests beside their modules and cross-crate tests in `tests/`. Name behavior tests descriptively, for example `recent_window_ranks_above_unused_match`. Test core logic with synthetic windows, notifications, clocks, and controller events. Gate platform adapter tests by target OS and record manual coverage for focus, DPI, multiple monitors, and permissions.

For new shell interaction behavior and regressions—especially launcher, focus, task switching, input,
surface identity, effect ordering, and multi-output behavior—prefer the semantic scenario harness in
`nickel-core::scenario` over bespoke state mutation, copied coordinates, or test-only reducers. Drive
semantic input through production reducers and hit testing, and assert production-owned state and
recorded effects. This is a default for new interaction tests, not a requirement to rewrite suitable
existing tests or to force pure unit, rendering, adapter-contract, and live platform tests into the
scenario harness.

## Commit & Pull Request Guidelines

Use concise, imperative commit subjects such as `Add fuzzy ranking pipeline`. Keep commits scoped and include tests with behavioral changes. Commit new specifications when written; archive them when completed. Pull requests should explain user-visible behavior, list platforms tested, link relevant specifications or issues, and include screenshots or recordings for visual changes.
