# Cloud Burrito logo exploration — round one

Created on 2026-09-04 with the built-in `image_gen` tool. These are concept previews for collaborative selection; no new logo is selected or applied to the application.

Open [the review board](index.html) to compare and enlarge the concepts:

1. [Cloud Fold](01-cloud-fold.png): a compact cloud with a folded corner. Recommended first direction for a simple primary symbol.
2. [Wrapped Signal](02-wrapped-signal.png): an abstract wrapping monogram.
3. [Little Burrito](03-little-burrito.png): a friendly character with potential as either a playful identity or companion mascot.

The PNGs have transparent backgrounds. The board displays them on warm ivory; a dark image viewer can conceal their charcoal shapes and lettering. The board does not alter the image files.

[Exact generation prompts](prompts.json) are retained for iteration. After choosing a direction, refine its vector outline, wordmark and monochrome treatment, then verify actual small-size rendering before producing application icon exports.

The existing app logo and Studio implementation remain in place. Review-board checks cover loaded images, readable contrast, click-to-enlarge and keyboard dismissal; no AWS or remote Git operation is involved.

Local preview:

```sh
python3 -m http.server 4188 --bind 127.0.0.1 --directory docs/design/logo-explorations
```
