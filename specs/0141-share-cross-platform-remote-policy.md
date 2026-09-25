# Spec 0141: Share Cross-Platform Remote Policy

## Problem

The Linux session owner and Windows desktop owner implement the same Nickel-owned remote
settings policy in separate files. Terminal presentation conversion, application scale policy
conversion, and launcher favorite edits are repeated. Their native observation, authorization,
and commit paths legitimately differ, but copying pure decisions makes drift likely and requires
the same behavior to be tested twice.

## Decision

Move platform-neutral projection, validation, and requested-state transformations into shared
Rust modules. Keep native application identity resolution, toolkit access, file observation,
transaction staging, authorization, deadlines, and commit ownership in their existing platform
paths. Both owners must call the same production policy functions.

## Required behavior

### Terminal presentation

- Share conversion between `TerminalSettings` and remote `Preferences`, cursor-style mapping,
  validation, and application of requested presentation fields.
- Preserve `default_shell` and `initial_working_directory` exactly; neither may enter the remote
  preferences payload.
- Keep generation and revision checks, protected-state checks, and durable commit sequencing with
  each desktop owner.

### Application scale

- Share conversion between `ApplicationScalePolicy` and remote `Policy`, including the supported
  custom scale range and step.
- Share projection of observed policy and generation decisions where inputs and semantics match.
- Keep Linux GTK/Qt reads and writes in the Linux backend. Windows continues to report those
  external toolkit controls unavailable while preserving their stored ownership and pending
  intent fields.
- A malformed remote policy must fail before either owner stages a settings write.

### Launcher favorites

- Share add, remove, reorder, deduplication, favorite limit, and unavailable-count policy.
- Pass an explicit catalog resolver into the shared policy. Linux desktop-entry aliases and
  Windows case-insensitive application IDs retain their existing native identity rules.
- Preserve hidden or currently uninstalled favorites and recent history during edits. Reorder
  only the visible installed subset and reject incomplete or repeated requested IDs.
- Keep catalog generation, file revision, and authorization checks with each owner.

## Boundaries

- No platform-specific API or `cfg(target_os)` branch belongs in the shared policy functions.
- Do not combine the Linux compositor owner and Windows winit owner into one transaction loop.
- Preserve existing remote protocol shapes and user-visible success and error categories.

## Verification

1. Run the same table-driven policy cases against both owners' production shared functions:
   valid and invalid terminal values, custom scale boundaries, catalog aliases, duplicate
   favorites, hidden favorites, and incomplete reorder requests.
2. Keep focused Linux and Windows tests for stale revisions, changing catalogs, protected input,
   deadlines, and commit boundaries. Verify that private terminal launch fields and toolkit
   ownership journals survive updates.
3. Confirm both target builds, workspace tests, formatting, and strict Clippy pass. Record native
   behavior on each platform where an adapter is exercised.

Completion requires the three policy families to have one production implementation each, with
platform differences expressed through typed inputs or narrow resolvers rather than copied
business rules.

## Current verification record

Both owners call the three shared policy families in `crates/nickel/src/remote_policy.rs`.
Application-scale observation generation, stale-state matching, and snapshot projection now use
the same production functions. On Windows, their shared test and focused application-scale owner
tests pass, along with the earlier terminal and launcher favorites tests. Linux tests and target
builds remain unrun under the current user instruction. Windows formatting and the exact strict
workspace Clippy command pass. Linux-only acceptance targets have Windows entry points so the
workspace check can compile on Windows. The expanded shared policy table covers terminal
validation before mutation, private launch fields, every supported custom scale step, alias
resolution, duplicate Add, hidden favorite preservation, and incomplete or repeated reorders.
The shared policy tests pass on Windows. `cargo test --workspace --no-run` builds every
workspace test target on Windows; it does not execute the suite.

The owners now also use shared toolkit capability projection, ownership and pending-intent
projection, and native-to-remote outcome mapping. Windows passes an unavailable backend; Linux
supplies its existing backend reads and retains its worker/commit owner. Eight shared policy tests
and both focused Windows application-scale owner tests pass. Formatting and the exact Windows
workspace strict Clippy command pass after this consolidation. The Linux path has not been built
or tested under the current user instruction.

Terminal snapshot projection is also shared now. Both owners derive the configured presentation
and the two private launch-field presence flags from the same function; neither private value
enters the remote payload. The shared policy suite and both focused Windows terminal-owner tests
pass. The Linux owner retains its native revision, authorization, and commit sequence.

Launcher favorites snapshots now use one shared projection as well. It carries the visible
favorites, unavailable count, catalog generation, and runtime-applied flag without exposing
hidden stored entries. The shared suite and all three focused Windows favorites-owner tests pass.
