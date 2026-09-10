# P6: initial UI feedback

Local repairs for the six observations reported on 2026-09-09. These changes
remain part of the 0.3.0 test candidate; package receipts identify the exact
source commit. Earlier 0.3.0 packages do not contain these repairs.

| Observation | Result |
| --- | --- |
| Classic shows the old logo | Classic uses the selected Cloud Fold mark, including saved appearance during startup. |
| Connection details occupy the workspace | Close the section or use **Connection** in the topbar. The choice survives polls and restart. A new failure opens recovery once; repeated polls respect dismissal. Opening from fullscreen reveals the section. |
| Retry connection appears to reset the login countdown | The topbar shows the selected cached SSO access token's remaining lifetime. It is reread on status checks and does not reset when new account credentials are obtained. Identity details show both expirations separately. Unknown token expiration stays unknown. |
| Returning to Studio is inaccessible | **Studio view** is in the Classic topbar and works during fullscreen and pending native settings loading. An explicit choice survives reload. |
| “Limited result” is unclear | A short explanation remains visible. For ten returned pipeline runs with a continuation token: “Showing 10 results. More were not loaded.” No total or older history is inferred. |
| Technical boxes consume too much space | Result details start collapsed. **Settings → Display preferences** provides **Show tips** and **Expand result details**, applied immediately on this device. Storage failure is reported as session-only. |

## SSO token timing

The topbar countdown uses the selected SSO cache's actual access-token expiry.
Reading that metadata does not invoke a token provider or renew the token.
A new account-credential expiry or a new connection attempt does not
reset this timer. The account-credential expiry remains available in Identity
details as a separate value.

Retry connection rediscovers profiles and verifies the selected account using
the existing SSO session. The CLI or SDK can genuinely renew a supported SSO
access token; when that happens the next status check shows its new actual
expiry. The app does not invent a login timestamp or infer the overall portal
sign-in session duration from an access-token or account-credential lifetime.

Unavailable, malformed or mismatched cache metadata produces an unknown token
expiry. An expired SSO access token can coexist with still-valid account
credentials; it does not by itself invalidate the account context. These paths
are checked using synthetic fixtures, without inspecting the owner's credentials
or connecting to AWS.

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

## Widget presentation follow-up

- Studio now starts with **Compact view** enabled and a compact overview banner.
  **Expand overview** restores the full introduction; **Hide** removes the banner.
  **Show overview** in the workspace toolbar or **Appearance → Overview banner**
  brings it back. Banner size and widget density are independent local choices;
  an explicit opt-out survives restart. The browser demo explains that it uses
  sample data without an AWS connection. Hiding the banner keeps a small mode
  indicator in the toolbar; native identity state remains separate and unchanged.
- **Configure widget → Header color** offers the existing soft tint, an accent
  line and a soft gradient. The miniature preview changes immediately; **Save**
  applies it and **Cancel** discards the draft. Pink and purple remain distinct.
  Appearance-only saves preserve loaded results, unfinished input and in-flight
  requests. Changing the widget's account or saved execution inputs still
  invalidates the old context as before.
- **Collapse widget** in the header hides the entire body and releases its grid
  space. **Expand widget** restores the previous height. Color, treatment and
  collapsed state survive dashboard reload. Fullscreen expands a collapsed
  widget; collapsing a fullscreen widget returns it to the grid. Hidden bodies
  are excluded from keyboard navigation. Collapsing is a display choice, not a
  pause or cancellation of background work.
- **Appearance → Contrast** adjusts neutral text and borders from softer
  (**−20**) to stronger (**+50**). **Original (0)** and **Reset contrast** restore
  the source palette exactly. The choice is local and works with all four
  Studio appearances, both color themes and Classic. A storage failure is
  reported as a session-only choice. Backgrounds, accent and status colors keep
  the selected design. This control does not claim an accessibility certification.

Synthetic regressions exercise save/cancel/reload, preserved live data and
draft input, collapse geometry and focus, fullscreen, contrast reset and
palette switching. Backend dashboard tests cover native round-trip storage and
reject unrecognized treatments, invalid collapse flags and out-of-range heights.

## Verification and next test

Regression coverage includes independent token and account-credential lifetimes,
retry and restart without timer reset, actual token renewal, unknown and expired
tokens, connection dismissal and recovery,
Classic/Studio navigation, disclosure focus, preference persistence and storage
failure. Browser fixtures block external requests and contain synthetic identities.

Use the newly built candidate's source receipt when testing these changes on a
device. Installation and native WebView behavior remain owner tests. No AWS
connection, credential-source inspection, native launch, push, tag or publication
was part of this repair pass.
