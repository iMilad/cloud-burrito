# P0 device checks

Status: **Deferred at the user's request during P1–P4 planning.** No output is needed now. Resume at Checkpoint 1 when the devices are available; these checks remain open evidence, not failed tests.

Run one checkpoint at a time. These inventory commands install nothing, contact no AWS service, and do not read AWS profiles or credentials. Collect OS/architecture first; choose build instructions after the prerequisites are known.

## Checkpoint 1 — Windows identity

In PowerShell:

```powershell
Get-CimInstance Win32_OperatingSystem |
  Select-Object Caption, Version, BuildNumber, OSArchitecture
Get-CimInstance Win32_ComputerSystem |
  Select-Object SystemType
```

Expected: Windows edition, version, build and OS bitness, followed by the system type distinguishing x64 from ARM64. OSArchitecture alone may report only 64-bit. This is the first user checkpoint. Record its output before moving to prerequisite discovery.

## Checkpoint 2 — Windows prerequisites

```powershell
$PSVersionTable.PSVersion
foreach ($cbTool in @('rustc', 'cargo', 'rustup', 'node', 'npm', 'git', 'aws')) {
  [PSCustomObject]@{
    Tool = $cbTool
    Installed = [bool](Get-Command $cbTool -ErrorAction SilentlyContinue)
  }
}
Get-ChildItem `
  'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients', `
  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients', `
  'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients' `
  -ErrorAction SilentlyContinue |
  Get-ItemProperty |
  Where-Object { $_.name -like '*WebView2*' } |
  Select-Object name, pv
$cbVsWhere = Join-Path ${env:ProgramFiles(x86)} `
  'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $cbVsWhere) {
  & $cbVsWhere -latest -products '*' `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationVersion
}
```

Expected: tool-presence flags, any registered WebView2 version and any detected Visual C++ build-tool version. Empty discovery is a prerequisite gap to investigate, not proof that the application cannot support Windows. Developer tools are source-build requirements, not requirements for every future installer user.

## Checkpoint 3 — Ubuntu identity and prerequisites

In a terminal:

```sh
cat /etc/os-release
uname -m
getconf GNU_LIBC_VERSION
for cbTool in rustc cargo rustup node npm git aws pkg-config; do
  if command -v "$cbTool" >/dev/null 2>&1; then
    printf '%s: found\n' "$cbTool"
  else
    printf '%s: missing\n' "$cbTool"
  fi
done
dpkg-query -W -f='${Package}\t${Version}\t${Status}\n' \
  'libwebkit2gtk-4.1-*' 'libgtk-3-*' \
  'libayatana-appindicator3-*' librsvg2-dev \
  libssl-dev build-essential pkg-config 2>/dev/null
```

Expected: Ubuntu release, architecture, glibc, tool-presence flags and installed native library/development packages. Missing-package results are evidence for the next setup step. Do not infer the supported Ubuntu baseline solely from the developer's installed distribution.

## Checkpoint 4 — Early build and launch spike

Once device inventory is recorded, prepare platform-specific instructions for the same source snapshot. Transfer source privately; no GitHub publication is required. Resolve missing prerequisites from the inventory using platform-specific setup instructions before attempting the build. Attempt a native source build and controlled launch with no real AWS activity; record exact errors before implementing fixes.

This early spike discovers compatibility problems. It does not validate installers or replace P6. Existing macOS release helpers cannot be assumed to work unchanged on Windows or Ubuntu.

## Evidence record

| Field | Value |
| --- | --- |
| OS version/build and architecture | Pending |
| Source commit/version | Pending; initial reference is `095d1ad` / `0.2.9` |
| Tool/runtime versions | Pending |
| Build command and result | Not attempted |
| Controlled native launch | Not attempted |
| First visible/usable window timing | Not measured |
| Failure text or screenshot | Pending, redacted |
| Follow-up issue and next checkpoint | Pending |

Keep shared evidence synthetic and redact usernames, real account/resource identifiers, configuration contents and credentials. A useful report names the failed step and observable result without exposing private data.

## Later P6 acceptance

Use packaged artifacts built from the same candidate commit and verify checksums. On each declared OS/architecture, test fresh install, OS warning behavior, first run, CB-J01–CB-J05, repeated refresh, cancellation, restart, upgrade and uninstall. Log actual results against the [journey contracts](phase-0.md); never substitute a mock browser pass for native acceptance.
