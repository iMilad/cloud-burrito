# P2-07 — Keyboard and readable desktop use

Status: complete locally; automated checks and visual review passed, native evidence pending. Base: `b920841`, version `0.2.9`.

## Scope

CB-J01–05 critical controls: context and pipeline selectors, side panels,
Live/Pinned and stack tabs, expanding evidence, refresh and recovery. Preserve
the existing framework, request ownership and evidence contracts.

Closed panels must be excluded from keyboard navigation. One shared lifecycle
manages their initial focus, Tab containment, Escape, close/return focus and
scrim state. Panel overlays must remain above a fullscreen widget. Selection
and refresh must not unnecessarily discard the control the user is operating.

Tabs and comboboxes expose their selected/active state and keyboard navigation.
Result metadata remains readable while live announcements convey concise state
changes instead of repeating full coverage on every local filter update.

The existing light/dark palette is retained. Compact controls can grow with
text, header/actions can wrap, and panels and wide evidence scroll locally.
Focus is visible and status labels do not rely only on colour.

## Validation

Seven grouped production-frontend fixtures pass in the focused run. They cover
all five panels, tab/combobox navigation, focus preservation, concise status,
fullscreen/panel stacking, and both themes at 1280×800 and 1024×768, 200% CSS
zoom and explicitly doubled computed text sizes. Geometry checks keep primary
controls reachable, Save/Retry operable and wide evidence inside a local scroll
area. Selected critical foreground/background pairs meet a 4.5:1 contrast ratio;
this is a bounded check, not a claim covering every possible colour combination.

Four compact/zoom screenshots across both themes were visually reviewed: no
clipped Save/Retry actions or overlapping panel content were observed. The zoom
fixture models CSS scaling in installed Chrome, not a native OS scaling claim.

Final verification:

- All 94 production-frontend browser cases pass in one installed-Chrome worker (1.5 minutes, exit 0), including the final repeated-connection announcement check. The preceding ownership, persistence, settings, result-state, investigation and hostile-rendering cases remain intact.
- All 248 locked, offline Rust library tests pass; binary/doc test targets also complete successfully. All-target Clippy passes with warnings denied.
- All 17 Node ownership/auth cases and 16 Python release/helper cases pass. The Node auth fixture stubs only the modal presentation helper; its production polling and obsolete-response assertions remain unchanged.
- The exact 15-command registry, version consistency, syntax, formatting, whitespace and tracked/new-file privacy checks pass. Version remains `0.2.9`.
- The repository security script exits 0. Gitleaks current tree/history and detect-secrets report no findings; TruffleHog reports zero findings with sandbox temporary-artifact cleanup warnings. Its cleanup warnings are not represented as a successful native cleanup check.
- The dependency audit uses its existing cached database without fetching. It retains 20 allowed warnings and does not certify fresh advisory status. No dependency was changed or installed.

Only synthetic provider/process fixtures, disposable storage and localhost browser bridges were used. No AWS, credential inspection, actual CLI execution, native app launch, remote Git or publication action was performed. Each P2 unit has its own local commit; native and live-provider acceptance remains deferred.

## Limits / next

These are browser fixtures with synthetic native boundaries, not accessibility
certification or native webview acceptance. Real keyboard/focus, clipboard,
fonts, filesystem and install/reopen journeys remain P5 on each declared OS and
architecture. No AWS connection or actual CLI execution is needed for this unit.

After P2, P3 addresses measured performance and bounded work. Query cancellation
and cleanup remain subject to P1/P2's existing explicit uncertainty until P3
implements and verifies the provider cleanup paths.
