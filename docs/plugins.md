# Developing a Nickel panel plugin

Nickel loads a compiled JavaScript entry from a directory containing
`plugin.json`. The [bundled hello panel](../assets/plugins/hello-panel/) is a
minimal working example. The [taskbar](../assets/plugins/taskbar/) shows how a
first-party plugin requests host actions.

Build the Rust development tools, then start an isolated nested session on
Linux:

```sh
cargo build -p nickel --bin nickel-plugin --bin nickel-nested --features backend-winit
target/debug/nickel-plugin dev assets/plugins/hello-panel
```

The command validates the manifest and JavaScript, stages the package in a
temporary Nickel profile, and launches `nickel-nested`. Saving `plugin.json` or
the declared JavaScript entry validates and restarts the nested session. An
invalid edit prints its error and leaves the previous session running. Press
Ctrl+C to stop and remove the temporary profile. The developer command
currently supports one panel surface; `nickel-plugin validate <directory>`
works without launching a shell.

The installed entry is plain JavaScript. JSX and TypeScript are development
source formats: compile them to the declared `.js` entry before Nickel loads
the package. For example, an external `esbuild` watch can produce `main.js`
from `main.jsx` while `nickel-plugin dev` watches the output:

```sh
esbuild main.jsx --target=es2022 --jsx-factory=h --outfile=main.js --watch
```

The runtime provides `h`, `Panel`, `Row`, `Column`, `Text`, `Button`, `Dialog`,
`Image`, `ImageButton`, `useState`, `useRef`, and other small native components.
Images use a host-provided asset key and explicit `width` and `height` (1 to
8192 logical pixels); `fit` is `contain`, `cover`, or `stretch`. `ImageButton`
also needs an ID, an accessibility label, and an `onClick` handler. A missing
host asset renders a placeholder without giving the plugin filesystem access.
Nickel does not embed
React, a browser DOM, or the TypeScript compiler. The host checks declared
capabilities and current shell state before executing a requested effect.
