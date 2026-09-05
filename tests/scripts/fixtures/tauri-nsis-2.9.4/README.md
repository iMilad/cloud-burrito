# Pinned upstream NSIS regression inputs

These unchanged files come from the published `tauri-bundler 2.9.4` crate,
embedded in the project's pinned `tauri-cli 2.11.4`:
`src/bundle/windows/nsis/installer.nsi`, `utils.nsh`, `FileAssociation.nsh`,
and `languages/English.nsh`. They are test data, never executed or used to
customize the application's installer. The MIT license is included.

The installer template SHA-256 is
`20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079`.
The other upstream hashes are pinned in `scripts/artifact_inspection.py`.
The regression extracts the stock include directives and four dormant hook
blocks, supplies synthetic version/install-mode fields, and reproduces the
bundler's generated language path and UTF-8 BOMs. Native helper tree hashes
remain a separate, mandatory build-time inventory check.
