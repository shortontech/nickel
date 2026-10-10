# JSX API ownership during extraction

This inventory records implemented ownership and remaining boundaries. It does not certify extraction completion.

| Contract | Implementation owner | Boundary |
| --- | --- | --- |
| Native element/property vocabulary, style values and admission | `twinkle-presentation`, `twinkle` | Bounded tree requests; native layout, input and hit testing remain Rust-owned |
| `h`, Fragment, state/reducer/ref/context/memo/callback/effect hooks | `twinkle-jsx-runtime` bootstrap | Generic transaction, identity and lifecycle semantics |
| `twinkle.data`, `twinkle.request`, `twinkle.component` | Generic bootstrap | Ordinary host data, queued effect requests and admitted export lookup; no action dispatcher is installed implicitly |
| Locale, theme, reduced motion and surface hooks | Generic bootstrap | Effective presentation observations; no saved-preference mutation |
| `TwinkleStores`, `useSyncExternalStore`, `useHostCapability` | Generic bootstrap | Branded subscriptions, selected values, explicit host vocabulary and generation checks |
| Windows, applications, notification, workspace, output topology, preview and menu stores/hooks | `nickel-jsx-host` bootstrap and `stores.rs` | Nickel schemas and filtered snapshots; host output resolver adapts topology for generic surface observations |
| `nickel` service clients and protected operation request schemas | Nickel host bootstrap | Grant-filtered service availability and bounded requests; Rust revalidates authority before executing effects |
| `NickelStores`, `useCapability` | Nickel host bootstrap | Compatibility names composed over generic machinery; no standalone Nickel capability union |
| Settings registration, metadata, pages, values, invocation and retirement | Nickel bootstrap and `settings.rs` | Nickel validates provider identity, registry membership, types and bounds |
| Checkpoint traversal, native trees, hooks, subscriptions, effects and rollback | Generic bootstrap/runtime | Host domains participate through bounded capture/restore hooks; registration seals before package initialization |
| `HostBindings`, `DataPublisher`, projection reuse | Generic `bindings.rs`/runtime | Versioned immutable Rust-owned initialization; bounded bootstrap/projection names, duplicate rejection; caches know registered names rather than desktop schemas |
| Default data-to-desktop-store adaptation and appearance configuration projection | Nickel `stores.rs` | Explicit registered publisher; standalone data does not imply desktop grants or shell schemas |
| Module normalization, limits, Oxc preparation/cache and relative linking | Generic `modules.rs` | No installed catalog discovery |
| Catalog resolution, package identities, provider ownership, Settings integration | Nickel composition modules | Admitted downstream policy and authority |
| Composition client prelude and Settings aliases | Nickel `composition_bindings.js` / `module_bindings.rs` | Generic module preparation accepts bounded shared/per-module host preludes; component proxy mechanics remain generic |
| Generic ambient declarations and host declaration composition | `types/twinkle.d.ts`, downstream `nickel-plugin.d.ts` | Neutral native vocabulary, generic hooks and store types; Nickel composes service/grant/Settings declarations and compatibility aliases |
| Standalone TSX preview and controller exercise | Optional `twinkle-workbench` JSX preview and generic TSX/CSS fixture | Production native semantic/keyboard/controller navigation, keyed state, selected stores and rollback pass; physical gamepad/Windows acceptance remains unverified |

The runtime and presentation unit fixtures use generic application APIs. Desktop schema, service, provider and shipped package fixtures live in Nickel's host tests. Desktop release-admission workloads moved to `nickel-jsx-host/tests/release_admission.rs`; generic workload coverage in that suite must be separated into reusable engine-owned coverage before extraction sign-off. No release timing or memory baseline has been rerun by this move.

Internal `__nickel` names and stable `useId` spellings remain implementation compatibility identities. They grant no host authority and do not imply an installed shell service. Renaming them is not required to remove dependency direction; public standalone globals are checked separately.
