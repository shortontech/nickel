# On-screen keyboard development

The initial implementation runs inside the Linux Nickel session. It uses the shared Rust UiHost,
keeps native text focus on the recipient, and supports pointer, touch, and controller navigation.
The US key layout has bounded square letter keys, wider modifiers, a navigation/function panel,
one-shot or held modifiers, Hide, and Move up/Move down controls. A compact layout is available when
there is insufficient logical space for the full keyboard.

In **Settings > Optional Features > On-screen keyboard**, choose Automatic, On, or Off. Automatic
follows direct-touch devices reported by the session. A nested winit device is not a physical
touchscreen. The adjacent tray button opens the keyboard. **Try keyboard** focuses a disposable
Settings field; use Move up if the keyboard covers it.

`NICKEL_ON_SCREEN_KEYBOARD=1` forces enablement, `0` forces it off, and unset or `auto` uses saved
preferences and device detection. The shell reads this once at startup. Settings shows an override
without overwriting the saved preference and makes the preference rows read-only for that session.

For nested testing, build `nickel-session` with `--features backend-winit`. Set
`NICKEL_NESTED_SIZE=1280x720` and `NICKEL_ON_SCREEN_KEYBOARD=1` on the supervised session process.
Use an isolated X server, a short private runtime directory, and disposable XDG config/state
folders. Clear inherited Wayland and session-control credentials before launching the session.
Do not start a second desktop on the active display for unattended tests.

The Rust `nickel-test-input` tool accepts `keyboard-status`, `semantic keyboard-toggle`,
`semantic keyboard KEY_ID`, and `touch down|move|up|cancel|frame` with the session test capability.
Key IDs come from the shared keyboard definitions. Semantic actions resolve production hit targets.
Wait for mapped geometry after showing a surface before resolving its key targets. Controller
acceptance uses the existing virtual device path; retain a real press interval rather than sending
press/release in the same instant.

Current evidence includes actual 1280x720 Smithay screenshots, launcher and Settings typing,
modifiers/deletion, controller invocation/navigation/typing/dismissal, touch invocation/cancellation,
focus changes during a press, and fresh automatic/manual/forced preference states.

This is not yet the complete cross-platform accessibility feature. The Windows adapter, external
IME/non-US coordination, per-field leases inside one native window, held-key repeat, simultaneous
modifier touch chords, automatic caret scrolling, exclusive controller ownership across separate
applications, and secure-session support remain pending. Do not make this keyboard a prerequisite
for unlocking a session. Windows compilation alone does not establish Windows keyboard support.
