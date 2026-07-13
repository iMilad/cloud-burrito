# Cloud Burrito agent instructions

## Release follow-up

- After every successful push to `origin/main`, wait for the `CI` workflow to
  finish. When it is green, run `python3 scripts/release-status.py` and report
  the result to the user.
- Never create or push a release tag, start a release, or publish a draft
  automatically. When the status is `release pending`, suggest creating the
  release and ask once for approval. If the user already approved the release
  in the current task, do not ask again.
- One approval covers the complete unsigned draft-release sequence: run the
  repository release and security gates; confirm clean, synchronized `main`;
  create the immutable lightweight `app-vX.Y.Z` tag at the green commit; push
  the tag; monitor the `Release` workflow; address in-scope failures; and
  verify the draft contains both architecture-specific unsigned DMGs, app
  ZIPs, and valid SHA-256 checksums.
- Releases must remain explicitly unsigned and identity-free. Never add Apple
  Developer ID signing, notarization, stapling, publisher certificates,
  account or team identifiers, or personal publisher metadata.
- Stop at a verified draft GitHub Release. Publishing the draft publicly
  requires a separate explicit request.
- If the expected version tag already points to the current commit, do not
  recreate or move it. Inspect the `Release` workflow and GitHub draft. Report
  the release as complete only after verifying the draft and required
  artifacts; if they are missing, in progress, or failed, resume monitoring or
  offer to rerun from that same immutable tag.
- If the expected version tag exists at a different commit, never move,
  replace, or delete it. Report `version bump required`; update the version and
  changelog only with user approval, then release from a new tag after CI is
  green.
