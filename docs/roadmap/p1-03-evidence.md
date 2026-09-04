# P1-03 — Verified connections and explicit SSO configuration

Status: **implemented and locally validated**, 2026-09-04. This is a local
working-tree change on top of P1-01/P1-02, version `0.2.9`, based on `095d1ad`.
No commit, push, release, live AWS call or real AWS CLI execution was performed.
Phase 1 as a whole remains in progress.

## Result

The selected account becomes active only after STS verifies credentials resolved
from its selected SSO profile. Resource clients receive that same fixed
credential snapshot. A mismatched account, incomplete identity, expired
credentials or changed principal cannot become a verified connection.

- Configuration is parsed from an explicit, bounded snapshot of the selected
  AWS file and profile. Inline SSO and named `[sso-session]` profiles are
  supported. Ambiguous sections, conflicting fields, static/process/role-chain
  credentials and endpoint/service indirection are rejected. There is no default
  profile fallback or mutation of `AWS_CONFIG_FILE`.
- The native adapter builds SDK configuration explicitly, without a default
  credential chain or environment credential/endpoint fallback. It checks the
  selected token cache's start URL and SSO region. Named sessions retain the
  locked SDK's OIDC token-renewal behavior; legacy profiles require a new external
  login when their token expires.
- SSO role credentials must have an expiry. An expiry-checking fixed provider
  with SDK identity caching disabled supplies both STS and resource clients.
  Near-expiry credentials are renewed and verified again. A changed account,
  ARN or user ID invalidates the context instead of silently replacing it.
- Connection attempts are ordered atomically: the latest attempt wins, including
  failures. Starting verification clears the previous active connection. Late
  credential/STS responses cannot restore an older selection.
- Pinned contexts resolve and verify independently. Their cache binds the full
  configuration snapshot, actual identity and credential-provider revision.
  Configuration-path changes invalidate active/cached contexts and pending
  attempts; selected profile/session changes are checked before work and before
  accepting asynchronous results.
- Backend result fences discard work from superseded connections. A result from
  an older credential revision cannot disconnect a newly verified revision of
  the same context. Frontend auth polls and identity-panel refreshes also discard
  obsolete responses instead of repainting a previous account.
- Local operation gates run before identity/provider work, including the
  StopQuery prerequisite for both query workflows. The existing per-operation
  widget gates remain in place.

**Temporary compatibility change:** desktop `aws-cli` execution returns
`CliContextUnavailable` until P1-04 implements verified credential handoff and a
constrained child environment. A CLI child must not inherit unrelated credentials
while the application labels its account verified. The P1-02 parser, its 18
reviewed operations, argument schemas, fake-runner tests and saved pins remain.
Internal SDK widgets use verified connections.

## Regression and checks

Before implementation, a controlled synthetic regression completed account B's
credential request first, then account A's older request. A became active,
failing the expected latest-selection assertion. No real credentials or network
were involved. The replacement command suite covers this ordering through both
credential resolution and STS verification, including old success/failure and
newest-attempt failure.

| Check | Observed result |
| --- | --- |
| Full Rust library suite, locked/offline, normal parallel execution | **97 passed; 0 failed; 0 ignored; 0 filtered** |
| Production frontend auth handlers through a deferred mock bridge | **5 passed; 0 failed** |
| Account mismatch, spoofed pinned account and incomplete pinned scope | Rejected; no unverified active connection or inherited-account fallback |
| Credential/STS completion ordering and settings changes during verification | Late results discarded; latest attempt preserved |
| Expiry, same-principal renewal, principal change and old-revision results | Verified renewal or invalidation as appropriate; fresh context survives stale results |
| Profile/session changes and independently cached pinned contexts | Invalidated or separately verified; no inherited-account fallback |
| Policy denial and missing query-cleanup permission | Zero provider, identity and process work |
| Desktop CLI execution | Blocked before provider or process work |
| Config parsing, fixed credential expiry and token-cache metadata | Synthetic fixtures pass; no personal AWS files used |
| All-target Clippy, locked/offline, warnings denied | Pass |
| Rust formatting, JavaScript syntax and diff whitespace | Pass |
| Version consistency and Tauri command registry | Version 0.2.9; unchanged 14 commands |
| Privacy scan of Rust source, frontend, README, roadmap and unit tests | Pass |

The Rust tests use isolated stores, controlled async completions, synthetic
identities, an injected clock and failing native/provider/transport/process
boundaries. The frontend tests execute the actual source handlers in Node with
a deferred mock bridge; they do not launch a browser or desktop app.

Reproduce from `src-tauri`:

```sh
env -u AWS_ACCESS_KEY_ID -u AWS_SECRET_ACCESS_KEY -u AWS_SESSION_TOKEN \
  -u AWS_PROFILE -u AWS_DEFAULT_PROFILE AWS_EC2_METADATA_DISABLED=true \
  cargo test -p cloud-burrito --locked --offline --lib
cargo clippy -p cloud-burrito --locked --offline --all-targets -- -D warnings
cargo fmt --all --check
```

From the repository root:

```sh
npm run test:unit
node --check frontend/app.js
python3 scripts/check-release-version.py
python3 scripts/check-tauri-commands.py
python3 scripts/check-release-privacy.py --path src-tauri/src --path frontend \
  --path README.md --path docs/roadmap --path tests/unit
git diff --check
```

## SDK review and limits

The implementation was reviewed against locally cached, locked primary SDK
source: `aws-config` 1.8.18 (`sso/token.rs`), `aws-runtime` 1.7.5
(`env_config/normalize.rs` and `fs_util.rs`), and the locked service/client
configuration. Supplying the full explicit SDK config avoids the token builder's
default loader. The returned token's expiry is checked separately because failed
renewal can return the cached token. Profile aliases and Windows token-cache home
precedence were checked against the same locked source.

The only new production dependency declaration is `aws-sdk-sso` 1.102.0, already
present in the lockfile/cache. No transitive dependency version changed. The
P1-01 development dependency is preserved.

These are offline source and behavioral checks, not proof of a successful real
SSO login, OIDC renewal or AWS service request. The public token provider rereads
its cache after metadata preflight; there is no atomic filesystem lock against
concurrent external replacement. Provider-internal OIDC requests are not
individually audited by the application registry. Existing audit records remain
preflights, not wire-call outcomes.

Native/webview acceptance on macOS, Windows and Ubuntu is still pending. Full
frontend tile/detail ownership and stale-result presentation remain P1-05; audit
and diagnostics redaction remain P1-06. Bounded CLI streaming, timeout and
cancellation belong to P1-04; remote query cancellation/cleanup remains P3.
This phase does not establish installer compatibility or performance claims.

Next: **P1-04 — verified CLI credential handoff and constrained process execution.**
