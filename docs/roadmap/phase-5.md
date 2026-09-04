# P5 — Validate the packaged candidate on real devices

**Planned; no device tests run or passed.** Use the [candidate handoff](../packaging/p5-handoff.md) for file names, transfer verification, runtime policies and installation examples. P4 supplies inspected artifacts; P5 supplies actual installation and application evidence. This document does not authorize native execution, AWS access or publication.

## Ordered work

| Unit | Work and exit condition |
| --- | --- |
| P5-01 — Freeze devices and candidate | Record each laptop's OS release/CPU/runtime and match the matrix; identify the complete candidate by source, version and checksums. Record unavailable target hardware explicitly. |
| P5-02 — Install and launch locally | On an authorized disposable test user/home, cover fresh installation, runtime present/missing, no-credentials startup, shortcuts, focus, display scaling and normal close/reopen. |
| P5-03 — Exercise product journeys | Run the shared cases below. Connected portions require separate explicit AWS authorization and a designated test account; keep them blocked until then. |
| P5-04 — Persistence and lifecycle | Confirm restart, settings/layout recovery, approved prior-version upgrade, same-version reinstall and uninstall with user data preserved. |
| P5-05 — Decide readiness | Attach case evidence and defect retests per OS/architecture. Support claims require completed evidence and no unresolved critical/high defects. Release/publication remains a separate decision. |

The three available laptops do not automatically cover all four targets: Intel and ARM64 macOS are distinct. A missing target remains unvalidated, even if its cross-compiled package passes static inspection.

## Shared acceptance cases

All statuses below begin **not run**. Record separate results for each target and each distribution route; link a shared observation only when it genuinely covers both.

| Case | Required observation |
| --- | --- |
| Install / runtime | Correct architecture, version, destination and launch entry. Existing runtime works; missing runtime gives an honest setup or blocked outcome. Record trust/elevation/network prompts without disabling OS safeguards. |
| No-credentials first launch | A disposable home has no usable credentials. Startup remains responsive, connection-dependent work stays unavailable, and missing/malformed config has a visible Settings/retry route. No AWS request or login is attempted. |
| CB-J01 — Identity | Local configuration recovery is understandable. After separate AWS approval: supported SSO verifies the intended identity before enabling work; expired/mismatched identity fails visibly. Missing optional CLI affects CLI features only. |
| CB-J02 — Context | After separate AWS approval: account/region changes cannot display stale results as the new identity; pinned cards and nested views retain their own context. Delayed/order failures need controlled fixtures where necessary. |
| CB-J03 — Investigation | After separate AWS approval: Pipeline → Build → Stack → Logs preserves context, unknown associations stay explicit, and empty/denied/partial/failed results remain distinct. Query scans need an approved bounded test scope. |
| CB-J04 — CLI | No native AWS CLI process during the offline checks. After separate approval: an exact allowed operation uses the verified context; forbidden operations/overrides are rejected. Timeout/cancellation/oversize failures use controlled synthetic fixtures, not destructive cloud actions. |
| CB-J05 — Durability / recovery | Saved theme, region, settings and layout survive normal close/reopen. On an injected/disposable storage failure, success is not reported and previous durable values remain. Retained results keep their context/freshness labels during failed refresh. |
| Upgrade / reinstall | Select and record the supported prior artifact before testing. Save synthetic settings/layout, close normally, install the next candidate, and verify preservation and one expected application entry. Same-version reinstall is a separate case. Windows downgrade refusal is tested separately; unsupported downgrade is not an upgrade baseline. |
| Uninstall / retention | Normal OS removal removes the application/expected shortcuts. Existing `.cloud_burrito` and `.aws` remain untouched by removal; verify only disposable synthetic fixtures. Reinstall can recover preserved settings. |

The [original CB-J01–CB-J05 contracts](phase-0.md#acceptance-fixtures) define the journeys; their historical baseline paragraphs are not current implementation claims. P1–P3 automated evidence complements these native observations and does not substitute for them.

## How to execute without overloading the session

Start with one identified laptop, one matching verified artifact, and P5-02's offline checks. Record failures before trying another format or machine. Retest a fixed defect against a new clearly identified candidate; never quietly reuse the old artifact's result. Runtime installation or repair requires an explicit scoped setup decision.

Before connected work, agree the test account/profile, allowed operations, narrow query scope and scan-cost boundary. The owner manages credentials; evidence must omit their contents. No production resource change, broad discovery, live destructive denial test or implicit AWS retry is part of this plan.

For each case, record: `target / OS + CPU / runtime / artifact + SHA256 / source commit / case ID / expected / observed / status / defect reference`. Use `blocked` for missing hardware, runtime, candidate or authorization; use `not run` when no attempt occurred. Only observed successful behavior earns `pass`.

P5 closes only when the declared target coverage, complete artifact set, lifecycle and journey evidence meet the gate above. Missing platforms remain named limitations. The next step is a separate portfolio/publication decision, with no automatic tag, upload, release or push.
