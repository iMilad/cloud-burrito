# Windows candidate policy

The sole target definition is [`packaging/targets.json`](../../packaging/targets.json).
Its Windows row declares an unsigned x64 NSIS installer, a native Windows 11
build host with Visual Studio 2022 C++ Build Tools and a recorded Windows SDK,
and Windows 11 24H2 x64 as a **candidate** device floor. Configuration checks do
not change that row to build-verified or device-validated.

## Installer and runtime choices

`src-tauri/tauri.windows.conf.json` uses the pinned Tauri CLI 2.11.4 configuration
shape. It selects NSIS `currentUser`, English, and the existing application icon.
The generated installer requests user-level execution and defaults to
`%LOCALAPPDATA%\Cloud Burrito`; install registration is per user. The application
does not need a machine-wide installation. The WebView2 runtime has its own
installation and update rules. [Tauri Windows installers](https://v2.tauri.app/distribute/windows-installer/)

The WebView2 policy is `downloadBootstrapper`, with silent runtime installation.
The generated installer checks for an existing runtime. If absent, it downloads
Microsoft's Evergreen bootstrapper; this path can need network access and can
fail. This is not an offline runtime bundle. Evergreen updates independently of
Cloud Burrito, and runtime scope or elevation can depend on the machine's
existing configuration and policy. Record the actual runtime version during P5.
[Microsoft WebView2 distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)

The installer, application and uninstaller remain unsigned. There is no
publisher certificate, timestamp service, custom signing command or updater
artifact. Build with the shared wrapper's `--no-sign` policy. Windows trust
prompts and enterprise installation policy remain P5 device observations; do
not present this configuration as eliminating those prompts. No personal
publisher identity is added. Existing product and bundle identifiers stay
stable, including Tauri's generated manufacturer metadata.

Updates are a deliberate installation of a later candidate. The overlay sets
`allowDowngrades:false`. Upgrade, same-version reinstall and uninstall still
need P5 evidence. The stock NSIS uninstall UI can offer deletion of Tauri's
application-data directories. In the pinned template that optional operation
targets the bundle-identifier directories in roaming/local AppData. No custom
hook removes the user's `.cloud_burrito` directory or any AWS configuration.

AWS CLI and existing user authentication tools are not packaged or installed.
Package inspection and device setup must not invoke AWS or inspect credentials.

## Build inputs and static inspection

Use the repository build wrapper on the declared native host. Before bundling,
run the static policy check and the [reviewed helper-cache preflight](native-helper-cache.md).
No helper is downloaded automatically. The native NSIS cache is a build input,
not an application runtime dependency. A Mac cross-build is not Windows device
evidence.

```sh
python3 scripts/platform_config.py --platform windows
```

Record the source commit, Rust/MSVC/SDK/CLI versions, target triple, helper
inventory digest and final artifact SHA-256. The matrix owns the release
filename. Inspect the generated `installer.nsi` and extracted payload without
executing the installer:

- Confirm the application PE machine is `AMD64` and its version/identity match
  the candidate. The NSIS setup stub may be a 32-bit PE even for an x64 payload;
  do not reject the correct application based only on the setup stub's machine.
- Confirm `currentUser`, user execution level, local per-user destination,
  expected English resources, runtime download branch and no added hooks.
- Check that application/setup/uninstaller have no publisher signature and
  that no extra installer, credentials, user state or development tools entered
  the payload. Confirm the icon and shortcuts point to the expected binary.
- Record the installer contents and supported uninstall paths. Do not execute
  installation, upgrade, uninstallation or the native app as part of P4 static
  inspection.

P5 supplies the missing Windows laptop evidence: normal launch, WebView2
present/missing behavior, trust prompts, UI/keyboard behavior, upgrade and
uninstall with settings preserved. No successful static check substitutes for
those journeys.
