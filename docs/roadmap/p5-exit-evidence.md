# P5 — Studio design and Classic fallback

Date: 2026-09-04. Frontend implementation source: `d211748` on the local `mk/p5-studio-design` branch. The P5-04 documentation commit follows this source without changing application bytes. Original P5 device work is now P6; original P6 portfolio work is now P7.

## What changed

Studio is the default presentation: a graphite navigation rail, a warm light overview, orange investigation shortcuts, clearer widget hierarchy, coordinated light/dark surfaces and a keyboard command launcher. The overview describes a workflow; its arrows do not claim detected relationships between cloud resources. Counts reflect actual open widget instances, including duplicates. Browser data is explicitly labeled synthetic; desktop identity labeling follows the existing verified connection state.

Navigation focuses tools already in the workspace. Missing tools have disabled shortcuts; navigation never silently creates a widget or starts a resource request. Commands open actual widget instances or existing available panels. Settings and Audit remain unavailable in browser-only mode. Compact view changes presentation spacing, without rewriting GridStack geometry. Short highlights and hover transitions honor reduced motion; there are no new remote fonts, image services, dependencies or continuously running decorative animations.

The launcher uses the existing modal focus owner, scrim and Escape behavior. It releases its command closures on close, including references to removed widgets. Its observer watches direct grid children, not result rows. Existing result ownership, resource execution and backend code are unchanged.

## Preservation and local commits

| Unit | Local commit | Result |
| --- | --- | --- |
| P5-01 | `5faa0c3` | Insert P5, renumber later phases, preserve historical evidence and record the rollback source |
| P5-02 | `e87c8a3` | Add the isolated, responsive Studio stylesheet; activation follows in P5-03 |
| P5-03 | `d211748` | Activate the new shell, commands, navigation, density and reversible design choice; add interaction regressions |
| P5-04 | Documentation commit containing this evidence | Record final local checks and the next device/package gate |

The pre-design source is `0a860cd`, retained by `mk/p4-design-baseline`. The original `frontend/styles.css` is unchanged. **Classic view** restores the original visual system; **Try Studio** returns to the new one. Neither action resets the dashboard, repins a context or changes the current theme.

The allowlisted presentation choice is stored separately under `cb.presentation.v1`. Studio is the default when no valid preference exists. A blocked store still allows a session choice. The optional `?design=classic` or `?design=studio` URL overrides the stored default for that load without overwriting it. Remove the query to resume the stored choice. Compact mode is session-only. This is a presentation preference, not an AWS setting.

## Validation

- Final frontend suite: **133 passed**. Chromium uses browser mock data or the existing synthetic native bridge. No real provider or native application is involved.
- Node unit suite: **30 passed**.
- Python packaging/release helper suite: **113 passed**, including the renumbered target metadata and orchestration template.
- JavaScript syntax and whitespace checks pass. The scoped release privacy scan passed across **277 files** after staging all new source and evidence files; it is not a fresh dependency, full-history or live-environment security certification.
- The six Studio cases cover keyboard focus and modal exit, command filtering and empty results, actual and duplicate tool selection, removal and truthful counts, URL preference allowlisting, raw layout/selection storage preservation across design switches and reload, theme preservation, reduced-motion navigation and a 1024-pixel compact workspace. Closed command results are released and native-only controls remain hidden in browser mode.
- Existing accessibility cases exercise native-mode synthetic UI at 1280 and 1024 pixels, both themes, 200% zoom and doubled text; fullscreen/panel ownership and critical controls remain covered.

Harness note: after all 133 individual cases reported success, one Playwright worker remained alive during teardown with no browser child. Normal termination did not close it; only that verified child of this test run was force-stopped. The parent runner then reported **133 passed** and exited **0**. This was test-process cleanup after assertions, not a skipped case or an unattended-run reliability claim. The final log is `/tmp/cloud-burrito-p5-browser-final.log`.

A broad run exposed an intermittent existing picker race: a pending blur callback could erase a new query after the same input regained focus. The callback exists on the preserved P4 source. A deterministic clock-driven test failed before the fix; checking that the input is still unfocused before closing resolved it. That regression and the original CodeArtifact account-change case each passed three repeated runs. No account-selection assertion was relaxed.

Actual browser screenshots were reviewed at 1480 × 920 and at the app's approximately 954 × 1194 viewport. Studio dark, Studio light, Classic, the command launcher and the compact synthetic desktop screenshot were inspected. The default viewport was restored after responsive review. A review also corrected storage-warning placement beside the sidebar and ensured hidden native controls do not appear merely because a CSS display rule overrides their hidden attribute.

## Try the local design

Serve the frontend locally and open its URL:

```sh
python3 -m http.server 4187 --bind 127.0.0.1 --directory frontend
```

1. Open the browser-only workspace. Confirm **Demo · synthetic data**.
2. Choose an investigation shortcut, then use the command launcher to find another open tool. Open commands with **Cmd+K** on macOS or **Ctrl+K** on Windows/Linux; Escape returns focus.
3. Try **Compact view** and the existing theme button.
4. Choose **Classic view**, then **Try Studio**. The workspace and current context remain in place.

The command launcher navigates this workspace; it is not global AWS resource search. The local preview is for reviewing the design, not installation acceptance.

## Next gate

P6 and P7 remain unstarted. Review the new design, select the final P5 source, and regenerate P4 candidates from that revision before device acceptance. The earlier inspected macOS artifacts contain Classic-era frontend bytes and do not validate Studio. Native Windows/Ubuntu build evidence, actual WebView rendering, installation/restart/upgrade/uninstall and live-provider behavior remain pending.

No push, remote Git/GitHub operation, AWS connection, real AWS CLI invocation, credential inspection, native launch/install, tag, release, signing or publication was performed in P5.
