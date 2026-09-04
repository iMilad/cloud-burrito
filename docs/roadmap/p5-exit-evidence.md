# P5 — Studio design and Classic fallback

Date: 2026-09-04. Final application source: `5d39eed` on the local `mk/p5-studio-design` branch. The P5-06 documentation commit follows this source without changing application bytes. Original P5 device work is now P6; original P6 portfolio work is now P7.

## What changed

Studio is the default presentation: a strong navigation rail, a focused overview, investigation shortcuts, clearer widget hierarchy, coordinated light/dark surfaces and a keyboard command launcher. The owner retained four related Studio styles: **Studio Original**, **Precision**, **Paper** and **Night Shift**. Paper is the fresh-install default. The overview describes a workflow; its arrows do not claim detected relationships between cloud resources. Counts reflect actual open widget instances, including duplicates. Browser data is explicitly labeled synthetic; desktop identity labeling follows the existing verified connection state.

Navigation focuses tools already in the workspace. Missing tools have disabled shortcuts; navigation never silently creates a widget or starts a resource request. Commands open actual widget instances or existing available panels. Settings and Audit remain unavailable in browser-only mode. Compact view changes presentation spacing, without rewriting GridStack geometry. Short highlights and hover transitions honor reduced motion; there are no new remote fonts, image services, dependencies or continuously running decorative animations.

The launcher and Appearance chooser use the existing modal focus owner, scrim and Escape behavior. The four-style control behaves as a keyboard radio group and announces a changed selection once. Style changes update the Studio mark and favicon immediately, without touching the theme, layout, widgets, account context or backend. The launcher releases its command closures on close, including references to removed widgets. Its observer watches direct grid children, not result rows. Existing result ownership, resource execution and backend code are unchanged.

Paper is also the canonical packaged icon. The existing PNG, ICO and ICNS paths were regenerated from the Paper SVG while keeping their tracked dimensions, alpha modes and icon frames. Runtime appearance changes affect only in-app marks and the favicon; launcher and installer icons stay Paper on every platform.

## Preservation and local commits

| Unit | Local commit | Result |
| --- | --- | --- |
| P5-01 | `5faa0c3` | Insert P5, renumber later phases, preserve historical evidence and record the rollback source |
| P5-02 | `e87c8a3` | Add the isolated, responsive Studio stylesheet; activation follows in P5-03 |
| P5-03 | `d211748` | Activate the new shell, commands, navigation, density and reversible design choice; add interaction regressions |
| P5-04 | `2fd99d8` | Record final local checks and the next device/package gate |
| P5-05 | `5d39eed` | Integrate four Studio appearances, make Paper the default, regenerate packaged icons and add regressions |
| P5-06 | Documentation commit containing this updated evidence | Record the selected design family, final validation and packaging handoff |

The pre-design source is `0a860cd`, retained by `mk/p4-design-baseline`. The original `frontend/styles.css` is unchanged. **Classic view** restores the original visual system; **Try Studio** returns to the new one. Neither action resets the dashboard, repins a context or changes the current theme.

The allowlisted view choice is stored under `cb.presentation.v1`; Studio is the default when it is absent or invalid. The four-style choice is stored separately under `cb.studio.appearance.v1`; Paper is the default when that key is absent or invalid. A blocked store still permits a session choice. The optional `?design=` and `?appearance=` preview parameters accept only known values and do not overwrite a stored preference. Compact mode is session-only. These are presentation preferences, not AWS settings.

## Validation

- Complete frontend run: all **139 individual cases reported passed with zero failures**. Chromium used browser mock data or the existing synthetic native bridge. No provider, native application or AWS request was involved.
- The focused Appearance and accessibility run exited normally with **13 passed**. It covers Paper fallback without an implicit write, all allowlisted styles, stored and URL-preview behavior, early mark synchronization before asynchronous native boot, Classic/theme independence, radio-key navigation, modal focus return, all eight style/theme combinations at 1480px and 1024px, and the chooser below its 520px breakpoint.
- The earlier P5 gates remain **30 Node unit** and **113 Python packaging/release helper** passes. No backend or helper source changed in P5-05.
- JavaScript syntax, local asset links and whitespace checks pass. The scoped release privacy scan passed across **301 files** on committed source `5d39eed`; it is not a fresh dependency, full-history or live-environment security certification.
- All 48 tracked native PNGs preserve their required dimensions and RGBA alpha. ICO decoding retains 16, 24, 32, 48, 64 and 256px frames; ICNS covers 16–1024px. The Paper icon was visually inspected at 256px. Actual launcher/Dock rendering remains a device check.
- Both `macos-aarch64 --check` and `macos-x86_64 --check` pass on `5d39eed`, reporting `built: false`, `device_validated: false` and `publication: false`. Windows and Ubuntu still need their declared native hosts and reviewed helper inventories.

Harness note: after all 139 individual cases reported success, the final Playwright runner again remained alive during teardown with no browser worker. Its localhost server and then the verified runner were stopped after more than two minutes. The redirected command therefore exited 143; the log contains 139 pass lines and zero failure lines. The focused 13-case gate exited 0 normally. This is test-process cleanup after assertions, not an unattended-run reliability claim. The full log is `/tmp/cloud-burrito-appearance-full-final.log`; the focused log is `/tmp/cloud-burrito-appearance-focused-final.log`.

A broad run exposed an intermittent existing picker race: a pending blur callback could erase a new query after the same input regained focus. The callback exists on the preserved P4 source. A deterministic clock-driven test failed before the fix; checking that the input is still unfocused before closing resolved it. That regression and the original CodeArtifact account-change case each passed three repeated runs. No account-selection assertion was relaxed.

Actual browser screenshots were reviewed for Paper in light and dark mode, the four-style Appearance chooser, Precision, Classic, the command launcher and compact layouts. The final Paper dark workspace and chooser were inspected again from the local `4191` preview. Review corrections include solid high-contrast focus rings, readable small labels, truthful storage copy, an early saved-mark sync and no duplicate initial status announcement.

## Try the local design

Serve the frontend locally and open its URL:

```sh
python3 -m http.server 4187 --bind 127.0.0.1 --directory frontend
```

1. Open the browser-only workspace. Confirm **Demo · synthetic data**.
2. Choose an investigation shortcut, then use the command launcher to find another open tool. Open commands with **Cmd+K** on macOS or **Ctrl+K** on Windows/Linux; Escape returns focus.
3. Open **Appearance** and try Studio Original, Precision, Paper and Night Shift. Paper is the default; reload to confirm a chosen style returns.
4. Try **Compact view** and the existing theme button. The color theme remains independent from the style.
5. Choose **Classic view**, then **Try Studio**. The workspace, selected Studio style and current context remain in place.

The command launcher navigates this workspace; it is not global AWS resource search. The local preview is for reviewing the design, not installation acceptance.

## Next gate

P6 and P7 remain unstarted. The selected application source is `5d39eed`; regenerate P4 candidates from that revision before device acceptance. The earlier inspected macOS artifacts contain older frontend bytes and do not validate this appearance set. Native Windows/Ubuntu builds, actual WebView rendering, installation/restart/upgrade/uninstall and live-provider behavior remain pending.

No push, remote Git/GitHub operation, AWS connection, real AWS CLI invocation, credential inspection, native launch/install, tag, release, signing or publication was performed in P5.
