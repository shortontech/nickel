# Native JSX declarations

`twinkle.d.ts` is the canonical generic component, hook, effect-request and presentation observation vocabulary. It contains no Nickel manifest, capability union, session protocol or desktop service schema. Hosts compose additional declarations over this file; Nickel retains its previous public type spellings as compatibility aliases.

These are native components, so use an ES library without DOM declarations. The ambient names `Window`, `Text`, `Image` and `Option` intentionally describe native controls. Oxc prepares TSX at load/change time; no TypeScript runtime or browser is used by the application.

The standalone fixture's optional editor/type check is:

```sh
tsc -p crates/twinkle-jsx-runtime/examples/tsconfig.json
```

The runtime suite checks generic declaration names against its public bootstrap globals and verifies Oxc preparation/rendering of the standalone TSX/CSS fixture. Native preview/input acceptance is a separate gate.
