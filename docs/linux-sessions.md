# Linux Sessions

Run commands from the repository root.

## Linux Nested Session

Run Nickel inside an existing Linux desktop:

```bash
cargo run -p nickel --no-default-features --features backend-winit --bin nickel-nested
```

Live compositor tests may add `--test-control`. This explicitly enables the
capability-authenticated `TestInput` protocol command for the nested backend, allowing tests to
inject semantic keyboard and pointer events through the same Smithay input path as physical
devices. The direct backend additionally requires `NICKEL_ALLOW_NATIVE_TEST_CONTROL=1`; test
control is disabled by default on both backends.
Ordinary nested and native sessions do not bind a compatibility control socket or export a
capability token; compositor-owned shell and file UI use typed in-process authority instead.

With the `NICKEL_SESSION_CONTROL` and `NICKEL_SESSION_TOKEN` variables issued only by an explicit
`--test-control` session,
`nickel-test-input` can inspect registered windows and inject individual production input events:

```bash
cargo run -p nickel --bin nickel-test-input -- windows
cargo run -p nickel --bin nickel-test-input -- workspaces
cargo run -p nickel --bin nickel-test-input -- outputs
cargo run -p nickel --bin nickel-test-input -- surfaces
cargo run -p nickel --bin nickel-test-input -- caches
cargo run -p nickel --bin nickel-test-input -- move 64 700
cargo run -p nickel --bin nickel-test-input -- wheel 0 -120
cargo run -p nickel --bin nickel-test-input -- button left pressed
cargo run -p nickel --bin nickel-test-input -- button left released
```

The same capability provides semantic workspace commands, nested output hotplug, and lock-boundary
acceptance without copied coordinates or private state mutation. It cannot invoke logout or power
actions. Run `nickel-test-input --help` for the complete command set.

Renderer-owned shell targets can be exercised without copying panel or overlay coordinates:

```bash
cargo run -p nickel --bin nickel-test-input -- \
  semantic panel-app org.nickel.Terminal hover
cargo run -p nickel --bin nickel-test-input -- \
  semantic preview 10 menu
cargo run -p nickel --bin nickel-test-input -- \
  semantic menu 10 minimize
cargo run -p nickel --bin nickel-test-input -- \
  scenario grouped-windows org.nickel.Terminal
```

The shell resolves these names from its live grouping and preview/menu frame records. The compositor
then translates the returned surface-local point through its authoritative shell-surface placement
and injects normal pointer motion and button events. The capability endpoint is absent unless the
nested session was started with `--test-control`, requires the session token, and is removed from
ordinary application environments.

The grouped-windows scenario requires two disposable windows with the supplied application ID. It
discovers their compositor IDs, drives hover, peek, activation, close, minimize, maximize, and
restore through renderer-resolved targets, and polls authoritative window snapshots after each
transition. Because it deliberately closes and changes those windows, use it only with fixtures in
an explicitly test-controlled nested session.

The client does not mutate shell state directly: events still pass through the compositor's normal
hit testing, focus handling, and input reducers.

## Linux Direct Session

Protocol policies, compatibility evidence, and known limitations are tracked in
[`docs/linux-application-compatibility.md`](linux-application-compatibility.md).
The compositor lock authority, PAM boundary, and remaining native acceptance are documented in
[`docs/session-locking.md`](session-locking.md).

Linux local audio cues require the PipeWire development package to build and a running PipeWire service for playback.

The direct backend requires DRM, GBM, libinput, udev, libseat, and EGL development packages. Build
it without the nested backend:

```bash
cargo build -p nickel
```

Run it from a text VT:

```bash
RUST_LOG=info target/debug/nickel --backend udev
```

For an explicitly authorized native TTY test, run these two lines in separate tmux panes (the
second pane may replace `surfaces` with another `nickel-test-input` command):

```bash
NICKEL_ALLOW_NATIVE_TEST_CONTROL=1 NICKEL_SECURE_STORAGE_REQUIRED=0 NICKEL_TEST_CONTROL_ENV_FILE=/tmp/nickel-test-control.env target/release/nickel --backend udev --test-control
nickelas surfaces
```

`NICKEL_SECURE_STORAGE_REQUIRED=0` is only for an isolated TTY diagnostic without the normal
wallet service. Remove `/tmp/nickel-test-control.env` after the test; ordinary SDDM sessions must
not use either override.

Set `NICKEL_DRM_DEVICE=/dev/dri/cardN` to select a specific GPU.

## Linux Login Session

Build the integrated compositor, shell, and login launcher:

```bash
cargo build --release -p nickel
```

Install the completed build as an SDDM Wayland session:

```bash
sudo packaging/install-nickel-session.sh
```

The installer resolves the checkout containing the script rather than assuming a fixed repository
path. `NICKEL_RELEASE_DIR` may select another completed release directory, and
`NICKEL_INSTALL_ROOT` stages the exact installed layout under a temporary packaging root.

To remove the session, delete only the files installed by the script:

```bash
sudo rm /usr/share/wayland-sessions/nickel.desktop
sudo rm /usr/share/applications/nickel-settings.desktop
sudo rm /usr/share/icons/hicolor/512x512/apps/nickel-settings.png
sudo rm /usr/local/bin/nickel-login /usr/local/bin/nickel
sudo rm /usr/local/bin/nickel-settings /usr/local/bin/nickel-terminal
```

If a development build cannot start, select another desktop from SDDM's session chooser. From that
desktop, inspect the previous boot with `journalctl -b -1 | rg 'nickel|sddm-helper'`, rebuild
Nickel, and rerun the installer. A compositor startup failure exits back to
the display manager; an intentional logout exits successfully. Do not replace the installed binaries
with symlinks into `target/`: a later default-feature build can replace the direct-backend binary.

Nickel asks the user D-Bus session for its configured `org.freedesktop.secrets` provider; it does not
select or start a KWallet-, GNOME Keyring-, or KeePassXC-specific service. The operating system may
use a provider-specific PAM module to unlock the wallet at login. Providers without PAM integration
remain supported through the standard Secret Service unlock prompt, but automatic login-password
unlock is not universal. Nickel verifies the existing default collection, exposes readiness to the
shell, warns before launching known credential-dependent applications while storage is unavailable,
and never creates a replacement collection.

To pin reconnections to a specific provider, place its absolute executable path in
`$XDG_CONFIG_HOME/nickel/secret-service-provider` (or
`~/.config/nickel/secret-service-provider`). Nickel rejects a different process taking ownership of
`org.freedesktop.secrets`; without this optional pin it reports the current owner for diagnosis but
does not persist an automatic selection.
