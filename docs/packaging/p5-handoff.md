# P5 candidate handoff

**Preparation only. No complete candidate is available yet, and no device test has passed.** macOS builds/inspection are underway; native Windows and Ubuntu build hosts are unavailable in this task. The owner has three laptops, but their exact OS versions and CPU architectures have not been recorded. A macOS ARM64 build does not validate Intel hardware.

This handoff authorizes no installation, native app launch, runtime download, AWS access, release or publication. The examples below are instructions for a later explicitly authorized P5 session. Use [the P5 plan](../roadmap/phase-5.md) for the single acceptance checklist.

## Identify the candidate before testing

The [p4-v1 matrix](../../packaging/targets.json) requires four targets and seven distributables. `{version}` must come from the accepted candidate manifest; the source version at preparation is `0.2.9`.

| Target | Required files | Evidence at preparation |
| --- | --- | --- |
| macOS ARM64 | `cloud-burrito_{version}_aarch64_unsigned.dmg`, `cloud-burrito_{version}_aarch64_unsigned.app.zip` | Native build/inspection pending |
| macOS Intel | `cloud-burrito_{version}_x86_64_unsigned.dmg`, `cloud-burrito_{version}_x86_64_unsigned.app.zip` | Native build/inspection pending |
| Windows x64 | `cloud-burrito_{version}_windows_x86_64_unsigned_setup.exe` | Native build unavailable |
| Ubuntu x64 | `cloud-burrito_{version}_ubuntu22.04_x86_64_unsigned.deb`, `cloud-burrito_{version}_ubuntu22.04_x86_64_unsigned.AppImage` | Native build unavailable |

Before handoff, attach `candidate-manifest.json`, `SHA256SUMS`, and all four `build-manifest-<target-id>.json` files from the [candidate assembler](provenance.md). Record the exact source commit, version and artifact hash in every device result. All seven files must share the same accepted source/version. A partial target set is not a complete candidate.

The candidate floors are macOS 13.0, Windows 11 24H2 x64 and Ubuntu 22.04 x64. They are proposed acceptance boundaries, not existing support claims. Record actual CPU architecture, OS release, desktop/WebView runtime and installation method before selecting a file. Stop if a laptop does not match a declared target.

## Verify transferred bytes

Obtain the manifests/checksums with the candidate through the agreed transfer route. Checksums detect changed bytes relative to that evidence; unsigned metadata does not authenticate a publisher. Do not continue after a mismatch.

For the complete transferred directory on macOS:

```sh
shasum -a 256 -c SHA256SUMS
```

For the complete transferred directory on Ubuntu:

```sh
sha256sum -c SHA256SUMS
```

For the selected Windows installer in PowerShell, compare the full output with its exact `SHA256SUMS` entry:

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath '.\cloud-burrito_0.2.9_windows_x86_64_unsigned_setup.exe'
```

The Windows example assumes version `0.2.9`; use the manifest's filename if different. A one-file comparison validates only that file. Full-set verification can also use `python3 scripts/assemble_candidate.py verify <candidate-directory>` from the matching reviewed source when Python is available; Python is not an end-user app requirement.

## Install only during the authorized device session

**macOS:** choose the DMG matching the Mac's CPU. Open it, copy `Cloud Burrito.app` to the chosen Applications location, then eject it. Test the ZIP alternative separately by extracting its app bundle; avoid confusing two installed copies. Record any Gatekeeper prompt and actual launch outcome. The app has no Developer ID/notarization; never disable system security globally or strip quarantine recursively to manufacture a pass. See [macOS policy](macos.md).

**Windows:** run the verified x64 setup executable and record its destination, shortcuts and prompts. The configured default is a per-user installation under `%LOCALAPPDATA%\Cloud Burrito`. Existing Evergreen WebView2 should be used. If missing, the installer may download Microsoft's bootstrapper; runtime acquisition/network access requires separate approval. Use a disposable test system for the missing-runtime case, rather than removing a runtime used by other apps. See [Windows policy](windows.md).

**Ubuntu:** test the `.deb` primary route and AppImage secondary route separately on the declared baseline. After approval, a local package installation example is:

```sh
sudo apt install ./cloud-burrito_0.2.9_ubuntu22.04_x86_64_unsigned.deb
```

Review dependency/elevation prompts before proceeding; downloading runtime dependencies is a separate authorized setup action. The generated package metadata determines GTK/WebKitGTK requirements. For the verified AppImage, grant execution to that single file, then open it during the device session:

```sh
chmod u+x ./cloud-burrito_0.2.9_ubuntu22.04_x86_64_unsigned.AppImage
```

AppImage still has host requirements, including a possible FUSE 2 compatibility need. Record a blocked launch without automatically installing libraries or executing extraction workarounds. See [Ubuntu policy](linux.md).

AWS CLI is optional for the CLI widget and is not bundled. Rust, Node and native compiler tools are build dependencies, not end-user requirements. No AWS login, credential inspection, account connection or connected query belongs to the initial installation checks.

## Retention and evidence

Use a disposable OS user/home with synthetic data for lifecycle and no-credentials tests. Leave the owner's real AWS files and credentials unopened. Install, upgrade and uninstall steps must preserve existing `.aws` and `.cloud_burrito` contents; intentional in-app settings saves are tested separately. Keep any installer option to delete application data unselected. Remove the installed application through its normal OS route; do not remove either user directory.

Capture only sanitized observations: target, source/version/hash, OS/runtime versions, case ID, expected/observed behavior, pass/fail/blocked/not-run status and an optional redacted screenshot. Never attach credentials, real AWS identifiers, personal filesystem paths or raw application/compiler logs.

Known open gates: complete four-target artifact set; actual laptop inventory; native runtime/launch behavior; upgrade baseline; real-device journeys; and separately authorized connected testing in a designated test account. Packaging preparation and automated source tests do not close these gates.
