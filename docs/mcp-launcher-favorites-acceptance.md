# Launcher favorites acceptance

The installed-application favorites domain exposes bounded read, add, remove and
reorder operations to full-session full-debug leases. It resolves exact installed
catalog IDs and retains unavailable stored favorites without exposing their IDs or
recent-history contents. Both local launcher actions and remote transactions use
`PreparedLauncherPreferences` and the same persistent sibling transaction lock.
Worker preparation reads bounded regular files and stages replacement contents;
the owner validates the file revision and original authority immediately before
rename. Accepted changes reconcile the launcher and panel models even if the reply
is subsequently cancelled. Filesystem metadata operations remain OS-bound, and
uncooperative external writers do not participate in the advisory lock.

On 2026-09-10, an owned nested X11 session ran under Xvfb display :92 with private
XDG config/data/state/runtime directories and audible indications disabled. Its
MCP endpoint was loopback port 42699. Three fixture desktop entries used `/usr/bin/true`
but no application was launched. Real MCP calls verified:

- Add, remove and reorder return the committed favorites and applied model state.
- Stale configuration generation, catalog generation and prior list are rejected.
- Arbitrary executable paths and invalid reorder membership are rejected.
- An independently held sibling lock prevents mutation; releasing it permits the
  next fresh transaction.
- Unknown stored favorites and a private recent-history canary survive changes,
  while responses disclose neither value.
- Production launcher semantic nodes show the two committed fixture favorites
  and exclude the removed favorite.
- A surface-scoped semantic action opens the production launcher item menu and
  commits its advertised unpin action through the guarded favorites owner,
  preserving the private recent-history record.
- Ordinary full-session and narrow surface leases cannot access the domain.
- External file replacement invalidates an earlier observed generation.
- Revocation rejects subsequent reads and writes without changing preferences.

Both owned compositor and Xvfb processes terminated after acceptance. Evidence is
in `/tmp/nickel-mcp-favorites-native/acceptance.log`, with the production semantic
tree in `launcher-tree.json` and fixture scripts in the same directory.

The earlier focused validation passed 98 LiveShell tests (three ignored), seven
core preferences tests, two remote favorites tests, DTO validation, UI consumer
inventory and strict Clippy for Nickel/core/storage/remote-control. The checked
commit unit test covers authority rejection immediately before rename; native
acceptance above does not claim an injected revocation race at that exact seam.
Local pin and retry behavior is covered by production LiveShell interaction tests;
the native run verifies remote mutation and production launcher observation.
Native Windows and independent pixel confirmation of panel updates remain
unavailable in this acceptance run. Semantic TogglePin integration is separate.

Final follow-up validation passed all three source reuse authority/inventory tests,
all 16 nickel-storage tests, `cargo fmt --all --check` and `git diff --check`.
The audit required restoring the explicit `atomic_write` import; this was an
import-only correction after the native binary build. The check transcript is
`/tmp/nickel-favorites-inventory-storage-tests.txt`.
