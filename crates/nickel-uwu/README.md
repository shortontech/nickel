# Nickel UWU: Universal Windows Usher

`nickel-uwu` is the Windows-only UWP presentation component. Its library owns
the immersive shell controller, shell-window registration, and automatic
presentation loop. Nickel starts that loop in a managed child of `nickel.exe`
when the session has no registered shell window. The child uses the same
executable, exits when Nickel exits, and is stopped during normal Nickel
shutdown. With Explorer or another shell registered, Nickel waits and checks
again so it can take over if that shell later exits.

The `nickel-windows-frame-probe` binary and the other binaries in `src/bin/`
remain manual diagnostics. Nickel depends on the library with its `diagnostics`
feature disabled. The rebuilt Nickel helper was also run while the old fixture
owned the shell window; it refused registration as expected. The existing live
fixture test below applies to the diagnostic host, not to a Nickel-managed
child. Private offsets and callbacks were derived from Windows build 26200.
The host validates the relevant live import targets and COM vtable entries at
runtime instead of rejecting other Windows build numbers before startup. The
managed child still needs live runtime validation before broad deployment.

## Diagnostic history

This isolated diagnostic fixture tries to create Windows' registered **Immersive
Application Manager** COM class (`{50FDBB99-5C92-495E-9E81-E2C2F48CDDAE}`),
connects to the **ImmersiveShell** COM server (`{C2F03A33-21F5-47FA-B4BB-156362A2F239}`),
and queries two candidate service IDs. It then runs `nickel-windows-app-probe`
with any acquired objects kept alive. Each phase reports an HRESULT. A successful
app launch after a failed service query is only a baseline; it does not show
that the fixture supplied the missing frame service. This remains separate
from the automatic presentation path.

Build both small probes in release mode with one compiler job:

```powershell
cargo build --release -j 1 -p nickel-uwu -p nickel-windows-app-probe
.\target\release\nickel-windows-frame-probe.exe 'Shorton.NickelUwpTarget_hmprcxg96edac!App'
.\target\release\nickel-windows-frame-probe.exe 'Shorton.NickelUwpTarget_hmprcxg96edac!App' --private-service
cargo build --release -j 1 -p nickel-uwu --bin band-probe
.\target\release\band-probe.exe
```

The launcher probe has its own 20-second timeout; the outer worker has a
30-second timeout. Read the target's package-local `activation.log` to see how
far startup progressed. This probe does not start or stop Explorer.
The opt-in `--private-service` mode loads Windows' own `twinui.pcshell.dll`
from System32, requests its internal `PrivilegedOperationsService` class
factory, and tries to construct that service before the manager. If private
construction fails, the worker exits immediately because the DLL may have
partially initialized process-local state.

## Result on Windows build 26200 (2026-09-23)

With Explorer running, direct manager creation returned `0x8027FFFF` in both
STA and MTA. `ImmersiveShell` creation succeeded, but querying the manager
CLSID and observed frame interface IID as service IDs both returned
`E_NOTIMPL` (`0x80004001`). Packaged-app activation still succeeded through
the regular Explorer session. With Explorer stopped, direct manager creation
again returned `0x8027FFFF`, and `ImmersiveShell` creation returned
`REGDB_E_CLASSNOTREG` (`0x80040154`). A separate cold-host control using only
`nickel-windows-app-probe` timed out after 20 seconds; the target reached
`initialize`, then reached `set-window` only after Explorer was restored.

Disassembly of Microsoft's `twinui.pcshell.dll` with public symbols shows
`CApplicationManager::RuntimeClassInitialize` calls
`_InitializeIAMSubcomponents`, which calls the non-exported
`CApplicationFrameService_CreateInstance` and sites its returned object. The
frame service then creates the Application Frame Manager COM class
(`{B9B05098-3E30-483F-87F7-027CA78DA287}`) and initializes it. Merely
starting `ApplicationFrameHost.exe -Embedding` did not make activation work.
One warm-host run succeeded after Explorer stopped, but a controlled repeat
timed out, so a live host alone is not a reliable substitute for the service.

The manager's startup contract has further private shell prerequisites,
described below. Calling the private frame-service factory directly would
skip required manager state and site setup shown by the disassembly.

## Private service result

Debugging the direct manager construction found the first failure in
`InitializePrivilegedOperationsForIam`: its process-local
`PrivilegedOperationsService::s_this` is null, so it returns `E_FAIL`.
`CApplicationManager::RuntimeClassInitialize` then raises shell-component
startup failure `0x80270233`; the outer `CoCreateInstance` reports
`0x8027FFFF`. This occurs before frame-service creation.

The opt-in probe identified the internal service CLSID
`{E7A8F97B-CA27-413B-851B-AF50345ECC63}` from the DLL's class-factory
table. `DllGetClassObject` succeeds, but `IClassFactory::CreateInstance`
returns `E_ACCESSDENIED` (`0x80070005`) with Explorer running and stopped.
The first failure comes from `NtUserAcquireIAMKey` in `win32u.dll`, called while
constructing `IamAccess` for the service's privileged window operations.
The later access-denied messages are propagation of that first failure.
The private class is not registered under `HKCR\CLSID` on this machine.

The managed host registers its hidden window through `SetShellWindowEx` to
acquire the shell privileges needed by the immersive manager. It does not
register for shell-hook messages.

## Shell window and next startup gate

An isolated `shell-key-probe` refuses to run if `GetShellWindow` is non-null.
With Explorer stopped, it registered a hidden top-level window through
`SetShellWindowEx`; `GetShellWindow` then returned that window and
`NtUserAcquireIAMKey` succeeded. This confirms that shell-window registration
is a prerequisite for the key on this build.

The opt-in `--shell-window` mode of this fixture then created the private
`PrivilegedOperationsService` successfully. Manager initialization advanced
past that service, but failed inside `CFallbackWindow::s_CreateInstance`.
The observed `CreateWindowInBand` call used band `12` and returned null with
Win32 error `5` (`ACCESS_DENIED`). The earlier band `1` creation succeeded.
The manager raised shell-component startup failure `0x80270233` before it
could create the frame service.

A separate `band-probe` calls `CreateWindowInBand` directly in the unsigned
fixture process, with Explorer left running and no manager or private service.
It creates a hidden 1-by-1 `STATIC` window in band `1` successfully, then
attempts the same window in band `12`; that call returns null with Win32 error
`5`. This isolates the denial to the band creation call, independently of
manager initialization. It does not identify which Windows eligibility check
rejected the process.

Band `12` is `ZBID_IMMERSIVE_BACKGROUND` in the published reverse-engineered
[window-band enum](https://blog.adeltax.com/window-z-order-in-windows-10/).
That research reports that private immersive bands require a `.imrsiv` PE
section, `IMAGE_DLLCHARACTERISTICS_FORCE_INTEGRITY`, and a Microsoft Windows
signature. Local PE inspection found the section and flag in `explorer.exe`
and `ApplicationFrameHost.exe`, but neither in this unsigned Rust probe.
This is consistent with the error; the exact access check on Windows build
26200 is detailed below. Adding a section or flag alone would not provide a
Microsoft signature.

## Exact band 12 denial on build 26200

Headless Ghidra analysis of this machine's `win32kfull.sys` and
`win32kbase.sys`, with their matching Microsoft public PDBs, resolves the
relevant path:

1. `win32kfull!xxxCreateWindowEx` calls `IsValidBandForProcess` with band
   `12` and the caller's `tagPROCESSINFO`.
2. For band `12`, `IsValidBandForProcess` returns true only when
   `win32kbase!IsImmersiveBroker` returns true. Its other allowances cover
   different bands, including ordinary band `1`.
3. `IsImmersiveBroker` accepts a process whose immersive type bits in
   `tagPROCESSINFO` at offset `0x330` equal `0x20`, or one of two process
   identities registered in session state. The unsigned probe is neither.
4. When validation returns false, `xxxCreateWindowEx` calls
   `UserSetLastError(5)` and returns no window. This matches the isolated
   probe's Win32 error exactly.

The immersive type bits are set by `win32kbase!SetProcessType` during process
initialization. Its input comes from `UserProcessImmersiveType`, which checks
the executable's `.imrsiv` section, process-creation callout flags, token
properties, and integrity level before assigning immersive broker type `2`.
The public symbols do not name the meaning of every callout flag; this
analysis identifies the immediate rejecting check, not every upstream
condition needed to qualify a new process. Explorer and RuntimeBroker both
have `.imrsiv`, force integrity, and valid Microsoft Windows signatures on
this installation; `band-probe.exe` has none of those properties.

Executing the call from code injected into RuntimeBroker would use
RuntimeBroker's existing process identity. It could confirm the process gate
experimentally, but it would not make an independent Nickel process pass the
gate or distinguish the upstream eligibility conditions.

## IAM access and changing a regular-band window

With Explorer stopped and the restore guard active, an opt-in
`shell-key-probe --set-band` registered its own hidden shell window, acquired
the IAM key, and successfully called `NtUserEnableIAMAccess(key, 1)` on the
same thread. A separate hidden window began in band `1`. Calling
`SetWindowBand` to move that window into band `12` failed with Win32 error
`5`; `GetWindowBand` still returned `1`. Disabling IAM thread access
succeeded, and Explorer was restored.

The matching `win32kfull.sys` code explains this result. In
`_DeferWindowPosAndBand`, IAM thread access is one prerequisite, followed by
an independent `IsValidBandForProcess` check. That process check rejects band
`12` for an ordinary process even when IAM thread access is enabled. The
published `NtUserEnableIAMAccess` workaround for `SetWindowBand` therefore
does not, by itself, move an unsigned Nickel-owned window to band `12` on
this Windows build.

## Band of a working app frame

With Explorer running, `nickel-windows-app-probe` activated the packaged
Nickel UWP target, which logged `initialize`, `set-window`, `load`, and `run`.
The read-only `window-band-inspect` probe enumerated top-level windows using
`EnumWindows` and called `GetWindowBand` on each window owned by the app or
`ApplicationFrameHost.exe`. All five frame-host windows and both target
windows reported band `1`, including the frame-host process whose main window
title was `Nickel UWP Target`. No app or frame-host top-level window required
band `12` in this successful launch. The target was stopped after inspection.
The probe does not enumerate child or message-only windows, so this does not
prove that no Windows component uses band `12` elsewhere.

## Substituting a band-1 fallback in an isolated fixture

The opt-in `--shell-window-fallback-band1` mode redirects only this probe
process's `twinui.pcshell.dll` import of `CreateWindowInBand` when the requested
band is `12`. It asks Windows to create that window in band `1` instead. This
is a diagnostic import-table hook for the installed Windows build, not a
Nickel implementation or a supported Windows contract.

With Explorer stopped under a timed restore guard, the probe registered its
shell window, created `PrivilegedOperationsService`, and then created
`CApplicationManager` successfully. It recorded exactly one redirected band-12
request. This demonstrates that the band-12 fallback is not required just to
construct the manager. Packaged target activation still timed out: its log
reached `initialize`, then reached `set-window` only after Explorer returned.
The `ImmersiveShell` local COM class also returned `REGDB_E_CLASSNOTREG` while
Explorer was absent. Manager construction alone therefore does not provide
the application frame integration required by activation.

An independent `frame-manager-direct` fixture requested the registered
Application Frame Host COM server and `IApplicationFrameManager` directly.
It waited in COM activation for 30 seconds even with Explorer running. Using
the exact class context `0x404` seen in
`CApplicationFrameService::_CreateFrameManager` did not change that result.
No frame-event callback was registered in this experiment, so it says nothing
about callback sufficiency.

## Role of the band-12 fallback window

The failed window is `twinui.pcshell!CFallbackWindow`, not an
`ApplicationFrameHost.exe` frame. Its symbolized methods include
`SetFallbackForeground`, `MoveToMonitor`, `_SizeWindow`, and `_GetVisibleApp`,
which suggest a shell background/foreground fallback role. The manager stores
its `IFallbackWindow` object separately from `IApplicationFrameService`.
`CApplicationFrameService_CreateInstance` receives an
`IImmersiveApplicationManagerInternal` interface, not a fallback HWND, and its
frame-manager creation calls the registered Application Frame Host COM server.
These observations do not rule out a later indirect request for the fallback
through the manager; that dependency has not yet been traced.

The historical [Back2TheFuture endpoint map](https://github.com/SafeBreach-Labs/Back2TheFuture/blob/main/rpc/endpoint_mapping.txt)
corroborates process/interface ownership: `sihost.exe` exposes
`IApplicationActivationBroker` IID
`{D98FD14A-522A-4D59-B875-811E83919A9E}`, Explorer exposes
`IImmersiveApplicationManager` IID
`{BF63999F-7411-40DA-861C-DF72C0FFEE84}`, and
`ApplicationFrameHost.exe` exposes `IApplicationFrameManager` IID
`{D6DEFAB3-DBB9-4413-8AF9-554586FDFF94}` and `IApplicationFrame` IID
`{143715D9-A015-40EA-B695-D5CC267E36EE}`. These are OLE/COM interface
entries, not method signatures or a current Windows 11 endpoint trace.

## Direct frame-service fixture (2026-09-23)

The `--frame-service-direct` mode runs in an isolated worker. It registers a
diagnostic shell window, redirects this Windows build's band-12 fallback-window
request to band 1, creates `CApplicationManager`, then reads the frame service
the manager itself stored at offset `0x260`. The offset and method addresses
were checked against this build's `twinui.pcshell.dll` PDB and vtables. The
manager had already created and sited its frame service. The fixture supplies
an `IUnknown` placeholder for the shell-chrome-controls service requested by
`CompleteInitialization` (`{D6F29401-6EA3-4757-A73C-B30ABB699DC3}`), then
calls `EnsureFramePool`. The placeholder is only suitable for this diagnostic;
it does not implement shell chrome controls methods.

With an old suspended `ApplicationFrameHost.exe` process, `EnsureFramePool`
blocked in `CoCreateInstance` for the frame manager. That host's threads all
had suspend count 1. After clearing the stale host, `EnsureFramePool` returned
`S_OK`. An old suspended `sihost.exe` likewise caused the app probe to block
before activation-manager creation; resuming it restored the original
behavior: `CoCreateInstance` succeeds, then `ActivateApplication` waits.

Holding the manager-owned frame service and its frame pool alive while
launching the packaged target still timed out after 20 seconds. The target
reached `initialize` but not `set-window`. The fixture also pumped its shell
thread's message queue during activation and dispatched zero messages. A
previously visible `Nickel UWP Target` window belonged to
`ApplicationFrameHost.exe` after the target process exited. Clearing that stale
frame before a fresh run did not change the timeout. A full `explorer.exe`
shell launched the same target successfully in this session.

Windows' `Windows.ImmersiveShell.ServiceProvider.dll` has a separate
`ImmersiveShellBrokerHost::PublishServices` method that registers the
`ImmersiveShellBroker` class `{228826AF-02E1-4226-A9E0-99A855E455A6}` through
`CoRegisterClassObject`. This is a concrete difference between the full
Explorer shell and the isolated fixture; whether that broker is necessary for
this particular view activation remains to be established.

The same DLL registers two in-process classes on this build:
`{23650F94-13B8-4F39-B2C3-817E6564A756}` is
`CImmersiveShellController`, and `{4075B76F-FCDC-43B6-B0B9-5A005B38B335}`
is `ImmersiveShellBrokerHost`, as confirmed by their live vtables and matching
PDB. The controller's `Start()` first checks that `GetShellWindow` belongs to
its process and obtains the separate `ImmersiveShell` class, so it is not by
itself a replacement for publishing that class. `DllGetClassObject` for
`ImmersiveShell` returned `CLASS_E_CLASSNOTAVAILABLE` from the checked shell
DLLs; that class is available with the full Explorer shell but absent without
it.

## Controller startup and application visibility (2026-09-23)

The later `--host-pump` fixture sets its own shell window, creates the
`CImmersiveShellController` through the installed Windows DLL, calls `Start`,
and pumps the shell thread. On this Windows build, startup requires two
diagnostic substitutions: the band-12 fallback window is created in band 1,
and component 94 (`TouchKeyboardExperienceManager`) is skipped because it
otherwise failfasts. `Start` returns `S_OK` and publishes `ImmersiveShell` and
the application frame service. With Explorer stopped, `ActivateApplication`
then starts the packaged Nickel UWP target, Windows Settings, and Calculator.
`ApplicationFrameHost.exe` creates their frames. This does not yet produce
working app content: Settings and Calculator remain on their splash images.

The target's instrumented `CoreApplicationView.Activated` handler runs under
the fixture. A fresh activation generally leaves `CoreWindow.Visible` false,
and the target is then suspended. One run did receive `VisibilityChanged(true)`
and continuously presented alternating D3D11 frames, but its frame still
showed the splash image. The Explorer baseline presents the target's blue
content. TWinUI event 171 reports `Application Shown` in the Explorer run but
not in the fixture run. The shell view transition is therefore a separate
step after process and frame creation.

The diagnostic controller's UWP wrapper subscribes to
`ClientWindowReadyForPresentationChanged`, but Explorer's second
`CApplicationFrame::SetPresentedWindow` call, with the target CoreWindow HWND,
does not occur in the fixture. Explorer also calls
`UwpWindowWrapperBase::WindowDiscoveredFromShellHook` for the target; the
fixture does not receive that callback. Registering the fixture's shell HWND
with `RegisterShellHookWindow` succeeded but delivered no UWP creation
messages. A diagnostic call to `WindowDiscoveredFromShellHook` with the live
wrapper and target HWND associated the window and updated the frame title.

On this Windows build, `UwpWindowReadyState::IsReadyForPresentation` checks
that its client HWND is nonzero and its wait flags are zero. The fixture's
wrapper retained visibility wait flag `1`. Calling the wrapper's
`VisibilityChanged` method with `EventPhase=0, Visibility=1` did not clear it;
the debugger showed that `EventPhase=1, Visibility=1` does. The latter caused
ApplicationFrameHost to remove the splash, leaving a black content region.
Both the frame and CoreWindow initially reported `DWM_CLOAKED_SHELL=2`, and
the target remained suspended with `CoreWindow.Visible=false`. Calling
`IApplicationView::SwitchTo` directly or through the view manager returned
`E_ACCESSDENIED`. The `switch-view AUMID --uncloak` fixture instead called
the view's `SetCloak(AVCT_DEFAULT, 0)` method. It returned `S_OK`; both HWNDs
then reported cloak state `0`, the target received `VisibilityChanged(true)`,
resumed its event loop, and continuously presented alternating D3D11 frames.
ApplicationFrameHost showed those colors with Explorer absent. This is the
first complete Explorer-free rendering path for the packaged target in this
fixture. It required manual wrapper discovery and readiness callbacks before
the uncloak call, so the fixture does not yet automate UWP launch or prove
that Settings renders. The private calls and offsets are guarded build-specific
diagnostics, not production Nickel behavior.

The `set-view-state` diagnostic obtains `IApplicationViewStateControl`
`{DE6E8A03-3811-4239-9DE9-96D0DFC301E6}` from the published
`ImmersiveShell` service provider and calls its build-specific vtable slot 3,
`SetViewStateForDesiredAppState`. This is a private interface, identified
from this build's public symbols and proxy vtable. State 0 minimizes an
existing Settings frame; state 1 restores it. Both calls return `S_OK`, so
the HRESULT alone does not establish that the app is rendered. Restoring
Settings resumes its `ApplicationView ASTA` thread from
`PsmWaitForAppResume`, but its frame still shows only the splash. The
remaining issue is the app view/content activation path, not frame creation.

## Explorer-free Settings and Calculator UI (2026-09-23, later run)

The remaining readiness flag was `2` for Calculator and Settings. This build's
`UwpWindowReadyState::HandleShellHook(0x26)` clears that layout flag. The
`post-window-layout WRAPPER HWND` diagnostic posts the call to the controller's
shell thread and logs wait flags before and immediately after it. For each
fresh app, the working order was:

1. Activate its AUMID with `nickel-windows-app-probe` while `--host-pump` runs.
2. Identify the frame and CoreWindow HWNDs. Run
   `find-window-wrapper FRAME_HWND_HEX` to get this build's wrapper interface,
   then call `post-window-discovery WRAPPER HWND`.
3. Call `post-window-visible WRAPPER HWND` and `post-window-layout WRAPPER HWND`.
4. Call `switch-view AUMID --uncloak` to remove shell cloaking from the view.

The wrapper must be matched to the app's live CoreWindow. The finder reads only
the diagnostic host's memory and validates this build's dispatcher and wrapper
vtables; it is not a production discovery path. The readiness event can briefly
clear flag `2` and then have it set again; a second layout event
cleared it in one Calculator run. The host logged `before=0x2 after=0x0` for
both apps in the later run. Both CoreWindows and frames became uncloaked.

With Explorer absent, Windows Settings rendered its populated Home screen and
accepted input. The user confirmed it resizes and is usable. Calculator also
rendered its controls and accepted number-button input. This establishes a
working Explorer-free UI path for two installed packaged apps in addition to
the Rust target. The host still requires manual frame/CoreWindow identification
and readiness callbacks; Nickel does not yet perform this sequence itself.

Calculator's `ApplicationFrameWindow` initially measured about `336x509`,
while its CoreWindow stayed `320x320`, leaving a colored strip beneath the
content. Resizing the frame with `SetWindowPos` moved its title bar and input
sink, but did not resize its CoreWindow. Resizing that CoreWindow separately
filled the strip with the app's dark background; Calculator's controls remained
at their original width. The user reported that Calculator's frame could not
be dragged or resized by mouse, while Settings could be resized. Both frames
have matching Win32 styles. Calculator's title area is partly covered by its
CoreWindow, which reports `HTCLIENT`; the exposed title-bar child reports
`HTCAPTION`, and the frame edges report resize hit zones. This leaves the
Calculator-specific move/resize handoff unresolved.

A later 90-second live poll sampled the cursor, frame rectangle, and CoreWindow
rectangle every 5 ms during mouse resizing. The frame changed 169 times,
including continuous changes during a drag, while the CoreWindow changed zero
times, including after the drag ended. The frame ended at `545x756`; its
CoreWindow remained `441x637`. Thus the visible size mismatch is a missing
frame-to-CoreWindow layout update, independent of the initial pause before the
frame starts moving. Clearing the wrapper's layout wait flag from `0x2` to
`0x0` did not resize the CoreWindow; a subsequent one-pixel `SetWindowPos` on
the frame also left the flag at `0x0` and the CoreWindow unchanged. The poll
did not record mouse-button transitions, so it does not measure the reported
delay between the first click and the first frame change.

Maximizing and immediately restoring the same frame does update the child:
before maximize the frame/CoreWindow were `546x757`/`441x637`; maximized they
were `1936x1040`/`1920x1024`; restored they were `546x757`/`530x748`. The
restored CoreWindow matches the frame's client area. The wrapper's layout wait
flag was `0x2` afterward, so clearing that flag is not a prerequisite for this
state-change layout path. A normal mouse resize and a one-pixel `SetWindowPos`
do not exercise the same child-layout behavior as maximize/restore. Sending a
bounded `WM_SIZE` with the frame's current client dimensions also left the
CoreWindow unchanged, so replaying that message alone is insufficient.

The reported first-drag pause may be an event handoff with a fallback rather
than a literal sleep in Calculator. `ApplicationFrame.dll` queues position-change
work, and `Windows.UI.dll` has a separate `WindowServer::OnWindowSizeEvent`
path. The earlier trace proves the position-change task was queued and run,
but does not yet prove whether the frame service delivered a matching size
notification or whether a fallback completed the drag. Trace those transitions
before attributing the pause to application code or a timeout.

A targeted live breakpoint in Calculator's `Windows.UI.dll` confirmed the
`WindowServer::OnWindowSizeEvent` distinction: a direct one-pixel frame resize
did not enter the handler and left the CoreWindow unchanged; maximizing and
restoring entered it twice and changed the CoreWindow to the matching bounds.
`WindowServer::TriggerWindowSizeEvent` did not run in either case, so the
working show-state transition uses another route into `OnWindowSizeEvent`.
No ordinary mouse drag occurred during this breakpoint capture. A separate
breakpoint on the shell's `CApplicationFrameService::OnPositionChanged` did not
fire for the direct resize or maximize/restore, although it was armed at the
symbolized address. These observations narrow the missing normal-resize
notification, but do not identify the reported initial-drag fallback.

The working maximize path was captured at `WindowServer::OnWindowSizeEvent`.
Its stack starts with a `CoreMessaging` ALPC property-change delivery, then
`CoreUIComponents!NavigationClient::PropertyChanged` calls
`NavigationClient::UpdateBoundsOnCoreWindow` and
`NavigationClientWindowClientAdapter::LegacyTransforms_UpdatePhysicalBounds`.
`Windows.UI!WindowServer::OnPhysicalBoundsChanged` calls `NtUserSetWindowPos`
on Calculator's CoreWindow. The resulting `WM_WINDOWPOSCHANGED` reaches
`DefWindowProcW`, which sends `WM_SIZE` and enters
`WindowServer::OnWindowSizeEvent`; XAML receives the new layout afterward.
This is a concrete event path into the working resize. The direct frame resize
does not produce that CoreWindow size event. The sender of the CoreMessaging
property change and the precise normal-drag fallback are not yet identified.

ApplicationFrameHost's `CApplicationFrameManager::EnableLayoutFrames` flag was
observed off. Enabling it temporarily in the live diagnostic process did not
make Calculator's CoreWindow follow a frame resize, so the flag was restored.

## Shell hook forwarding experiment (2026-09-23)

[GyroShell PR 31](https://github.com/Pdawg-bytes/GyroShell/pull/31) pointed to
`SetTaskmanWindow` and the private `IImmersiveShellHookService`. The fixture's
`RegisterShellHookWindow` had succeeded, but its stock `STATIC` window
procedure discarded hook messages before the `GetMessageW` diagnostic could
see them. An earlier `--host-pump` experiment subclassed that window,
registered it as the task manager window, and forwarded window-created and
window-destroyed events (codes `1` and `2`) through
`PostShellHookMessage`. The separate `shell-hook-service-probe` confirms
the service can be queried without
Explorer and reports the current task manager HWND with `--get-taskman`.

With no task manager HWND, launching Calculator produced no observed hook
events. With one registered, the host received creation events for both its
`ApplicationFrameWindow` and its `CoreWindow`; forwarding returned `S_OK`.
Calculator's CoreWindow still stayed separate from its frame until the manual
discovery, visibility, layout, and uncloak sequence above. Forwarding code `6`
(`HSHELL_REDRAW`) caused a redraw feedback loop. The managed host no longer
registers a task manager window or queries and forwards through
`IImmersiveShellHookService`; its automatic presentation loop instead
reconciles live windows every 250 ms. The probe and these observations remain
as research history. A cold-login UWP run is still needed to validate the
removal end to end.

This build's `UwpWindowEventDispatcher::OnShellHookMessage` handles several
private event codes from `17` through `26`, but does not directly handle `1`
or `2`. Its `GetViewFromHwnd` lookup compares against each wrapper's window
ID, which is zero for a new Calculator wrapper before discovery. The active
dispatcher does hold a collection of those wrappers, and the wrapper for
Calculator already records its frame HWND. `find-window-wrapper` uses that
identity without attaching a debugger or restarting the host. In the live
Calculator session it returned the same wrapper pointer found by CDB and
reported the layout wait flag returning to `2` after the first layout call; a
second layout call cleared it. The production lifecycle and resize path still
need investigation.

## Automatic presentation in the diagnostic host (2026-09-23)

`--host-pump` now performs the previously manual discovery, visibility, layout,
and uncloak sequence automatically. Its shell-thread timer checks every 250 ms
for a frame and a new CoreWindow with matching application user model IDs. It
uses the frame's property store and the CoreWindow process's package identity;
it does not match by window title. The existing wrapper finder is shared with
the host and retains its build-specific vtable checks. The discovery/readiness
calls retain their function-prologue checks.

Windows that existed before controller startup are excluded from discovery so
orphan CoreWindows from earlier experiments cannot be attached to new frames.
Ambiguous matches, including multiple frames or CoreWindows for the same app,
are left untouched. This fixture therefore automates distinct apps, but does
not yet resolve multiple simultaneous views of one app. Each matched frame has
an independent 15-second retry budget. Success requires the CoreWindow to be
parented to the selected frame after uncloaking. Completed pairs are not acted
on again, and records are removed when their windows disappear. The manual
diagnostic commands remain available.

Validation on Windows build 26200: stopped the previous fixture and AFH, started
the rebuilt host with Explorer absent, and activated fresh Calculator and
Armoury Crate SE instances. Both reached `phase=auto-present result=ready` about
512 ms after matching began, with no manual presentation commands. The user
confirmed both appeared and worked. Calculator's CoreWindow also continued to
follow a normal frame resize. The user then closed Armoury Crate independently.
These timings measure presentation retries after matching, not total app launch
or first-paint latency. This remains an isolated, build-specific fixture, not
Nickel's production UWP integration.

Formatting checks passed. Package Clippy passed with `too_many_arguments` and
`collapsible_if` allowed for two existing warnings in the legacy manager path;
the strict invocation still reports those warnings.

## Fresh Calculator resize comparison (2026-09-23)

The `resize-inspect` diagnostic enumerates application frames and CoreWindows,
including children, and prints their process/thread IDs, rectangles, client
dimensions, and window data pointers. With an explicit frame HWND it can request
a normal resize or a maximize/restore transition:

```powershell
cargo build --release -j 1 -p nickel-uwu --bin resize-inspect
.\target\release\resize-inspect.exe
.\target\release\resize-inspect.exe FRAME_HWND_HEX 700 740
.\target\release\resize-inspect.exe FRAME_HWND_HEX maximize
.\target\release\resize-inspect.exe FRAME_HWND_HEX restore
```

A fresh Calculator under Explorer followed an ordinary frame resize correctly.
Explorer and that ApplicationFrameHost process were then stopped. A fresh
`--host-pump` fixture was started, the baseline Calculator process was stopped,
and Calculator was activated again. The existing manual discovery, visibility,
layout, and uncloak sequence was applied. No fixture behavior was changed.

With Explorer absent, three ordinary frame resizes produced these dimensions:

| Frame outer size | Frame client size | Calculator CoreWindow client size |
| --- | --- | --- |
| 700 x 740 | 684 x 732 | 684 x 731 |
| 550 x 640 | 534 x 632 | 534 x 631 |
| 800 x 760 | 784 x 752 | 784 x 751 |

The CoreWindow followed each resize, with its top one pixel below the frame's
client origin. The user also confirmed that both the initial mouse-resize pause
and the stale controls were gone in this fixture session. The remaining reported
rendering delay occurs when Calculator first opens. Startup still requires the
manual readiness/uncloak calls, so this run does not establish an application
timeout or measure an automatic startup delay.

The Calculator frame reported band 1 in both the Explorer baseline and the
working fixture. Always-on-top behavior under the fixture was not verified in
this capture. These results do not establish the cause of the earlier failure:
the successful fixture was tested after Explorer had run in the same login
session, and both AFH and the tested Calculator instance had been replaced.
Session initialization, app state, and stale process state remain possible
differences. This is a successful fresh-process reproduction attempt, not a
resize fix or proof that a cold login works.

Validation: the new diagnostic built in release mode, passed targeted Clippy
with warnings denied, and was exercised against live Windows build 26200 frames.


## Presentation recovery (2026-09-24)

The embedded host now reconciles surviving frames as well as fresh frames.
A wrapper's existing client identity, an already selected pending client, or
an attached CoreWindow takes precedence over matching by application identity.
Unassociated windows still require an unambiguous match, with fresh cores
preferred over old orphans. A child of another frame is never selected.

Attachment no longer removes a pair from recovery. The host observes wrapper
association, zero readiness wait flags, correct parenting, visible window styles,
and uncloaked frame and CoreWindow state before reporting readiness. Those
observations must remain ready over at least one second across timer ticks;
callbacks or a failed observation restart confirmation. This verifies shell
presentation state, not application first paint or the correctness of app content.
Completed pairs are left alone until a window disappears. Minimized frames are
not restored by pending recovery.

After fifteen seconds, incomplete initialization retries every five seconds
instead of abandoning the frame. Disappearing clients reset pending association
and confirmation when a replacement is discovered. The existing private uncloak
lookup is by AUMID, so uncloaking remains deferred when multiple frames share that
identity. Per-app errors, slow recovery, readiness, and observed presentation
state now use tracing and are retained in Nickel's normal shell log; standalone
fixture stdout/stderr messages remain available.

Automated coverage uses the production reconciliation loop with synthetic native
observations and effects. It covers attachment followed by uncloak failure,
asynchronous attachment, readiness reverting, retry backoff and later recovery,
surviving attached and unattached cores, replacement clients, and ownership
conflicts. This adapter lifecycle is outside nickel-core's shell scenario model.

Validation: Windows package tests pass (17 tests), including all diagnostic
binary test targets. Package Clippy passes for all targets and features with
warnings denied; package formatting checks pass. Live cold-login,
Explorer handoff, minimizing during initialization, and first-paint/resize
acceptance remain to be exercised with a rebuilt shell. The private Windows
interfaces and build-specific offsets remain subject to the existing constraints.


### Multiple live dispatchers (2026-09-24)

Live Armoury Crate SE launch exposed a separate discovery failure: the heap
finder selected dispatcher 0x193c7e067e0, which did not own frame 0x2d066e.
Read-only enumeration found its wrapper in dispatcher 0x193f295abb0 instead.
The running shell eventually recovered the app after 81,256 ms without manual
presentation commands; inspection then showed the expected client and zero
readiness flags.

The finder now continues past dispatchers that do not contain the requested
frame, including empty or invalid collections. It caches the successful
candidate per thread, revalidating process, module, dispatcher vtables, and
frame ownership on each lookup. The new read-only probe located the live
Armoury Crate wrapper directly. Package library tests pass (20 tests) and
Clippy passes for all targets and features with warnings denied.


Live acceptance after rebuilding and restarting Nickel: the user launched both
Calculator and Armoury Crate SE and confirmed that both opened very quickly,
where both had previously been slow. ApplicationFrameHost had been terminated
before this Nickel restart, so this run exercised newly created frames. This
was a same-login-session test; a cold-login result has not been recorded.
