# Security policy

## Supported versions

Security fixes are made on the default branch and released in the newest
published version. Older releases are not maintained separately.

## Reporting a vulnerability

Please do not open a public issue for a suspected vulnerability. Open the
repository's **Security** tab and use its private vulnerability-reporting form.
If no private form or other project-provided private channel is available, do
not post sensitive details publicly.

Include the affected Cloud Burrito version, macOS version and architecture,
reproduction steps, security impact, and any suggested mitigation. We aim to
acknowledge complete reports within three business days.

Never include AWS credentials, SSO tokens, account IDs, full ARNs, private
resource names, customer information, or other confidential data. Replace them
with clearly synthetic values before attaching logs or screenshots.

Cloud Burrito's in-app read-only guard is a safety layer, not a replacement for
least-privilege IAM. A report that bypasses that guard or reaches an unregistered
AWS operation is considered security-sensitive.
