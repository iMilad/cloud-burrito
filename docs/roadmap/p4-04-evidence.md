# P4-04 — Windows installer policy

Implemented locally on 2026-09-04. Native Windows build and installer inspection
remain pending; configuration and synthetic fixtures do not establish support.

The Windows overlay selects x64 MSVC NSIS, per-user installation, no downgrade,
and an Evergreen WebView2 download bootstrapper. It does not bundle AWS CLI,
credentials, signing material, an updater, or a personal publisher identity.
Missing WebView2 can require network access during the later installer journey.

The static policy validator checks the reviewed projection of Tauri CLI 2.11.4's
schema and exact project decisions. The native-helper preflight checks a supplied
reviewed inventory against every local NSIS toolchain file before any build. It
neither downloads nor executes helpers and is explicitly not a firewall.

Validation: 8 Windows policy fixtures and 10 synthetic helper-cache fixtures
passed; the actual overlay passes its static check. No Windows host, native
installer, OS warning, first launch, upgrade, or uninstall result is asserted.

See [Windows policy](../packaging/windows.md) and
[helper provisioning boundary](../packaging/native-helper-cache.md).
