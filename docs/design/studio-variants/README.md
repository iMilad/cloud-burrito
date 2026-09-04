# Cloud Burrito — close design variations

Created on 2026-09-04 in response to the owner's request for a few refinements before making a final choice. Open [the interactive comparison](index.html): four direction cards update the same synthetic workspace and show the chosen icon at 16, 24, 40, 64, and 150px. Light/dark controls change the study's appearance.

| Direction | What changes |
| --- | --- |
| 01 — Studio Original | Exact current production icon; the applied warm Studio palette is the comparison reference. |
| 02 — Precision | Cleaner fold, slightly squarer tile, flatter surfaces, restrained orange and aligned shortcuts. Recommended refinement. |
| 03 — Paper | Softer tile/cloud, terracotta fold, olive ink, warmer surfaces and more breathing room. |
| 04 — Night Shift | Smaller angular fold, neutral ink/chalk palette, compact cards and technical labels. Try dark mode. |

These are presentation studies, not four independently implemented app themes. The workspace is a simplified component mockup with identical static synthetic content in each variation; its navigation and resource rows are visual examples. Only the direction and appearance controls are interactive. No application state, credentials, native bridge, or network service is used. There are no remote fonts, dependencies, or image requests.

The approved app at `f006269` remains unchanged: Studio and the original folded-cloud identity stay active, and Classic remains available. No new direction is selected for production by opening or clicking this board. The owner can select a direction by number/name, after which that exact choice can be integrated and the platform icons regenerated before candidate builds.

The current SVG is copied byte-for-byte from the production asset. The three variants are local vector edits of that existing identity; they retain the same cloud/fold family. Their names are internal study labels, not new product names. They have not undergone a new similarity or trademark review; see the [original bounded review](../logo-explorations/selection.md).

Local preview:

```sh
python3 -m http.server 4190 --bind 127.0.0.1 --directory docs/design/studio-variants
```

Keyboard: Tab to the direction or appearance buttons and press Enter/Space. Selection state is exposed through `aria-pressed`; the summary announces the chosen direction. Hover motion is disabled when reduced motion is requested. The board does not persist a choice or alter the application's preferences.

Validation: all eight direction/theme combinations loaded their SVGs with one selected direction and no horizontal overflow at 1480px. Actual screenshots were reviewed for the card overview, Precision light, Paper light, Night Shift dark, and the 460px compact layout. Keyboard Enter/Space selection worked; the narrow icon strip remained inside the viewport. JavaScript syntax and local asset/link checks passed. These checks cover the study, not native app behavior.

For the separate build question, see [the packaging entry point](../../packaging/README.md). Build readiness, completed artifact generation, and real-device acceptance are separate stages.
