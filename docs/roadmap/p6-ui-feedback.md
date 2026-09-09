# P6: initial UI feedback

Local repairs for the six observations reported on 2026-09-09. These changes
remain part of the 0.3.0 test candidate; package receipts identify the exact
source commit. Earlier 0.3.0 packages do not contain these repairs.

| Observation | Result |
| --- | --- |
| Classic shows the old logo | Classic uses the selected Cloud Fold mark, including saved appearance during startup. |
| Connection details occupy the workspace | Close the section or use **Connection** in the topbar. The choice survives polls and restart. A new failure opens recovery once; repeated polls respect dismissal. Opening from fullscreen reveals the section. |
| The auth countdown looks like elapsed login time | The label explicitly describes remaining temporary AWS credential lifetime. Identity details show the expiration and last account verification separately. Unknown expiration stays unknown. |
| Returning to Studio is inaccessible | **Studio view** is in the Classic topbar and works during fullscreen and pending native settings loading. An explicit choice survives reload. |
| “Limited result” is unclear | A short explanation remains visible. For ten returned pipeline runs with a continuation token: “Showing 10 results. More were not loaded.” No total or older history is inferred. |
| Technical boxes consume too much space | Result details start collapsed. **Settings → Display preferences** provides **Show tips** and **Expand result details**, applied immediately on this device. Storage failure is reported as session-only. |

## Credential timing

The countdown is computed from the backend's credential expiration, not from
the time the app opened or the original SSO login. Reconnect rediscovers profiles
and verifies the selected account. That can obtain fresh temporary role
credentials using an existing SSO session, which can explain a renewed twelve
hours. The app does not track the original SSO login timestamp. This behavior was
checked against the credential provider code and synthetic responses; no live
AWS verification was performed.

## Compact feedback preserves meaning

The status, original receipt time, cache indication, limited-result explanation,
errors, stale-result warnings and remote-cleanup uncertainty remain visible.
Profile/account/region stay in each widget's context; repeated context and
technical coverage counts, limits and reasons are available in **Result details**.
Opening a disclosure survives refresh, including keyboard focus when a response
arrives. Background results do not take focus from Settings.

Optional tips are hidden by default. Operation permissions, query cost,
retention and recovery guidance are not treated as optional tips. Display
preferences are independent of the AWS settings Save/Cancel transaction.

## Verification and next test

Regression coverage includes synthetic credential countdowns (12h → 10h and
37m → 32m after reconnect), unknown expiry, connection dismissal and recovery,
Classic/Studio navigation, disclosure focus, preference persistence and storage
failure. Browser fixtures block external requests and contain synthetic identities.

Use the newly built candidate's source receipt when testing these changes on a
device. Installation and native WebView behavior remain owner tests. No AWS
connection, credential-source inspection, native launch, push, tag or publication
was part of this repair pass.
