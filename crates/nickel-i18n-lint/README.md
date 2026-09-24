# Nickel localization lint

Run the repository gate from the workspace root:

```text
cargo run -p nickel-i18n-lint -- crates
```

The scan excludes the `nickel-ui-workbench` fixture crate and Rust `examples/`
directories. They are development surfaces; the shell and bundled applications
remain in scope. For an intentional literal in shipped UI, use a same-line or
preceding-line `nickel-i18n-lint: allow <reason>` comment.

The command reports every detected hardcoded UI string and fails while any
findings remain. There is no accepted-findings baseline. Use a narrow
`nickel-i18n-lint: allow <reason>` comment only when a literal does not need
translation.
