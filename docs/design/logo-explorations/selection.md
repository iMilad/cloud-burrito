# Selected identity: the folded-cloud family

Selected and applied on 2026-09-04. Public product name: **Cloud Burrito**.

The user chose concept 01 and authorized application after a preliminary similarity check. Its production family combines a rounded cloud, a diagonal opening and a folded corner. Four related Studio appearances now express that family: Studio Original, Precision, Paper and Night Shift. **Paper** is the fresh-install default and canonical packaged icon, using dark olive `#30392c`, warm ivory `#f3eee4` and terracotta `#c87852`. The existing text wordmark remains live text.

## Preliminary similarity review

This was a bounded web and visual review, not a reverse-image search, trademark registry search, or legal clearance. No obvious near-identical mark was found among the examples actually inspected. Cloud silhouettes and folded corners are common motifs, so this does not establish worldwide uniqueness or permission to use every possible similar design.

Queries included `cloud fold logo`, `cloud folded corner logo`, `orange cloud folded corner logo`, `CloudFold logo`, `"Cloud Burrito" logo`, and `"Cloud Burrito" software app`. Search results were used to identify candidates; only loaded artwork was treated as visual evidence.

| Source | Observation | Effect on this choice |
| --- | --- | --- |
| [Cloudflare official logo](https://www.cloudflare.com/nl-nl/logo/) | Shares the cloud motif and orange association. Its low, extended silhouette and horizontal division differ from our taller cloud and inward diagonal opening. | Broad family resemblance, no obvious near-identical geometry in the inspected artwork. |
| [Cloudsmith official brand page](https://cloudsmith.com/company/brand) | Angular diamond/ring construction with a circular interior. | Visibly different silhouette and negative space. |
| [Cloudsoft official identity article](https://cloudsoft.io/blog/cloudsoft-new-website-and-brand-identity) | Thin circular glyph used with its wordmark. | Visibly different construction. |
| [CloudFold AI](https://cloudfold.io/) | An existing product uses the name CloudFold. | Keep Cloud Fold as an internal concept label; do not rename Cloud Burrito to CloudFold. |
| [GraphicLoads “Cloud folded” listing](https://icon-icons.com/icon/cloud-folded/30478) | A relevant stock-icon title surfaced, but the artwork could not be inspected because the site blocked loading. | Unresolved visual comparison; not counted as cleared. |

Exact-name searches also surfaced unrelated food and music uses of “Cloud Burrito.” A third-party [domain listing](https://www.ipaddress.com/website/cloudburrito.com/) had inconsistent claims about `cloudburrito.com`; the domain could not be inspected directly. Neither domain availability nor brand-name clearance is established by this review. No project image was uploaded to a third-party search service.

## Production assets

| Asset | Purpose |
| --- | --- |
| [`cloud-burrito-icon.svg`](../../../frontend/assets/cloud-burrito-icon.svg) | Canonical Paper square icon source: warm ivory cloud and terracotta fold on a dark olive rounded tile. Used for the README and packaged operating-system icons. |
| [`cloud-burrito-icon.png`](../../../frontend/assets/cloud-burrito-icon.png) | 1024px transparent-edge raster companion. |
| [`cloud-burrito-mark.svg`](../../../frontend/assets/cloud-burrito-mark.svg) | Standalone dark-olive and terracotta Paper symbol for light backgrounds. |
| [`cloud-burrito-style-current.svg`](../../../frontend/assets/cloud-burrito-style-current.svg), [`cloud-burrito-style-precision.svg`](../../../frontend/assets/cloud-burrito-style-precision.svg), [`cloud-burrito-style-paper.svg`](../../../frontend/assets/cloud-burrito-style-paper.svg), [`cloud-burrito-style-night.svg`](../../../frontend/assets/cloud-burrito-style-night.svg) | Runtime marks for the four selectable Studio appearances. They change in-app marks and the favicon, not the packaged launcher icon. |
| [`cloud-burrito-classic-icon.svg`](../../../frontend/assets/cloud-burrito-classic-icon.svg) | Preserved original mark for Classic view. |
| [`src-tauri/icons`](../../../src-tauri/icons) | Existing PNG, ICO, and ICNS exports regenerated from the canonical square source. Existing Android/iOS exports are retained as assets; no mobile target is enabled. |

Keep the icon square and preserve the gap between the cloud and folded corner. Do not add a second colored background or padding around the SVG in the navigation rail. The runtime marks may follow their selected appearance, while Windows, Linux and macOS launcher or installer assets remain Paper.

Generate native exports with the repository's installed Tauri CLI (2.11.4 at application time):

```sh
cargo tauri icon frontend/assets/cloud-burrito-icon.svg \
  --output /tmp/cloud-burrito-paper-icons
```

Copy only existing tracked paths so the generator cannot silently add platform assets or configuration:

```python
from pathlib import Path
import shutil
import subprocess

generated = Path("/tmp/cloud-burrito-paper-icons")
for name in subprocess.check_output(
    ["git", "ls-files", "src-tauri/icons"], text=True
).splitlines():
    relative = Path(name).relative_to("src-tauri/icons")
    shutil.copyfile(generated / relative, name)
```

The frontend raster companion can be reproduced separately:

```sh
cargo tauri icon frontend/assets/cloud-burrito-icon.svg \
  --output /tmp/cloud-burrito-paper-raster --png 1024
cp /tmp/cloud-burrito-paper-raster/1024x1024.png \
  frontend/assets/cloud-burrito-icon.png
```

## Validation and recovery

- All 48 existing native PNGs preserve their previous dimensions, bit depth, color mode, and alpha mode. Both Android XML files remain byte-identical.
- ICO decoding confirms 16, 24, 32, 48, 64, and 256px frames. ICNS decoding with `iconutil` confirms ten standard slots covering 16–1024px; `sips` confirms alpha.
- Browser review covered all four styles in light and dark mode, compact layout, the Appearance chooser and the preserved Classic header. The focused Appearance/accessibility gate exited normally with **13 passed**. The complete synthetic frontend run reported all **139 cases passed with zero failures**; its documented teardown hang required stopping the finished runner.
- No native app was built, launched, or installed for this asset change. Both macOS input preflights pass on application source `5d39eed`, while Windows and Ubuntu still require their native build hosts. Real launcher, installer, Start menu, taskbar and Dock appearance remain device acceptance checks.
- The original concept artwork and prompts remain in this directory. The concept board is retained in `cf4d11b`, the first applied folded-cloud identity in `f006269`, and the source immediately before the four-style application in `774939a`. Classic view retains its old header icon.
