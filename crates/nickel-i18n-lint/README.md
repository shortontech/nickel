# Nickel localization lint

Run the repository gate from the workspace root:

```text
cargo run -p nickel-i18n-lint -- --baseline assets/i18n-lint-baseline.tsv crates
```

The scan excludes the `nickel-ui-workbench` fixture crate and Rust `examples/`
directories. They are development surfaces; the shell and bundled applications
remain in scope. For an intentional literal in shipped UI, use a same-line or
preceding-line `nickel-i18n-lint: allow <reason>` comment.

The baseline fingerprints the complete sorted set of known findings without hiding their source
locations from an ordinary unbaselined run. Any addition, removal, or replacement changes the
fingerprint and fails CI, forcing the reviewer either to localize the affected text or deliberately
refresh the baseline after reviewing the full diagnostic list:

```text
cargo run -p nickel-i18n-lint -- crates
cargo run -p nickel-i18n-lint -- --print-baseline crates
```

Review the findings before refreshing the baseline so new shipped UI text does
not silently become accepted localization debt.
