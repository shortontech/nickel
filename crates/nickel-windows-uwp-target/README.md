# Nickel UWP activation target

This is the right side of the Windows packaged-app experiment. It is a minimal
Rust `CoreApplication` with a UWP manifest. Each startup stage appends a line to
its package-local `activation.log`. The launcher side is
`nickel-windows-app-probe`.

Build the target in release mode:

```powershell
cargo build --release -j 1 -p nickel-windows-uwp-target
```

Stage the loose package in a directory outside the repo, then register that
manifest with `Add-AppxPackage -Register` in Developer Mode:

```powershell
$stage = Join-Path $env:LOCALAPPDATA 'Nickel\fixtures\uwp-target'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item target\release\nickel-windows-uwp-target.exe $stage
Copy-Item crates\nickel-windows-uwp-target\AppxManifest.xml $stage
Copy-Item assets\branding\nickel-logo.png (Join-Path $stage 'Logo.png')
Add-AppxPackage -Register (Join-Path $stage 'AppxManifest.xml')
Get-AppxPackage -Name Shorton.NickelUwpTarget | Select-Object PackageFamilyName
```

The AUMID is the installed package family name followed by `!App`.

The app writes `activation.log` under
`$env:LOCALAPPDATA\Packages\<package-family-name>\LocalState`. A new `main` line
proves that Windows started the packaged process. Later stage lines show that
Windows created and ran its CoreWindow. Run the launcher probe with that AUMID
under the desired shell session, then read the log. Close the target before a
subsequent activation comparison.

This fixture is based on the public `windows-rs` CoreApplication sample. It
contains no Wine implementation code.

## Nickel shell result (2026-09-23)

With Nickel as the Winlogon shell, registration succeeded after enabling
Developer Mode. In both local-server and in-process COM modes, the launcher
probe completed `CoInitializeEx` and `CoCreateInstance`, then timed out after
20 seconds inside `ActivateApplication`. Each attempt launched a separate
target process. The target logged `main`, `create-view`, and `initialize`, but
never logged `set-window`, `load`, or `run`. The target was stopped after each
attempt. This shows that process creation and early UWP startup work in this
session; the remaining hang is later in the window activation lifecycle.

After `explorer.exe` was started as the desktop shell in the same signed-in
session, the same local-server probe returned successfully in under a second
with target PID 15076. The target logged `main`, `create-view`, `initialize`,
`set-window`, `load`, and `run`. No sign-out was needed for this comparison;
the Winlogon registry value had been changed to `explorer.exe`, but the current
session was not restarted. This shows that a new sign-in is unnecessary for
activation to work. Explorer's presence is the likely relevant change, though
this run alone does not separate it from a possible dynamic registry effect.

A full user-mode dump of `ApplicationFrameHost.exe` from the working session
showed a thread named `Shorton.NickelUwpTarget_hmprcxg96edac!App` waiting in
`user32!GetMessageW` through `ApplicationFrame.dll`. That is the normal idle
message loop after the frame exists. The dump is a working baseline.

The Explorer-free result was reproduced with Nickel stopped and Explorer
temporarily stopped. The probe again timed out after 20 seconds, and the target
again reached `initialize` but not `set-window`. No `ApplicationFrameHost.exe`
process appeared during this run. Explorer was restored afterward. A target
minidump and all-thread stack trace are in
`%LOCALAPPDATA%\Nickel\diagnostics\uwp-target-explorer-absent.dmp` and
`uwp-target-explorer-absent-stacks.txt`.

The stalled activation thread is in
`twinapi_appcore!CoreApplication::GetWindowFactory+0x14d`, waiting in
`combase!ObjectStubless` / `RPCRT4!LRPC_BASE_CCALL::DoSendReceive` /
`ntdll!NtAlpcSendWaitReceivePort`. The caller is
`CoreApplication::ActivateForeground`, reached through
`CoreApplication::ActivateApplication`. Disassembly of `GetWindowFactory`
shows that it first creates `ShellServiceHostBrokerProvider` (CLSID
`{3480A401-BDE9-4407-BC02-798A866AC051}`), asks
`IServiceHostBrokerProvider` (IID
`{0F4ACCB1-D8F9-4011-BA37-2557925A78CF}`) for
`IApplicationActivationBroker` (IID
`{D98FD14A-522A-4D59-B875-811E83919A9E}`), and then stalls in a method of
that broker. These names and IDs match this machine's COM registry. The dump
establishes the blocked client call, but does not identify the server process
or prove that `ApplicationFrameHost.exe` is the direct recipient. Its absence
may be downstream of the broker wait.

Headless Ghidra analysis of `Windows.Shell.ServiceHostBuilder.dll` found
`CoRegisterClassObject` calls in `FUN_180002000` and `FUN_1800026c0`.
`FUN_180002000` obtains class IDs from an object passed into the builder and
registers one or two class factories; its diagnostic source path is
`servicehostbrokercomponent.cpp`. This machine's running
`ShellExperienceHost.exe` has `Windows.Shell.ServiceHostBuilder.dll` and
`execmodelproxy.dll` loaded, whereas the running Explorer process does not
have either module loaded. The provider CLSID is supplied at runtime and was
not present as a static GUID in either the builder DLL or ShellExperienceHost
EXE, so static inspection did not identify the broker server.

## RPC comparison with and without Explorer

Elevated ETW traces in `%LOCALAPPDATA%\Nickel\diagnostics` are named
`uwp-com-working.etl` and `uwp-com-explorer-absent.etl`. Both use
`Microsoft-Windows-COMRuntime` and `Microsoft-Windows-RPC`. The successful
run's target PID was 11736 and the failed run's target PID was 3032; both
calls were handled by the same `sihost.exe` PID 10324. This disproves the
earlier ShellExperienceHost server candidate for these broker calls.

In the successful run, `sihost.exe` called interface
`{92696C00-7578-48E1-AC1A-2CA909E2C8CF}` method 6 in the target, which
returned in about 80 ms. During that callback, the target called
`IApplicationActivationBroker` method 5 in `sihost.exe`, which returned in
about 40 ms. In the Explorer-free run, both calls reached their servers but
neither returned before the probe's 20-second timeout. `sihost.exe` and
`ShellExperienceHost.exe` both remained alive, and no
`ApplicationFrameHost.exe` appeared. The nested calls explain why the launcher
probe waits on activation, but they do not yet establish why the broker cannot
produce the view result.

A live, noninvasive `sihost.exe` stack capture at
`%LOCALAPPDATA%\Nickel\diagnostics\sihost-explorer-absent-stacks.txt` shows
`activationmanager!CApplicationActivationBroker::GetViewActivationResult`
waiting in `PendingViewActivationRequest::WaitForViewResult`, while another
activation thread waits on an outgoing COM call. The probe for that particular
capture eventually succeeded, so the failed ETW run remains the decisive
evidence for the sustained wait.

In a fresh successful baseline, `ApplicationFrameHost.exe -Embedding` had
`svchost.exe -k DcomLaunch -p` as its parent process. Explorer did not directly
create that observed frame host process. Independent headless Ghidra analysis
of this machine's `explorer.exe` found a call to
`IApplicationActivationManager::ActivateApplication` at `0x1400E12CF`; the
manager is created by a helper at `0x1400BC814` via `CoCreateInstance` at
`0x1400BC849`. Explorer's inspected process creation sites did not directly
launch ApplicationFrameHost. The frame host's DCOM launch still depends on
earlier shell activation state; the exact dependency has not been identified.

Manually creating the registered Application Frame Host COM class while
Explorer was stopped successfully started `ApplicationFrameHost.exe -Embedding`
through DCOM (parent `svchost.exe -k DcomLaunch -p`). The target still stalled
at activation for 20 seconds. Thus the missing step is not merely starting the
frame host process.

A fresh successful launch with no preexisting frame host was traced using
`Microsoft-Windows-RPC` in `fresh-frame-rpc.etl`. Explorer PID 5948 sent the
first observed calls into the newly created frame host PID 11356: interface
`{A914F499-4633-4B26-A93E-707EB5BDC0B6}` methods 7 and 3, followed by
many calls on `{143715D9-A015-40EA-B695-D5CC267E36EE}` and
`{C8E34820-D46A-41BC-8C5C-5BC9FDEE243D}`. The frame host received those
calls. Those interface GUIDs appear in `ApplicationFrame.dll`, and two appear
in `twinui.pcshell.dll`. This is direct evidence that Explorer participates
in the frame host handshake after DCOM starts the process. The methods are
internal Windows interfaces; their semantics still need to be mapped before
Nickel can reproduce the sequence.

The ADK's `xperf` captured user stacks for the frame RPC calls in
`xperf-rpc-stack-working.etl`. Resolving the first Explorer caller with CDB
identified `twinui_pcshell!CApplicationFrameService::_CreateFrameManager`.
Its disassembly at `0x7ff9_7aaad331` calls `CoCreateInstance` for the frame
host CLSID `{B9B05098-3E30-483F-87F7-027CA78DA287}`, requesting IID
`{A914F499-4633-4B26-A93E-707EB5BDC0B6}`. Afterward it invokes method 7
on the returned interface, passing a pointer into its
`CApplicationFrameService` object; this matches the first observed Explorer
to frame host RPC method. `twinui.pcshell.dll` also contains
`CApplicationFrameService_CreateInstance`, which accepts an
`IImmersiveApplicationManagerInternal*`. Thus the missing Explorer role is
a running application frame service, not just the host process or the public
`IApplicationActivationManager` launch call. These symbols identify internal
Windows APIs; they do not establish a stable supported contract for Nickel.

`CApplicationFrameService_CreateInstance` is an internal function in
`twinui.pcshell.dll`; it is not one of that DLL's three PE exports. Its
signature requires `IImmersiveApplicationManagerInternal*`. On this Windows
Home installation, `CustomShellHost.exe` is absent. Microsoft's supported
[Shell Launcher v2](https://learn.microsoft.com/en-us/windows/configuration/shell-launcher/)
path for a desktop replacement shell launching UWP apps is available on
Enterprise, Education, and IoT Enterprise editions, not Home. A Home-compatible
Nickel path would need to supply the missing frame service integration or use
another Windows-supported activation arrangement available on Home.

## Frame host COM path

Headless Ghidra analysis of the installed Windows binaries found one call to
`CoRegisterClassObject` in `ApplicationFrameHost.exe` at `0x140001fae`, inside
`FUN_140001d80`. It publishes
`{B9B05098-3E30-483F-87F7-027CA78DA287}` as a reusable local server.
Before that call, the same function creates the in-process `Application Frame`
class `{DDC05A5A-351A-4E06-8EAF-54EC1BC2DCEA}` from
`ApplicationFrame.dll`, requesting interface
`{A914F499-4633-4B26-A93E-707EB5BDC0B6}`. The DLL exports
`DllGetClassObject` at `0x180033760`; it dispatches through a class table
initialized by `FUN_18003c6d0`. These are static code paths, not proof of
which step stalls in the Explorer-free session.

## Immersive manager control

The isolated `nickel-windows-frame-probe` tested the registered Immersive
Application Manager and ImmersiveShell COM classes. Directly constructing the
manager returned `0x8027FFFF` with Explorer present or absent. The
ImmersiveShell local server was available with Explorer running but returned
`REGDB_E_CLASSNOTREG` without it; two candidate `QueryService` calls returned
`E_NOTIMPL` when Explorer was present. Public-symbol disassembly places
`CApplicationFrameService_CreateInstance` inside
`CApplicationManager::_InitializeIAMSubcomponents`, followed by
`IUnknown_SetSite` on the new frame service. This is more startup machinery
than a standalone `CoCreateInstance` of ApplicationFrameHost.

A cold-host Explorer-off control again timed out in `ActivateApplication`.
On that run, the target logged `initialize` and did not log `set-window` until
Explorer was restored. A previously initialized frame host survived Explorer
shutdown in one successful run, but a controlled repeat with a warm frame
host still timed out. Therefore a warm host is not a reliable fix.

Further tracing of `0x8027FFFF` found that the in-process Immersive
Application Manager requires `PrivilegedOperationsService::s_this` before it
constructs the frame service. The standalone probe lacks that singleton.
Creating the private service through Windows' own DLL class factory reaches
`NtUserAcquireIAMKey`, which returned `E_ACCESSDENIED` in the original probe
without a registered shell window. The direct-manager error therefore
identifies a shell startup prerequisite; it is separate from the later
packaged-app activation timeout.

A later isolated probe registered a hidden shell window through
`SetShellWindowEx` while Explorer was stopped. `NtUserAcquireIAMKey` and
private-service construction then succeeded. Manager startup advanced to
`CreateWindowInBand` for immersive background band `12`, which failed with
Win32 `ACCESS_DENIED` in the unsigned probe, before frame-service creation.
See the frame probe README for the evidence and limits of that result.

## Explorer-off view-creation wait

A noninvasive stack capture of the target while Explorer was absent located
the blocked call in `twinapi_appcore!CoreApplication::GetWindowFactory`.
That function obtains `IApplicationActivationBroker` from the registered
`ShellServiceHostBrokerProvider` COM class
`{3480A401-BDE9-4407-BC02-798A866AC051}`. The target was waiting for an
RPC reply from the broker method at vtable offset `0x28`, whose arguments
match `CApplicationActivationBroker::GetViewActivationResult`. The target's
main thread was waiting in `CoreApplication::WaitForExit`, and its application
view thread was waiting for view readiness. This is the precise point behind
the `initialize`-to-`set-window` gap; it is not evidence that the target's own
`SetWindow` callback is slow.

The broker's result depends on a view activation path that has not yet been
traced to a particular missing Explorer-provided service. An Explorer-off
stack capture of `sihost.exe` and the existing Application Frame Host did not
show an active broker or frame-creation call at that instant. Explorer was
restored immediately afterward, and the target then reached `set-window`,
`load`, and `run`.
