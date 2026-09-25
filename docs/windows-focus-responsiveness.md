# Windows focus responsiveness

## Failure observed on 2026-09-24

Nickel's log recorded shortcuts being forwarded at 23:19:56 UTC while the UI
loop stopped handling them until 23:20:34. Fourteen queued switch actions then
ran in about half a second. The keyboard hook and its forwarding thread stayed
responsive during the pause. This establishes a UI-thread stall and backlog;
no stack was captured to identify the exact blocking call in that occurrence.

Both launcher focus and application activation attached Nickel's input queue
to foreign threads using `AttachThreadInput`. Activation could consequently
wait for another application to process its messages. The launcher's 75 ms
polling deadline did not bound the preceding native calls. Restoring,
minimizing, and maximizing foreign windows also used synchronous `ShowWindow`.

Microsoft describes why [attached input queues make focus changes synchronous](https://devblogs.microsoft.com/oldnewthing/20130607-00/?p=4143)
and documents [ShowWindowAsync](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync)
as avoiding waits on an unresponsive window owner.

## Current behavior

- Input queues remain separate during foreground requests.
- Launcher show/focus runs only on the thread owning its HWND. The launcher is
  shown without implicit activation, foreground is requested once, and native
  foreground ownership is checked directly. A rejected request hides the
  launcher and does not project internal typing focus. There is no sleep loop.
- Application restoration, minimization, and maximization use asynchronous
  show-state requests. The normal window feed observes actual state changes;
  request admission is not treated as proof of completed activation.
- Minimized titlebar parking happens when the feed observes `IsIconic`, after
  minimization, so a queued request cannot move a normal window off screen.
- Foreground requests log their start and completion, with elapsed time, to
  help identify any remaining native stall during live testing.

The native adapter regression creates a hidden minimized window on another
thread and deliberately stops pumping its messages. Activation and show-state
requests must return within 500 ms while its owner remains paused. A launcher
request against that foreign window must be rejected immediately. The fixture
does not take foreground focus and releases its window even on failure.

Existing launcher rejection, focus-loss dismissal, and retry tests cover the
shell-state contract. This native wait regression uses real Win32 windows
rather than the platform-neutral scenario harness because it tests Windows
message-queue blocking itself.

Live acceptance: alternate focus among Calculator, Armoury Crate, and desktop
apps; open and dismiss the launcher; restore minimized apps; and verify that
hotkeys remain responsive when a target app is busy. The historical freeze's
exact native stack remains unverified.


Validation on Windows: all 87 tests selected by `cargo test -p nickel --lib
launcher` passed, including the native unresponsive-window regression. Formatting
and diff whitespace checks passed. Strict package Clippy was blocked in the
unchanged dependency `nickel-codex/src/process.rs:56` by
`clippy::obfuscated_if_else`, before checking the Nickel library.

## Desktop context-menu activation

Right-clicking the desktop and dismissing its menu could leave the wallpaper
above applications. The detached topmost popup used the passive desktop as
its native owner, allowing popup activation and dismissal to involve the
wallpaper. Desktop configuration previously placed the wallpaper at the bottom
only when configuring the window.

Passive windows now have independently owned popups. A desktop window subclass
rejects mouse activation and forces window-position changes to the bottom with
no activation. Dropping a popup session requests cancellation; desktop pointer
presses explicitly drop the session because a passive desktop click need not
produce a native focus-loss event.

Windows validation passed: one native desktop regression, three popup tests,
and four existing `desktop_menu` interaction scenarios.

A native regression verifies mouse activation rejection and a real attempted
topmost promotion. Popup tests cover passive-owner selection and cancellation
on session drop. Manual acceptance: right-click the desktop, left-click outside
the menu, and Alt+Tab between applications; repeat on each monitor.

Live validation: the user confirmed that desktop menu dismissal and switching
away from the wallpaper work after rebuilding and restarting Nickel.

## Background task-switch activation

Subsequent live Alt+Tab testing showed immediate `SetForegroundWindow`
rejections (`requested=false`, `elapsed_ms=0`) for Chrome and Terminal.
Receiving a shortcut through a low-level hook does not guarantee foreground
permission for Nickel's UI thread. Removing input-queue attachment exposed
this missing activation path.

An attempted `SwitchToThisWindow` fallback proved insufficient in user testing
and was removed. Focus-only live checks also missed the user's more precise
report: an app could receive focus without being raised above other windows.

Application activation now explicitly raises the selected window with
`SetWindowPos(HWND_TOP, SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOMOVE |
SWP_NOSIZE)`. This preserves the ordinary window band rather than making apps
topmost, and posts cross-thread positioning instead of waiting for the owner.

Launcher and application focus share one recovery path. When an ordinary
request fails, the target is valid and activatable, and no Shift, Ctrl, Alt,
or Super key is held, Nickel sends a paired Alt press/release and retries
foreground admission once. Windows documents Alt as releasing the foreground
lock in [LockSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-locksetforegroundwindow).
Both edges are submitted together; partial submission attempts a release.
Injected input is ignored by shortcut recognition. Rejected input or focus
requests remain failures. Passive desktop surfaces are never activated.
No explicit input-queue attachment or focus polling is used in production.

Validation: the unresponsive-window regression, now including asynchronous
raising, passed in 0.01 seconds. The native desktop topmost-rejection test also
passed. The opt-in live test verifies foreground ownership and, when given a
comparison window, actual Z-order within two seconds. Chrome-to-Terminal and
Terminal-to-Chrome checks both passed. These are asynchronous effects, so the
test waits for both instead of treating request dispatch as completion.

Set decimal `NICKEL_FOCUS_TEST_HWND` and `NICKEL_FOCUS_TEST_OTHER_HWND`, then run
`cargo test -p nickel --lib explicitly_selected_live_window_receives_foreground
-- --ignored --nocapture`. This changes focus and is excluded from normal tests.
The user confirmed that the rebuilt shell works after testing the changes for
Alt+Tab, panel clicks, and the Windows key.
