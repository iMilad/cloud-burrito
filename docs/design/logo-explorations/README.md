# Cloud Burrito logo exploration — round one

Created on 2026-09-04 with the built-in `image_gen` tool. The user selected **01 — Cloud Fold** after reviewing these concepts. Its refined vector mark is now applied to Studio, the favicon, the README icon, and the existing native icon exports. The product name remains **Cloud Burrito**; Cloud Fold is an internal concept label.

Open [the review board](index.html) to compare and enlarge the concepts:

1. [Cloud Fold](01-cloud-fold.png): a compact cloud with a folded corner. **Selected direction.**
2. [Wrapped Signal](02-wrapped-signal.png): an abstract wrapping monogram.
3. [Little Burrito](03-little-burrito.png): a friendly character with potential as either a playful identity or companion mascot.

The PNGs have transparent backgrounds. The board displays them on warm ivory; a dark image viewer can conceal their charcoal shapes and lettering. The board does not alter the image files.

[Exact generation prompts](prompts.json) and the original PNGs are retained for provenance. The production SVG is a deliberately simplified, manually authored interpretation of the selected concept. The existing live text wordmark is retained rather than using raster lettering.

See [the selection, similarity review, and asset guide](selection.md) for evidence, limitations, reproduction commands, and rollback. Classic view retains its previous header mark. Review-board checks cover loaded images, readable contrast, click-to-enlarge and keyboard dismissal; no AWS or remote Git operation is involved.

Local preview:

```sh
python3 -m http.server 4188 --bind 127.0.0.1 --directory docs/design/logo-explorations
```
