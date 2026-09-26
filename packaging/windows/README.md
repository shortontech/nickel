# Windows MSI packaging

Tagged releases build a per-user x86-64 MSI on a GitHub-hosted Windows runner. The installer places
Nickel under `%LOCALAPPDATA%\Nickel`, creates Start menu shortcuts, and offers **Make Nickel the
default shell** as an optional feature. The feature is off by default.

When selected, `nickel-shell-setup.exe` writes the fully qualified Nickel executable to
`HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon\Shell`. The next sign-in starts Nickel
instead of Explorer. Removing the feature or uninstalling Nickel deletes that value only when it
still names the installed Nickel executable, allowing Windows to fall back to its normal Explorer
shell. A shell value changed after installation is left untouched.

## Build locally on Windows

Build and stage the four executables plus the license files as shown in
`.github/workflows/tagged-release.yml`, then run:

```powershell
dotnet build packaging\windows\Nickel.Installer.wixproj `
  --configuration Release `
  -p:ProductVersion=0.1.0 `
  -p:PayloadDir="$PWD\target\package\windows" `
  -p:OutputPath="$PWD\target\installer\"
```

The project pins WiX Toolset 5.0.2 through its MSBuild SDK and NuGet package references. A local
build therefore needs the .NET SDK but does not need a separately installed WiX toolset.

## Native acceptance

The release workflow performs an administrative MSI extraction and checks the packaged binaries.
Before calling a release installer fully accepted, test these steps on Windows:

1. Install without selecting the shell feature and confirm Explorer remains the sign-in shell.
2. Modify the installation, select the feature, sign out, and confirm Nickel owns the desktop.
3. Remove the feature and confirm the next sign-in starts Explorer.
4. Select the feature again, change the per-user `Shell` value manually, then uninstall and confirm
   the installer preserves that newer value.
5. Reinstall with the feature selected, uninstall normally, and confirm the per-user override is
   absent and Explorer starts at the next sign-in.

Release MSIs are unsigned until a code-signing identity is configured for the workflow. Windows may
therefore show an unknown-publisher warning.
