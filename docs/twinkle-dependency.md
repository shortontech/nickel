# Twinkle dependency

Nickel consumes the independent [Twinkle workspace](https://github.com/shortontech/twinkle) through one exact Git revision in the root Cargo.toml and Cargo.lock. Engine, input, presentation, protocol, theme, testkit and the winit fork share that source. There is no local authoritative Twinkle source tree or winit patch. Keep the winit source/revision coherent with Twinkle so native event and window types remain identical.

Nickel owns `nickel-ui-host`, `nickel-jsx-host`, its consumer workbench, session/compositor policy, localization and shell packages. Upstream owns generic examples, fixtures, declarations, CI and performance workloads. Guide-button requests are generic `HostMenu` events; Nickel explicitly maps them to launcher policy.

When updating the pin, copy upstream `crates/twinkle-jsx-runtime/types/twinkle.d.ts` to `assets/plugins/twinkle.d.ts` and update its JSON provenance revision and SHA-256. This is a packaged declaration artifact, not a runtime source copy. The Nickel declaration entrypoint references that artifact and adds its own domain APIs.

The history-preserving import and all local commits are prepared in `/external/projects/twinkle`. Local Git-pin validation used a command-scoped URL rewrite to fetch the unpublished commit from that checkout. No global Git configuration changed. A fresh remote checkout will require publishing the pinned upstream commit first. No remote was pushed by this work.

Independent Linux build/test coverage and downstream host coverage are recorded in active specs 0273–0278. Physical controller, Windows and manual multi-output interaction coverage remain pending. Existing Nickel inventory/workbench assertions and Cupertino CSS validation failures are recorded separately; extraction does not widen their limits.
