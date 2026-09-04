# P5 — Give Cloud Burrito a distinctive creative interface

**Implementing locally.** Approved on 2026-09-04. This phase introduces the Studio design while keeping the existing Classic experience available. The original device-validation phase moves to [P6](phase-6.md), and the portfolio phase moves to [P7](phase-7.md). Existing P1–P4 commits, results and artifact identities remain historical evidence.

## Design direction

Make the investigation workspace inviting and memorable: confident typography, a strong color identity, clear visual hierarchy, purposeful depth and responsive interactions. The result should help an engineer choose a next action, recognize the active context and keep evidence together. A visually striking screen still has to work with an empty workspace, long names, partial failures, large results and a keyboard.

Studio is a frontend presentation of the existing product. Keep Rust + Tauri, the verified active/pinned account model, approved capabilities, bounded work and honest result states. Visible controls must lead somewhere useful; decorative metrics, invented live activity and fake cloud status are not product evidence. Demonstration data must remain explicitly synthetic and separated from connected work.

## Ordered work

| Unit | Work | Exit condition |
| --- | --- | --- |
| P5-01 — Preserve the baseline | Record pre-design source `0a860cd`; keep a local `mk/p4-design-baseline` reference; insert the new roadmap phase and retain Classic through an in-app design choice | A recoverable source baseline is named. Classic can be selected without losing the workspace or changing account/region context |
| P5-02 — Introduce the visual system | Implement Studio with an intentional layout, typography, color, spacing, surfaces and distinct interaction states | The primary screen and critical overlays share one coherent system; existing dark/light modes, readable hierarchy and honest error states remain usable |
| P5-03 — Make interaction meaningful | Connect visual entry points to real workspace, widget, context and investigation actions; add measured transitions and useful feedback | Actions affect the actual workspace, preserve existing ownership contracts and explain empty/pending/failed outcomes; no dead decorative controls or invented cloud results |
| P5-04 — Validate and hand off | Review the actual rendered UI, exercise both designs with a synthetic bridge and run applicable regression gates | Record viewport, keyboard, reduced-motion, theme and layout checks; preserve functionality and note untested native behavior; identify the final source for fresh P4 packages |

Commit local work by the P5 unit IDs. More than one focused commit is acceptable where a unit needs a follow-up; no previous phase history needs rewriting.

## Preservation and switching

The safety reference preserves the entire pre-design source. The Classic choice provides an everyday visual fallback inside the current application, while Studio carries the new presentation. Switching designs must not clear saved layouts, disconnect a context, repin a widget or replace durable settings. A later full source rollback is a separate deliberate Git action, not a destructive command embedded in this plan.

Keep the default, persistence mechanism and observed reload behavior in the final P5 evidence. Reusing a browser preference for presentation does not make it an AWS configuration setting. A failed preference write must leave the current UI usable.

## Local validation

- Inspect the rendered Studio and Classic screens at representative desktop widths, including a compact window and enlarged text. Check clipping, long labels, empty workspaces, dense results and critical dialogs.
- Exercise the design switch, navigation, workspace/widget actions, settings and the existing synthetic investigation route. Verify that focus can reach and leave every critical control and overlay.
- Honor reduced motion. Avoid continuously running decorative animation, network fonts or remote design assets; startup must not depend on external design services.
- Keep profile/account/region and result context visible. Preserve warnings, retry, cancellation, partial-result and stale-result semantics from P1–P3.
- Run the existing source/browser gates relevant to the change, recording actual results. A screenshot is visual evidence, not a functional test; synthetic browser success is not native WebView or real AWS acceptance.

## Exit and next step

P5 closes locally when the preserved baseline, complete Studio interface, working Classic fallback and recorded local checks exist. Any inaccessible critical workflow, lost state, context confusion or false success keeps its affected unit open.

Select the final reviewed P5 source, then use the P4 build and inspection workflow to produce a fresh matching candidate set. The macOS artifacts recorded in [P4 evidence](p4-exit-evidence.md) precede this redesign; their checks cannot validate new frontend bytes. Native Windows/Ubuntu build gates remain open. Only then continue to the [P6 handoff](../packaging/p6-handoff.md) and real-device acceptance.

This local design task includes no AWS connection, real AWS CLI invocation, credential inspection, native installation/app launch, remote Git/GitHub action, push, tag, release or publication.
