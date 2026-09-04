# Selected identity: the folded cloud

Selected and applied on 2026-09-04. Public product name: **Cloud Burrito**.

The user chose concept 01 and authorized application after a preliminary similarity check. The production mark combines a rounded cloud, a diagonal opening and an orange folded corner. It uses the Studio palette: charcoal `#141612`, ivory `#e9eddf`, and orange `#ff8657`. The existing text wordmark remains live text.

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
| [`cloud-burrito-icon.svg`](../../../frontend/assets/cloud-burrito-icon.svg) | Canonical square icon source: ivory cloud and orange fold on a charcoal rounded tile. Used by Studio, favicon, and README. |
| [`cloud-burrito-icon.png`](../../../frontend/assets/cloud-burrito-icon.png) | 1024px transparent-edge raster companion. |
| [`cloud-burrito-mark.svg`](../../../frontend/assets/cloud-burrito-mark.svg) | Standalone charcoal/orange symbol for light backgrounds. |
| [`cloud-burrito-classic-icon.svg`](../../../frontend/assets/cloud-burrito-classic-icon.svg) | Preserved original mark for Classic view. |
| [`src-tauri/icons`](../../../src-tauri/icons) | Existing PNG, ICO, and ICNS exports regenerated from the canonical square source. Existing Android/iOS exports are retained as assets; no mobile target is enabled. |

Keep the icon square and preserve the gap between the cloud and folded corner. Do not add a second colored background or padding around the SVG in the navigation rail. Use the charcoal tile on dark surfaces; the standalone charcoal symbol is intended for light surfaces.

Generate native exports with the repository's installed Tauri CLI (2.11.4 at application time):

```sh
cargo tauri icon frontend/assets/cloud-burrito-icon.svg \
  --output /tmp/cloud-burrito-cloud-fold-icons
```

Copy only existing tracked paths so the generator cannot silently add platform assets or configuration:

```python
from pathlib import Path
import shutil
import subprocess

generated = Path("/tmp/cloud-burrito-cloud-fold-icons")
for name in subprocess.check_output(
    ["git", "ls-files", "src-tauri/icons"], text=True
).splitlines():
    relative = Path(name).relative_to("src-tauri/icons")
    shutil.copyfile(generated / relative, name)
```

The frontend raster companion can be reproduced separately:

```sh
cargo tauri icon frontend/assets/cloud-burrito-icon.svg \
  --output /tmp/cloud-burrito-cloud-fold-raster --png 1024
cp /tmp/cloud-burrito-cloud-fold-raster/1024x1024.png \
  frontend/assets/cloud-burrito-icon.png
```

## Validation and recovery

- All 48 existing native PNGs preserve their previous dimensions, bit depth, color mode, and alpha mode. Both Android XML files remain byte-identical.
- ICO decoding confirms 16, 24, 32, 48, 64, and 256px frames. ICNS decoding with `iconutil` confirms ten standard slots covering 16–1024px; `sips` confirms alpha.
- Browser review confirmed the updated Studio rail, compact header, light/dark themes, and the preserved Classic header. The generated 16px and 32px icons were also visually checked. The existing browser-mode and Studio tests passed: **15 tests**, using synthetic browser data.
- No native app was built, launched, or installed for this asset change. Real OS launcher/Dock appearance remains a device acceptance check.
- The original concept artwork and prompts remain in this directory. The full previous frontend/native icon set is recoverable from commit `cf4d11b`. Reverting the logo application commit restores the prior integration as well; Classic view already retains its old header icon.
