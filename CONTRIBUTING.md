# Contributing to Cloud Burrito

Thank you for improving Cloud Burrito. Keep changes focused, explain the user
problem they solve, and open a pull request against the default branch.

## Development setup

Install the versions declared in `rust-toolchain.toml`, `package.json`, and the
README, including Node.js/npm, Python 3, and Xcode Command Line Tools, then run:

```bash
npm ci
npx playwright install chromium
./scripts/dev.sh
```

Before opening a pull request:

```bash
npm run test:frontend
./scripts/security-check.sh
```

The security check validates release metadata, source privacy, formatting,
tests, Clippy, script syntax, whitespace, and any supported security scanners
already installed locally.

## Safety and privacy requirements

- Preserve the read-only AWS invariant. New AWS operations must be registered,
  structurally classified as read-only, policy-gated, audited, and tested.
- Tests and CI must not make live AWS API calls or depend on AWS credentials.
- Never commit credentials, SSO tokens, account IDs, personal paths, customer
  names, private resource identifiers, or unsanitized production logs.
- Use synthetic examples and run `./scripts/security-check.sh` before pushing.
- Update tests, documentation, and `CHANGELOG.md` when behavior changes.

## Pull requests

Describe the motivation and implementation, link related issues, and list the
commands you ran. Include screenshots for visible UI changes. Prefer small,
reviewable pull requests over unrelated cleanup bundled with feature work.

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
