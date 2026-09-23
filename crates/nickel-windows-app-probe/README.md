# Windows packaged-app activation probe

This small executable calls Windows' `IApplicationActivationManager` directly. It has no
dependency on Nickel's shell, renderer, or launcher. It does not start Explorer or change
the Winlogon shell setting.

Build it from the workspace root:

```powershell
cargo build --release -j 1 -p nickel-windows-app-probe
```

Launch one installed app by its exact AUMID:

```powershell
.\target\release\nickel-windows-app-probe.exe 'Microsoft.WindowsCalculator_8wekyb3d8bbwe!App'
```

The default uses an out-of-process COM activation manager, as Nickel currently does.
Add `--inproc` to compare an in-process manager. Each run prints whether COM
initialization, manager creation, and app activation succeeded, including the HRESULT
at the first failure. Run the same command in a session with Explorer as the shell and
one with Nickel as the Winlogon shell to isolate session setup from Nickel's launcher.
An activation success only confirms the API call; the app's window lifecycle must be
observed separately.
The probe kills only its own activation worker after 20 seconds if Windows does not
return from the call.

## Current session baseline (2026-09-23)

With Nickel running and Explorer absent, Armoury Crate SE's AUMID reached
`CoCreateInstance` successfully in both local-server and in-process modes.
`ActivateApplication` then blocked until the probe's 20-second timeout in both modes.
Nickel remained responsive. This session was started before the Winlogon shell
registry value was changed to Nickel. After signing out and back in with Nickel
as the Winlogon shell, the local-server call again reached `CoCreateInstance`
and timed out in `ActivateApplication` after 20 seconds. Explorer remained absent
and Nickel remained responsive. Changing the Winlogon shell did not fix activation.

App activation can bring the selected app to the foreground. Use Calculator for the
first comparison; close it before switching sessions.
