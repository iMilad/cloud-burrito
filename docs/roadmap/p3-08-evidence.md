# P3-08 — Bounded rendering and responsive filtering

Tables mount at most 100 data rows per page, with at most ten remembered expanded rows. Filtering covers the complete returned dataset, including permitted structured CLI details, while stable source indexes keep full values associated with the correct row. Pagination preserves keyboard access and expansion identity. A same-context refresh restores the focused control and filter selection; a context change discards obsolete work. Small tables retain their row nodes for keyed progressive metadata updates.

Cells show bounded previews (512 characters, with shorter title/ARIA text). Explicit full-value disclosure opens one value at a time and respects the producer's withheld-value state. Source arrays still exist in memory; limiting DOM nodes does not prove a native memory ceiling.

Table, pipeline-picker and error-table regex matching runs in a same-origin worker. Input has a 512-character limit and 30 ms debounce, a 150 ms work deadline and a separate two-second worker-start deadline. An expensive expression terminates its worker and retains the previous matches with an explanation. Invalid regex retains the earlier case-insensitive literal fallback, visibly labelled. Generation checks reject obsolete answers; enrichment patches rerun an active filter. Filtering completion is measured separately from the immediate input handler.

Removing or invalidating a result disposes worker/timer resources. Retained result DOM can lazily restart its filter. Global pipeline-picker listeners are installed once; detached demo-log and clipboard timers stop. The worker is bundled as a local frontend asset; the existing CSP is unchanged. Actual worker behavior in packaged WKWebView, WebView2 and WebKitGTK remains a P4/P5 acceptance item.

## Validation

The complete browser suite initially passed 124 of 126 cases. Its two failures exposed premature CFN table clearing that lost filter focus, and an old exact-payload assertion missing P3-05's explicit cache-reuse flag. Both were corrected. All 17 cases in the affected browser files then passed, followed by 33 ownership/result-state regressions. Thus all 126 distinct browser cases are covered by the full run and targeted reruns. The focus regression waits for the actual replacement input before checking focus and caret.

Thirty Node cases pass, including worker matching, invalid-input fallback, patches and input limits. The final backend regression passes 339 library tests, with two explicitly opt-in benchmark entries ignored; all-target Clippy passes with warnings denied. Twenty-two Python helper cases and the repository security gate pass. The helper registry assertion now expects the 17 reviewed IPC commands introduced across P3. Public benchmark fingerprints are distinguished from secret findings only after exact local-source/Git verification; other findings remain fatal. Large-table tests cover 100/1,000/10,000 source rows, full-value identity after filtering, pathological regex, progressive filter updates, context changes and ten worker add/remove cycles.

## Measurement and handoff

The [P3 exit report](p3-exit-evidence.md) compares the retained before/after measurements and records scanner results, source provenance and open native gates. P3's eight implementation units are complete locally after final checks; the next unit is P4-01, freezing the unsigned artifact and compatibility contract.

No AWS, real AWS CLI, credential source, native application, GitHub, push, tag or release action was used.
