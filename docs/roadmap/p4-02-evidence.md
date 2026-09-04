# P4-02 — Runtime portability without changing storage identity

Native startup requires an absolute user home before constructing Tauri or the audit/storage runtime. Missing, empty or relative homes fail with a fixed message instead of placing `.cloud_burrito` in the installation/launch directory. The normal data location stays unchanged; no migration or existing-user file inspection occurred.

Tilde expansion returns a typed failure when home is unavailable and accepts Windows `~\` spelling. Explicit custom paths preserve their spelling. The named SSO cache retains the SDK's environment-key precedence but rejects an invalid selected home rather than silently selecting a different cache. Tests inject these values; real AWS configuration/cache files are not opened.

Windows CLI discovery already supported `aws.exe` and standard installation paths. This unit requires absolute ProgramFiles candidates and suppresses a console window for the GUI-owned piped child. Direct executable selection, exact arguments, isolated home/environment, verified temporary credentials and P1 process supervision remain intact. The CLI is neither bundled nor installed; native AWS CLI compatibility remains pending.

Validation: 350 Rust library tests pass with two opt-in benchmarks ignored; all-target Clippy and formatting pass. Eleven portable fixtures were added, plus three Windows-only fixtures awaiting a Windows host. Cases include missing/relative homes, spaces/Unicode, no-PATH GUI discovery, drive/UNC settings round-trips, no-shell argument preservation and unreadable settings locations. Simulated Windows path strings on this Mac do not establish native Windows behavior.

No AWS call, real CLI execution, native application launch, user configuration mutation, remote Git or publication occurred. Next: shared native build adapters in P4-03.
