# Nickel Markdown platform coverage

Coverage recorded on 2026-08-25. A platform is claimed here only when Nickel supplies viewer
association metadata for it.

## Linux

- `desktop-file-validate packaging/nickel-markdown.desktop` accepts the desktop entry without an
  error or hint.
- An isolated temporary XDG application database accepts `nickel-markdown.desktop` as the default
  `text/markdown` handler. `gio open` then launches the built viewer with the exact Markdown path;
  the launched process was observed before being terminated. The real user default is not changed
  by this check.
- The packaged `Exec=nickel-markdown-ui %f` contract matches the verified one-path CLI. Both an
  existing Markdown path and a missing path keep an SDL application window alive under the dummy
  video driver, and neither creates application state beside the input.
- External `http` and `https` destinations are typed before reaching `nickel-platform`; the Linux
  platform test verifies that the exact URL is passed as one argument to `xdg-open`.
- Native compositor presentation could not be exercised from the automated workspace because SDL
  reported no displays for its Wayland connection. Light and dark application rasters were rendered
  and inspected separately at narrow, ordinary, and high-DPI sizes.

## Windows

No Windows file-association metadata is supplied or claimed in this version. The ShellExecuteW URL
adapter compiles behind its Windows target boundary, but association and native presentation remain
untested.

## macOS

No macOS file-association metadata is supplied or claimed in this version. The `open` URL adapter
compiles behind its macOS target boundary, but association and native presentation remain untested.
