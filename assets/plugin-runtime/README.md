# Nickel JSX runtime

The generic [`bootstrap.js`](https://github.com/shortontech/twinkle/blob/aa0a2c3a00884a7fea48cad09ca579717be99250/crates/twinkle-jsx-runtime/src/bootstrap.js) defines native components, hooks, application data, and render/event transactions. Nickel installs its own [`host bootstrap`](../../crates/nickel-jsx-host/src/bootstrap.js) for Settings registration and the public `nickel` service clients inside a package context.
Nickel uses Oxc to erase TypeScript and lower JSX, then runs the cached JavaScript
through direct V8 bindings without Node, Deno, a browser, or a DOM. TSX is the
canonical authoring format; JavaScript, JSX, TypeScript, and CSS remain supported
package files.

Rust supplies bounded capability snapshots and validates effects before native
execution. Presentation and interaction logic belong in JSX; a custom Settings
page uses the same components and capability functions as any other package UI.
The optional Settings window in [nickel-default](../plugins/nickel-default/)
reads registered settings and custom pages. Its controls are replaceable public
components and inherit overridable CSS variables.

`twinkle-jsx-runtime` owns module loading, hooks, render/event transactions,
and generic component linking. `nickel-jsx-host` owns admitted shell composition. Packages have isolated JavaScript contexts. Foreign
component callbacks retain their producing package's authority; a parent grant
does not authorize its replacement's effects. Native component parsing, layout,
input, rendering, and operating-system services remain host-owned.

The `nickel-default` Settings window uses the shared package runtime and public
APIs. See the active shell-package spec for remaining composition and ABI work.

## React-familiar component lifecycle

Nickel implements a small retained component runtime; it does not embed React.
Function components may use `useState`, `useReducer`, `useRef`, `useEffect`,
`useMemo`, `useCallback`, `useId`, `useContext`, and the Nickel store hooks.
`memo` and `ErrorBoundary` provide retained prop comparison and local failure
containment. Hook calls must be unconditional and in the same order on every
render of a component.

State setters and reducer dispatchers have stable identity. Functional state
updates receive the last admitted value, reducer actions retain dispatch order,
and updates admitted during one event or passive-effect turn are batched. An
`Object.is`-equal result does not dirty the component. Updates from an unmounted
component are ignored. A local update executes its retained owner and the
minimum reconciliation ancestry; it does not rerender unrelated components or
surface mounts.

`useMemo` and `useCallback` compare dependencies with `Object.is`. They are
performance tools, not durable storage: Nickel may discard their values when a
component unmounts or an incompatible hot replacement occurs. `useRef` retains
one mutable object for the compatible mounted component lifetime. `useId`
returns a deterministic opaque ID scoped to the package, mount, component, and
hook slot; it is suitable for accessibility relationships but conveys no native
authority.

Effects run synchronously after the native tree has committed. An effect never
runs for an abandoned or rejected render. Cleanup runs before a changed effect
runs again and on unmount, package retirement, or incompatible hot replacement.
An omitted dependency list reruns after every committed render; an empty list
runs once per compatible mount. Effect failures are reported to the nearest
`ErrorBoundary`. Effects must delegate long-running work to a bounded native
operation; the runtime does not provide browser timers, promises with ambient
I/O, or a background event loop.

Contexts are package-local. Providers scope ordinary descendant components,
including keyed moves, but context objects cannot be passed across package
ownership boundaries. Pass a bounded selected value as a prop to a foreign
component instead. `memo` defaults to shallow `Object.is` prop comparison;
component-local state, context, and selected-store changes always pierce it.

`ErrorBoundary` preserves the last safe surrounding surface and renders its
fallback for component render, reducer, memo, selector, effect, and cleanup
failures. A fallback function receives the bounded error and a reset callback.
Changing a member of `resetKeys` also requests reset. Native capability and
surface authority are still validated by the host; catching an error never
grants authority.

## Versioned native stores

Use the domain hooks rather than copying broad `nickel.data` snapshots into
component state:

- `useWindows(selector?)` and `useActiveWindow()`
- `useWindowPreviews(selector?)` and `useWindowMenu(selector?)`
- `useApplications(selector?)`
- `useNotifications(selector?)`
- `useWorkspaces(selector?)` and `useWorkspace()`
- `useOutputs(selector?)` and `useOutput()`
- `useTheme(selector?)` and `useReducedMotion()`
- `useCapability(capability)`
- `useSurface()`, `useScaleFactor()`, and `useSurfaceFocus()`
- `useLocale()`

Snapshots are immutable and retain identity within a store generation.
Selectors are compared with `Object.is`, so a component rerenders only when its
selected result changes. Selectors should therefore return an existing scalar
or snapshot member, or explicitly memoize a derived object. A selector grants
no additional capability. Native actions separately revalidate package,
surface, generation, revision, and capability authority when invoked.

Unavailable host observations are explicit. Depending on the hook they appear
as `null`, `available: false`, `known: false`, or a null member; Nickel does not
invent an OS value. `useSurface()` is scoped to the exact mount, so two surfaces
from one package have distinct surface snapshots and hook state. Replicated
all-output mounts intentionally have no singular output.

`useSyncExternalStore` is deliberately narrower than React's API. It accepts
only a matching `subscribe` and `getSnapshot` pair from a branded
`NickelStores` entry. Arbitrary JavaScript subscriptions are rejected. The host
rechecks store generations before commit so one committed tree cannot combine
torn snapshots.

## Keys, commits, and hot reload

Use stable `key` values for dynamic or reorderable component and native-child
lists. Unkeyed static children use positional identity. Duplicate or missing
dynamic keys are development errors because positional churn loses retained
state and native identity.

Successful updates cross the JavaScript/native boundary as bounded typed
mutations. Event callbacks receive owner-bound handles during admission; there
is no post-render action-ID recovery pass. Validation is transactional: a
rejected mutation keeps the last admitted native subtree, handlers, hook state,
subscriptions, and effects active.

Development hot reload preserves state only when owner, module/export identity,
component kind, key, and hook signature remain compatible. Incompatible
replacement runs cleanup and starts a fresh component incarnation. Evaluation
or validation failure leaves the accepted package graph running.

## Intentional differences from React

- Nickel has no DOM, portals, synthetic browser events, Node globals, Suspense,
  concurrent rendering, server rendering, or arbitrary external-store bridge.
- Effects are bounded post-commit work in the package capability context, not a
  general asynchronous runtime.
- Context objects stop at package ownership boundaries.
- Native components, layout, focus, input, accessibility, and capability checks
  remain host-owned; component code declares them but cannot retain native
  objects directly.
- A successful render is admitted transactionally as typed native mutations.
  Failed validation rolls the JavaScript lifecycle back to the last native
  commit.

The runtime TypeScript entry point is [`index.d.ts`](index.d.ts), which references the authoritative
[`nickel-plugin.d.ts`](../plugins/nickel-plugin.d.ts) ambient declarations. The compiler, validation,
nested-development, and layout-inspection commands are documented in
[`docs/plugins.md`](../../docs/plugins.md).

The copied `assets/plugins/twinkle.d.ts` is the canonical upstream declaration artifact. Its repository revision, source path and SHA-256 are recorded in `assets/plugins/twinkle-declaration-provenance.json`; update it together with the pinned workspace dependency.
