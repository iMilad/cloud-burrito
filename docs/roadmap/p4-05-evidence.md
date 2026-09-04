# P4-05 — Ubuntu package policy

Implemented locally on 2026-09-04. Native Linux builds and package inspection
remain pending; no claim of compatibility with all Linux distributions is made.

The Linux overlay emits Debian and AppImage artifacts from the matrix's Ubuntu
22.04 x64 baseline. Runtime dependency generation stays with the pinned bundler;
no custom install hooks, background services, repositories, media payload, or
root-running application mode were added. The application retains its existing
identity and user-owned data directory.

Validation: 6 Linux policy fixtures passed and the actual overlay passes the
static schema/project policy check. The shared helper-cache fixtures cover
missing, unreviewed, changed, redirected and incomplete helper inputs. They are
synthetic evidence, not a real Linux build or helper-provisioning record.

The available local container engine has no dedicated reviewed Ubuntu build
image/helper set. No unrelated image was used, no image was pulled, and no
container was started. Native `.deb`/AppImage acceptance remains open.

See [Linux policy and runtime prerequisites](../packaging/linux.md).
