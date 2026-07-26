# Localization String Lint

## Goal

Prevent untranslated user-facing text from entering Nickel interfaces while allowing ordinary
internal strings, protocol tokens, paths, logs, tests, and glyphs to remain normal Rust literals.

The lint is a Rust workspace tool named `nickel-i18n-lint`. It parses Rust with `syn`; it does not
use regular expressions as a substitute for syntax and does not depend on unstable compiler
internals.

## User-visible sinks

The first slice recognizes direct literals passed to known presentation APIs:

- Nickel components such as `Text::new`, `Button::new`, `ButtonLabel::new`,
  `RadioButton::new`, and `Header::new`.
- Glyphon buffer construction through Nickel helpers such as `text_buffer`.
- Native window-title methods such as `set_title` and `with_title`.
- Format and concatenation macros used directly as an argument to one of those sinks.

A literal is acceptable when the sink receives the result of a localization API rather than the
literal itself. Non-language symbols are not automatically exempt because a one-letter label can
still be user-facing.

Code under `#[cfg(test)]` is excluded. Fixtures may test arbitrary strings without creating
translation debt.

## Suppression

An intentional literal uses a source comment on the same line or the immediately preceding line:

```rust
// nickel-i18n-lint: allow icon-only control
Text::new("⌁")
```

The text after `allow` is required. Suppressions are local, reviewable, and searchable. File-wide
suppression and unexplained `allow` markers are not supported.

## Diagnostics and command

Run from the workspace root:

```text
cargo run -p nickel-i18n-lint -- crates
```

Diagnostics use compiler-style `path:line:column` locations and the stable code `NIL001`.
The process exits unsuccessfully when violations are present, making the command suitable for CI
once existing interface debt has been migrated or baselined.

## Analysis phases

### Phase 1: direct syntax

Parse `.rs` files, recognize direct sinks, unwrap references and conversion calls, and report
literal or formatting templates flowing directly into them. This phase must have focused fixture
tests before it is used as a repository gate.

### Phase 2: local data flow

Track immutable local bindings, struct fields explicitly annotated as presentation state, and
small wrapper functions around known sinks. Propagate three facts: localized, literal, and
unknown. Unknown values do not fail the lint.

### Phase 3: interprocedural flow

Build a call graph for workspace functions and component constructors. Collapse strongly connected
components with Tarjan's algorithm, then iterate facts over the resulting directed acyclic graph.
This makes recursive helpers converge without depending on traversal order.

The interprocedural pass remains deliberately conservative around traits, dynamic dispatch,
procedural macros, and unresolved external calls. A future compiler-backed implementation may
replace resolution internals without changing diagnostic codes or suppression syntax.

## Non-goals

- Validating Fluent grammar or catalog completeness; that belongs in `nickel-i18n`.
- Spell checking, capitalization policy, or visual text-fit checks.
- Parsing non-Rust application code.
- Treating every string assigned to a field named `status`, `title`, or `message` as visible.

## Verification

- Detect direct literals and direct `format!` templates at every supported sink.
- Accept localized expressions and dynamic model data.
- Respect reasoned suppressions on the same and preceding lines.
- Ignore `#[cfg(test)]` modules and functions.
- Recursively scan explicit files and directories while skipping `target` and `.git`.
- Produce stable diagnostic locations and a nonzero violation exit status.

## Completion

Archive this specification after the direct pass is enforced in CI and the local/interprocedural
phases cover Nickel's wrapper components with an acceptably low false-positive rate.
