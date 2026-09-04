# Nickel localization lint

Run the repository gate from the workspace root:

```text
cargo run -p nickel-i18n-lint -- --baseline assets/i18n-lint-baseline.tsv crates
```

The baseline fingerprints the complete sorted set of known findings without hiding their source
locations from an ordinary unbaselined run. Any addition, removal, or replacement changes the
fingerprint and fails CI, forcing the reviewer either to localize the affected text or deliberately
refresh the baseline after reviewing the full diagnostic list:

```text
cargo run -p nickel-i18n-lint -- crates
cargo run -p nickel-i18n-lint -- --print-baseline crates
```

Intentional literals should normally use the narrower same-line or preceding-line suppression
described by `nickel-i18n-lint: allow <reason>` instead of expanding repository debt.
