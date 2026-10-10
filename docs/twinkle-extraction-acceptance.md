# Twinkle extraction acceptance

Status: active. Nickel `feature/twinkle` consumes the independent Twinkle
repository at `aa0a2c3a00884a7fea48cad09ca579717be99250`. Implementation and
local validation are complete for the boundaries below; publication and manual
platform/controller sign-off remain outstanding. Specs 0273–0276 remain active.

## Implemented boundaries

Twinkle owns the native engine, input, font/render assets, macros, testkit,
workbench, executor-neutral protocol, theme, optional JSX runtime and native
presentation. Its ten-member workspace has no Nickel dependency in all-feature,
all-platform Cargo metadata. The generic presentation default and ordinary Rust
engine do not require V8/Oxc.

Nickel owns session/compositor/controller authority and transport, localization,
package manifests/admission, catalog composition policy, Settings and desktop
stores/actions, shell packages, applications and consumer fixtures. Its native
adapters use the same Twinkle engine. Multi-owner execution, mounts, opaque
references, callback/child grants, checkpoints, expansion, patches and execution
reload have one authoritative implementation upstream.

The history-preserving import, required licensed winit fork, source provenance
and exact downstream pin are committed locally. Nickel contains no authoritative
copy of migrated Twinkle or winit sources. The packaged declaration artifact is
byte-identical to upstream and has revision/SHA provenance.

## Current verification

| Command or evidence | Result | Scope |
| --- | --- | --- |
| Independent `cargo test --workspace` | 848 passed, 0 failed, 14 ignored, 28 suites | All default workspace tests; ignored release workloads are not counted as passed |
| Independent `cargo check --workspace --all-targets --all-features` | Passed | Linux normal, development and optional targets |
| Independent all-feature Cargo metadata | No Nickel packages | Includes target-specific and development dependency closure |
| `cargo check --workspace --exclude twinkle-jsx-runtime --all-targets --target x86_64-pc-windows-gnu` | Passed | Native Windows GNU compilation; default features exclude optional V8 |
| `cargo check --workspace --all-targets --all-features --target x86_64-pc-windows-msvc` | Passed | Windows MSVC compilation, including optional V8/JSX; no native Windows execution |
| Nickel workspace all-targets/all-features check | Passed | Production adapters and applications; existing unrelated `nickel-file` unused-variable warning remains |
| Nickel host runtime unit tests | 124 passed | Production generic manager plus Nickel policy/provider/Settings/security fixtures |
| Independent runtime unit tests | 117 passed | Generic hooks, composition, provider isolation, rollback, retirement and reload |
| Nickel hosted-controller tests | 24 passed | Production lease, routing and protected transition scenarios |
| Nickel UI-host unit tests | 4 passed | Session transport/reset and supplied localization adapter |
| Default production package validation | Passed | Default TSX/CSS package admission |
| Isolated nested acceptance harness | Passed | Default shell, optional JSX Settings, scoped Meta, sibling/dialog/overlay lifecycle, screenshots, socket layouts and clean shutdown |
| Optional standalone JSX preview tests | 11 passed | Keyed state, stores, callbacks, editing, keyboard/controller semantics and native admission |
| Optional JSX presentation suite | 87 passed | Native materialization and JSX integration |
| DMA-BUF readiness regressions | 3 passed | Producer readiness, immediate-ready commit and synchronized-parent admission |
| Rust/native presentation dependency trees | No Nickel, V8, Oxc or JSX runtime | Default Rust consumer and no-default-features presentation |
| Shell integration tests | 12 panel and 3 CSS tests passed | Downstream presentation and shipped CSS contracts |
| Release admission workloads | 4 generic and 2 Nickel workloads passed | Keyed insert/remove/reorder, leaf hooks, lifecycle churn and independent mount/store revisions |
| Aliased Windows consumer | Passed | Separate workspace uses `lights` to compile native UI/macros for Windows MSVC |
| Minimum Rust 1.96 all-targets/all-features check | Passed on Linux and Windows MSVC | Declared minimum toolchain, including optional V8/JSX |
| Native retained-render release unit workloads | 7 passed, 1 failed | Existing 2,000-node retained-layout timing gate also fails on isolated pre-extraction revision; spec 0280 |
| Native cache and overlay release workloads | 4 passed | Cache admission, long-Unicode selection and retained-overlay bounds |
| Selected package formatting and declaration bytes/SHA | Passed | Changed native code and packaged declaration provenance |

Commands ran from the corresponding repository roots. The nested harness builds
`nickel-nested`, `nickel-test-input`, `nickel-test-tray` and
`nickel-nested-acceptance` with `--no-default-features --features backend-winit`,
then runs `target/debug/nickel-nested-acceptance`. It creates isolated runtime and
configuration directories and shuts down its own compositor.

## Outstanding evidence and known issues

- Publish the pinned Twinkle revision before fresh remote Nickel checkouts can
  fetch it. Local verification used a command-scoped URL rewrite; no global Git
  configuration or remote push was performed.
- Nickel's full Windows hosted cross-check could not finish here: its native
  `mozjpeg-sys`/`ring` builds require Windows CRT headers. The initial attempt
  lacked `lib.exe`; selecting actual `clang-cl` and `llvm-lib` reached the missing
  `assert.h`/CRT-header prerequisite. No dependency or compiler checks were
  bypassed. Twinkle's independent Windows checks passed; this does not imply
  Nickel's hosted Windows build passed.
- An independent Windows CI job is prepared upstream on `windows-2025` with
  Rust 1.96, full feature/target checks, serialized full tests and aliased macro
  validation. It has not run; publication is still pending.
- Physical controller hotplug, two simultaneous devices, held input across
  direct-session focus/ownership changes, manual multi-output behavior and native
  Windows hosted/standalone interaction have not been verified here.
- The nested harness is evidence for its listed assertions. It does not prove
  every Alt-Tab, physical-input, direct-session or multi-output requirement.
- Release performance/cardinality evidence must be audited separately from the
  default suite's ignored workloads. Current focused release results are saved in
  [twinkle-release-evidence.json](twinkle-release-evidence.json), with serialized
  builds and six passing deterministic workloads. No comparable pre-extraction
  timing baseline is claimed. The native retained-layout timing failure is explicitly reproduced on the
  pre-extraction revision and recorded in active spec 0280. Other ignored
  workloads remain individually open.
- Existing Nickel authority/inventory/workbench assertions and the derived
  Cupertino package's unsupported CSS property failure were recorded before
  extraction. Their limits were not widened to obtain green results.
- Additional active specs 0277–0280 track generation exhaustion and development
  reload recovery. Isolated candidate preflight does not make later installation
  failures transactional over arbitrary package globals.

The compositor's DMA-BUF handler file is unchanged from the pre-extraction
readiness fix `a42901f9`;
this extraction does not move compositor buffer admission into Twinkle.
