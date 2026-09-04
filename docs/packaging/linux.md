# Ubuntu candidate policy

[`packaging/targets.json`](../../packaging/targets.json) is the sole target and
artifact matrix. Its Linux row declares native Ubuntu 22.04 x64 with glibc 2.35
and WebKitGTK 4.1 as the build baseline. The `.deb` and AppImage both contain an
x86-64 application. Ubuntu 22.04 is a candidate baseline; newer Ubuntu releases
and other distributions need their own P5 evidence. Static checks do not turn
this into a claim of support for every Linux desktop.

## Build dependencies and application runtime

Use the declared oldest baseline to avoid accidentally introducing a newer
glibc requirement. Record the OS, installed toolchain, system library and SDK
versions. The build host needs the Tauri development toolchain and system
development packages; end users do not need compiler headers merely to run the
application. Follow the official prerequisite list when provisioning a host
under a separately authorized setup task. No package installation or helper
download is part of these static checks.
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/),
[Tauri Debian packaging](https://v2.tauri.app/distribute/debian/)

The overlay selects `deb` and `appimage`, Debian section `utils`, priority
`optional`, and no extra media framework. It intentionally leaves `deb.depends`
unset. The pinned CLI generates the required `libwebkit2gtk-4.1-0` and
`libgtk-3-0` dependencies; extra feature-driven dependencies must be checked
in the actual generated control file. Development-package names are not a
substitute for runtime dependencies. Ubuntu Jammy provides the WebKitGTK 4.1
development package needed for this build baseline.
[Ubuntu Jammy package](https://packages.ubuntu.com/jammy/libwebkit2gtk-4.1-dev)

The package uses the existing generic Cargo author, `Cloud Burrito contributors`,
as its generated Maintainer. No person, contact address or publisher identity is
invented. A missing contact address is an explicit metadata limitation: Debian
Policy expects a name and email address in that field. This local unsigned
candidate is not evidence of Debian archive acceptance. If the project later
chooses a public project contact, review that separately.
[Debian Maintainer field policy](https://www.debian.org/doc/debian-policy/ch-controlfields.html#s-f-maintainer)

AppImage does not remove the glibc compatibility floor or all host desktop
requirements. FUSE availability can affect execution: Ubuntu 22.04 hosts may
need the FUSE 2 compatibility library even when FUSE 3 is installed. Verify the
actual laptop's runtime route in P5. The absence of bundled GStreamer media
support is intentional; do not promise arbitrary media playback.
[Tauri AppImage limitations](https://v2.tauri.app/distribute/appimage/#limitations),
[AppImage FUSE guidance](https://docs.appimage.org/user-guide/troubleshooting/fuse.html)

AWS CLI and authentication helpers are not bundled. The package has no custom
install/remove hooks, system service, updater or extra filesystem payload.
Installation/upgrade/removal must preserve the user's `.cloud_burrito` directory
and AWS files. Manual updates replace the application; actual preservation and
desktop integration remain P5 device checks.

## Static inspection before device testing

Use the shared build wrapper on the declared native host, with the
[reviewed local helper cache](native-helper-cache.md). It requires all five
AppImage helpers even with media bundling disabled. Keep compiler dependency
resolution offline and separately enforce the network boundary for native
helpers. No AppImage helper or installer is acquired automatically.

```sh
python3 scripts/platform_config.py --platform linux
```

Inspect the completed artifacts without installing or launching them:

- Use `dpkg-deb --info`, `--contents`, `--control` and `--extract` in dedicated
  inspection directories to review the control data and application payload.
  These inspect/extract a package; do not replace them with `dpkg -i` or an
  installation command.
- Check `Architecture: amd64`, exact version, generated dependencies, generic
  Maintainer, desktop entry, icon, executable permissions and expected paths.
  Inspect any generated maintainer scripts as data; no custom lifecycle hook
  or unrelated payload is permitted.
- Read the application ELF header, dynamic dependencies and symbol-version
  requirements with non-executing inspection tools. Record its highest GLIBC
  requirement; a build-host label alone does not prove compatibility. Compare
  against the matrix baseline and fail any newer requirement.
- Inspect the AppImage with an external archive/SquashFS reader, then verify
  the extracted application has the same architecture and application SHA-256
  as the Debian payload. Invoking the AppImage's own `--appimage-extract` still
  executes its runtime, so it is outside this static inspection policy.
- Record artifact SHA-256 and contents. An unsigned `.deb` and unsigned
  AppImage are candidate files, not a configured package repository or an
  automatically trusted distribution channel.

P5 covers opening the installed application, desktop integration, permissions,
WebKitGTK behavior, keyboard/UI journeys, manual upgrades and removal with
settings preserved on the owner's Ubuntu laptop. A different desktop session,
distribution or Ubuntu release needs named evidence before support is claimed.
