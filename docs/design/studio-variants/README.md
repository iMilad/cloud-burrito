# Cloud Burrito — close design variations

Created on 2026-09-04 in response to the owner's request for close refinements of the selected folded-cloud identity. Open [the interactive comparison](index.html): four direction cards update the same synthetic workspace and show the chosen icon at 16, 24, 40, 64, and 150px. Light/dark controls change the study's appearance. The owner kept all four directions and selected **Paper** as the application default.

| Direction | What changes |
| --- | --- |
| 01 — Studio Original | Preserved first folded-cloud production icon and warm Studio palette. |
| 02 — Precision | Cleaner fold, slightly squarer tile, flatter surfaces, restrained orange and aligned shortcuts. Recommended refinement. |
| 03 — Paper | Softer tile/cloud, terracotta fold, olive ink, warmer surfaces and more breathing room. **Default.** |
| 04 — Night Shift | Smaller angular fold, neutral ink/chalk palette, compact cards and technical labels. Try dark mode. |

All four directions are implemented as Studio workspace styles in application source `5d39eed`. Use **Appearance** in the Studio rail, or the compact top-bar control, to switch among them. Style, light/dark theme and Classic/Studio view are independent. The style is stored locally under the allowlisted `cb.studio.appearance.v1` preference; missing or invalid values resolve safely to Paper without rewriting storage during load.

This comparison page remains a simplified component mockup with identical static synthetic content. It does not alter application preferences, credentials or native state. Its navigation and resource rows are visual examples; only the board's direction and light/dark controls are interactive. There are no remote fonts, dependencies or image requests.

The four variants retain the same cloud/fold family. Their names are internal style labels, not new product names. Paper is also the canonical packaged icon for Windows, Linux and macOS; changing the runtime style changes only in-app marks and the favicon. The variants have not undergone a new similarity or trademark review; see the [original bounded review](../logo-explorations/selection.md).

Local preview:

```sh
python3 -m http.server 4190 --bind 127.0.0.1 --directory docs/design/studio-variants
```

Keyboard: Tab to the direction or appearance buttons and press Enter/Space. Selection state is exposed through `aria-pressed`; the summary announces the chosen direction. Hover motion is disabled when reduced motion is requested. The board does not persist a choice or alter the application's preferences.

Board validation covers all eight direction/theme combinations, keyboard selection, loaded local SVGs and compact layout. Application validation covers all four styles in both color themes at 1480px and 1024px, a 500px chooser layout, reload persistence, URL allowlisting, early mark synchronization, Classic independence, modal focus and reduced motion. The complete synthetic frontend run reported **139 cases passed with zero failures**; its known teardown hang required stopping the finished local runner. The focused appearance/accessibility run exited normally with **13 passed**. These checks do not establish native WebView or device behavior.

For the build handoff, see [the packaging entry point](../../packaging/README.md). Both macOS target preflights pass on `5d39eed`; completed artifact generation and real-device acceptance remain separate stages.
