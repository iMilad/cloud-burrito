# P2-03 — First run and recovery

Status: complete locally; automated checks passed, native evidence pending. Base: `92cc37e`, version `0.2.9`.

## Result

CB-J01: discover the configured profiles, explain unsupported or unavailable
prerequisites, verify the selected identity, and recover through visible controls.
CLI availability is an optional local check and never starts a subprocess.

Discovery and verification share the same logical-section index and pure SSO
rules, including duplicate aliases and the saved session constraint. Discovery
limits configuration input to 2 MiB and returns at most 500 logical profiles with explicit
omission counts. Missing, unreadable/nonregular, malformed/oversize, empty, and
unsupported configurations have distinct safe recovery states. Only bounded
profile/account/role/resource-region/session metadata is returned; original
config contents and provider inputs are not diagnostics.

The frontend waits for fresh discovery before using an inherited connection.
Stored profile listings cannot authorize a new selection or populate pinned
choices. Visible Settings, Retry connection, and Identity controls expose
recovery, with the selected profile and actual verified account/region shown
together. A delayed poll cannot restore identity from an obsolete discovery.

The optional CLI route checks the runner's existing native/wrapper candidate
rules through an injected local command. Only discovery environment keys are
consulted; no version probe, credential lookup or subprocess is started. The
exact command registry is now 15, with the local main-window capability retained.

## Validation

- 200 locked, offline Rust library tests pass, including bounded discovery,
  malformed/duplicate/unsupported SSO cases, saved constraints, zero provider or
  process work, synthetic executable candidates, and native discovery fences.
- All-target Clippy with warnings denied passes.
- 17 Node production-handler tests pass, including no inherited work before
  discovery and rejection of delayed auth replies from an obsolete discovery.
- Three focused registry tests and the exact 15-command check pass.
- The full production-frontend suite passes 60 cases in one installed-Chrome
  worker (51.4 seconds, exit 0). After finalizing fixed connection/status failure
  messages, all 14 focused recovery cases pass (7.0 seconds, exit 0), including
  two added rejected-bridge scenarios. Settings, ownership and hostile-rendering
  assertions remain intact.
- Tracked/new-file privacy and current-tree Gitleaks scans pass.

Synthetic cases cover missing/unreadable/malformed/empty config, unsupported SSO,
expired credentials, mismatched identity, delayed discovery, status failure,
missing CLI and retry, and removal while CLI availability is pending. Dependency
correction remains manual; retry does not launch login or widen a capability.

No AWS, credential inspection, actual CLI execution, native app launch, remote
Git, release or publication action was performed.

## Limits / next

No automatic login, credential creation, new credential mechanism, AWS operation,
or executable operation is added. Candidate discovery does not establish AWS CLI
v2 version, publisher trust, successful execution, or cross-platform compatibility.
Native WebView, real SSO/provider and device acceptance remain deferred.
P2-04 next makes result state, freshness and partial evidence visible.
