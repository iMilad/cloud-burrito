# P4-03 — Shared local build adapters

The Bash entry point now delegates to one Python candidate builder; a PowerShell wrapper provides native Windows invocation without shell argument reconstruction. The builder reads the P4-01 matrix, checks the native host/tool versions, and exports a clean immutable tracked Git tree. Ignored local files under the frontend directory cannot enter that export. Both lockfiles and relevant build/frontend/resource inputs are fingerprinted.

Builds use the pinned Rust/Tauri versions, `--ci --no-sign`, Cargo `--locked --offline`, a fixed verified target directory and encoded path-remapping arguments that preserve spaces/Unicode. Provider and signing environment inputs are removed from the child environment. Separate platform overlays preserve shared application identity, CSP, capabilities and frontend assets. The Windows/Linux policies are completed in P4-04/05; macOS details follow in P4-06.

The helper preserves previous bundle output in an ignored backup before building, requires a new candidate directory, and rejects missing/duplicate expected artifacts. It keeps the old macOS bundle output location for the existing separate release workflow. Building alone does not install, launch, upload, tag or publish. Format-aware acceptance is supplied in P4-07; an incomplete adapter cannot declare uninspected output accepted.

Native helpers use a verified local cache before bundling. Cargo offline mode alone cannot block arbitrary helper network activity, so it is not described as complete network isolation. In this task, absent Windows/Linux hosts or unprovisioned helpers stop the relevant build before acquisition. No GitHub helper download is permitted.

Four deterministic adapter tests pass: all target command plans retain locked/offline/no-sign flags, encoded remaps retain complete paths, missing/duplicate outputs fail, and wrong-host/dirty-source conditions fail before building. Bash syntax and all four plan modes pass. PowerShell execution and actual target builds remain pending the subsequent native gates.
