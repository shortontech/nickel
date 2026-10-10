# Support

Nickel is under active development. The Windows shell, nested Linux session,
direct Linux session, plugins, and experimental shell packages have different
levels of platform and hardware coverage. Good reports identify the exact mode
being used.

## Questions and setup help

Read the main [README](README.md) and the relevant guides first:

- [Linux sessions](docs/linux-sessions.md)
- [Windows packaging and installation](packaging/windows/README.md)
- [Plugin and shell development](assets/plugins/README.md)
- [Cargo workspace guide](docs/cargo-workspace.md)

If the documentation does not resolve the problem, [open a GitHub
issue](https://github.com/shortontech/nickel/issues/new) and describe what you
are trying to do. Questions about unsupported configurations are welcome, but
support may be best-effort.

## Bug reports

Before filing a bug, search the [existing
issues](https://github.com/shortontech/nickel/issues). A useful report includes:

- the Nickel version or commit;
- operating system and version;
- Windows, nested Linux, or direct Linux session mode;
- GPU, display layout, and input hardware when relevant;
- exact reproduction steps and expected behavior;
- logs or backtraces with secrets and personal data removed; and
- whether the issue reproduces with the default shell package.

For freezes or crashes, include the command used to start Nickel and the last
relevant log output. Do not upload credentials, session tokens, private window
contents, or unreviewed diagnostic archives.

## Feature requests

Open a GitHub issue describing the user problem, the desired behavior, and the
platforms it should support. For large architectural changes, discuss the idea
before investing in a complete implementation.

## Security reports

Do not report suspected vulnerabilities in a public issue. Follow the private
reporting process in [SECURITY.md](SECURITY.md).

## Contributing fixes

If you want to fix an issue yourself, see [CONTRIBUTING.md](CONTRIBUTING.md).
