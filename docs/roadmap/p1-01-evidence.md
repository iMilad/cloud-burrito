# P1-01 — Isolated test boundaries

Status: **Implemented and locally validated on 2026-09-03.** P1 as a whole remains in progress.

Version: `0.2.9`. Changes are in the working tree based on `095d1ad`; no commit, push or release was created. Windows/Ubuntu and packaged/native-app acceptance remain deferred.

## Delivered

- [AppPaths](../../src-tauri/src/paths.rs) gives settings, dashboard, policy and audit functions an explicit data location. Production keeps the existing per-user directory and persistence/error/migration behavior. Tests must supply a temporary location; accidental default personal-storage access panics.
- [Runtime](../../src-tauri/src/runtime.rs) carries storage, AWS/provider/configuration, process and clock boundaries through `AppState`, `AwsContext` and widget dispatch. Native adapters retain existing behavior. Tauri command names, frontend arguments and response shapes stay unchanged.
- [ProcessRunner](../../src-tauri/src/process.rs) isolates executable discovery and child execution. Test builds cannot use the native runner; fake requests/responses exercise the real CLI fetch path. Existing command policy, environment, timeout and output buffering behavior are preserved for their later P1 work packages.
- [AWS test configuration](../../src-tauri/src/test_aws.rs) rejects unexpected credential and HTTP work before a connector or network operation can run. Shared counters make zero-boundary assertions observable. The fake SDK config does not load environment credentials, profile files, SSO caches or metadata providers.
- [Scoped temporary storage](../../src-tauri/src/test_support.rs) replaces all five HOME-mutating persistence fixtures. Each test owns a unique directory and cleans it up on drop. A two-store test covers independent settings, dashboard, policy and audit data.
- [Command fixtures](../../src-tauri/src/commands.rs) provide synthetic Demo A/Demo B credentials and identity responses, manually released asynchronous completions, a fixed clock, and failing unexpected process work. The previously excluded pinned-context test now uses explicit storage and passes in the normal suite.

The only dependency addition is an exact development dependency on `aws-smithy-runtime-api` 1.12.3, already present in the lockfile/cache. It exposes the test HTTP boundary; no crate version changed and no dependency download was needed.

## Validation

| Check | Observed result |
| --- | --- |
| Full Rust library suite, locked/offline, normal parallel execution | **63 passed; 0 failed; 0 ignored; 0 filtered** |
| Previous settings-reading test | Included and passed; no `--skip` or forced single-thread setting |
| Clippy, locked/offline, all targets, warnings denied | Passed; covers production and test compilation |
| Tauri command registry | Passed: 14 commands |
| Version consistency | Passed: 0.2.9 |
| Rust formatting and diff whitespace | Passed |
| Source privacy rules, including new Rust files | Passed: 30 files; synthetic account labels used |

Reproduce the Rust checks from `src-tauri`:

```sh
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  -u AWS_PROFILE -u AWS_DEFAULT_PROFILE AWS_EC2_METADATA_DISABLED=true \
  cargo test -p cloud-burrito --locked --offline --lib
cargo clippy -p cloud-burrito --locked --offline --all-targets -- -D warnings
cargo fmt --all --check
```

The suite now contains 17 additional tests compared with the 46-test P0 source snapshot. Tests cover controlled credential completion/failure, scripted identity, denied command/widget/process paths, safe SDK/provider boundaries, process result rendering, deterministic audit time and independent storage.

## What this proves and what remains

The real command/widget handlers can be tested without the user's settings, real credentials, real AWS calls or an AWS CLI child. Denial cases assert zero credential-provider, transport and process work where the existing gate precedes those boundaries. SDK configuration may still be built before widget preflight; the injected configuration performs no external discovery.

The delayed-completion fixture demonstrates controlled ordering. It does not prove stale-response rejection or account verification: those behaviors are intentionally unchanged and remain P1-03/P1-05. Exact CLI allowlisting, constrained child environment, streaming output caps and remote query cleanup are also not implemented by P1-01.

The HTTP/provider fences protect test clients constructed from the injected configuration. Future tests must keep those boundaries and await spawned tasks; creating a fresh default SDK loader would bypass that fixture. Native runtime guards reject accidental use of the production adapter in the library suite.

No desktop app launch, browser suite, live AWS/SSO integration, actual CLI process test or installer/device validation ran. Existing frontend source is unchanged. Offline compilation and unit results do not establish platform compatibility or a performance claim.

Next: **P1-02**, beginning with a regression test for a forbidden operation under wildcard policy and exact denial before process creation.
