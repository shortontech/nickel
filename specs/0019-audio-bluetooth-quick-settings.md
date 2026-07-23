# Audio and Bluetooth Quick Settings

## Goal

Provide taskbar controls for sound and Bluetooth that remain functional when Nickel runs without Plasma or another desktop environment.

## Scope

- Add platform-neutral audio and Bluetooth models to `nickel-core`, including output devices, volume, mute state, adapter power, discovery state, paired devices, connection state, and pending authorization.
- Add narrow platform adapters. Linux uses PipeWire/WirePlumber for audio and BlueZ over D-Bus for Bluetooth; Windows and macOS implementations follow their native APIs.
- Add sound and Bluetooth status buttons beside the clock. Icons must communicate muted, unavailable, powered off, and connected states without opening the menu.
- Open a compact quick-settings surface from either button. It supports output selection, volume and mute, Bluetooth power, discovery, connect/disconnect, and navigation to the full settings application.
- Keep blocking enumeration and device operations off the UI/compositor thread. Adapters publish cached snapshots and accept bounded commands through channels.
- Preserve ordinary StatusNotifier tray items. Quick settings are shell-owned controls, not synthetic tray applications.
- When no backend is available, render a disabled state with a concise explanation rather than hiding controls or invoking another desktop’s utilities.

The first implementation targets Linux. Pairing prompts, microphone controls, per-application volume, codecs, advanced device profiles, and cross-platform adapters are follow-up slices.

## Security and Session Services

- Use the existing user D-Bus for BlueZ clients and provide a Nickel Bluetooth agent before supporting new-device pairing.
- Never persist pairing secrets or audio credentials in Nickel.
- Treat device names and D-Bus properties as untrusted display data.
- Do not depend on Plasma, KDED, portal implementations, shell commands, or external scripting languages.

## Verification

- Unit-test snapshot reduction, icon states, device ordering, and command routing with synthetic adapters.
- Verify the panel remains responsive while Linux services are absent or slow.
- In a Nickel-only login session, change volume, mute, select an output, reconnect an existing Bluetooth device, and confirm sound continues after Plasma exits.
- Run workspace tests and Clippy with warnings denied.

## Completion

Archive this specification when sound and previously paired Bluetooth devices can be inspected and controlled from Nickel without another desktop environment running.
