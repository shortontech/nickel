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
On Windows, the shared policy tests and the focused terminal, application-scale, and launcher
favorites owner tests pass. Linux tests and target builds remain unrun under the current user
instruction. Formatting passes; strict Clippy remains blocked by warnings elsewhere in `nickel`.
