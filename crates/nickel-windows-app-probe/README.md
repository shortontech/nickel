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

App activation can bring the selected app to the foreground. Use Calculator for the
first comparison; close it before switching sessions.
