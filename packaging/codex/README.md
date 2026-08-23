# Bundled Codex CLI

Nickel release packaging stages a pinned native Codex CLI; ordinary Cargo builds do not download it.
The manifest records both the upstream npm archive digest and the extracted executable digest for each
supported target.

Prepare an offline release cache containing the six exact archive names in `manifest.toml`, then run:

```text
cargo run -p nickel-codex --bin nickel-codex-bundle -- \
  --manifest packaging/codex/manifest.toml \
  --archives /absolute/offline/archive-cache \
  --output /absolute/release-root \
  --target TARGET-TRIPLE \
  --license LICENSE-APACHE
```

The Rust command verifies the archive, extracts only the pinned native executable, verifies that member
independently, marks it executable on Unix, and installs the full Apache license and provenance manifest.
A missing or changed artifact fails the release build.

To update Codex, change the version and revision, obtain every declared target artifact in a private
temporary cache, recompute both digest layers, run schema comparison and the safe live probe, review the
protocol diff, and exercise staging for each archive before committing. Never place downloaded archives
or native binaries in the source tree.
