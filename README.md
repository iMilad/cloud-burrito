<p align="center">
  <img src="frontend/assets/cloud-burrito-icon.svg" alt="Cloud Burrito icon" width="104">
</p>

<h1 align="center">Cloud Burrito</h1>

<p align="center">
  A fast, <strong>read-only</strong> desktop dashboard for browsing AWS across multiple accounts — built in pure Rust.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square" alt="License: MIT">
  <img src="https://img.shields.io/badge/Rust-1.91.1%2B-orange.svg?style=flat-square&logo=rust" alt="Rust 1.91.1+">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB.svg?style=flat-square&logo=tauri" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Platform-macOS-lightgrey.svg?style=flat-square&logo=apple" alt="Platform: macOS">
  <img src="https://img.shields.io/badge/AWS-read--only-success.svg?style=flat-square&logo=amazonaws" alt="AWS read-only">
</p>

<p align="center">
  <img src="docs/screenshot.svg" alt="Cloud Burrito dashboard" width="820">
</p>

---

Cloud Burrito is a small native desktop app for engineers who want a quick,
**safe** window into their AWS estate — pipeline runs, CloudFormation stacks, log
tails, error aggregates — without the risk of fat-fingering a destructive action.

It is **read-only by construction**: the binary has a closed registry of AWS
operations, every request path passes a structural read-only guard, and
`policy.yaml` must explicitly allow the operation before the SDK call is issued.
Admin credentials do not give the app a generic "run any AWS call" escape hatch.

No Python, no Node, no sidecar process, no runtime dependencies — just one
compiled binary that talks to AWS in-process via the AWS SDK for Rust.

## Contents

- [Highlights](#highlights)
- [Security model](#security-model)
- [Widgets](#widgets)
- [Getting started](#getting-started)
- [Configuration](#configuration)
- [Architecture](#architecture)
- [Project layout](#project-layout)
- [Development](#development)
- [Release pipeline](#release-pipeline)
- [Contributing and security](#contributing-and-security)
- [License](#license)

## Highlights

- **Read-only, guaranteed.** Write operations aren't in the binary. Admin
  credentials grant the app no powers it doesn't have code for.
- **User-narrowable policy.** An IAM-style `policy.yaml` allowlist (edited from
  the Settings panel, with live YAML syntax highlighting) further restricts which
  read operations may run — and can only narrow, never widen.
- **Full audit trail.** Every allowed or blocked AWS call preflight is appended
  to a JSONL audit log and shown live in an in-app Audit panel, tagged `aws`,
  `aws-blocked`, or lifecycle.
- **Multi-account via AWS SSO.** Uses your existing `~/.aws/config` SSO profiles;
  search for a default account/region from the top bar, or pin account/region per widget.
- **Customizable dashboard.** Drag/resize widget tiles (GridStack); the layout
  and per-tile config persist across launches.
- **Single binary.** Pure Rust + a webview UI. No interpreter, no helper process.
- **Offline demo mode.** Open the frontend in a plain browser to explore the UI
  with mock data — no AWS account required.

## Security model

The read-only guarantee is enforced as a hierarchy. Every gate must pass before
the app issues an AWS SDK request; any failure stops the call locally:

```
  requested AWS operation
            │
            ▼
  1. Compiled registry
     Is this exact service:Operation built into the app?
            │ yes
            ▼
  2. Structural read-only guard
     Is the operation classified as read-only / non-mutating?
            │ yes
            ▼
  3. policy.yaml
     explicit Deny > matching Allow > default-deny
            │ yes
            ▼
  AWS SDK request is issued
```

1. **Compiled registry.** Only operations listed in `src-tauri/src/aws/policy.rs`
   can run. Wildcards in `policy.yaml` never add new capabilities; they only
   match this closed set.
2. **Structural read-only guard.** The operation name must pass the local
   read-only classifier. This blocks mutating verbs even if someone adds them to
   `policy.yaml` by mistake.
3. **`policy.yaml` allowlist.** A familiar IAM-statement-style file you control:

   ```yaml
   # ~/.cloud_burrito/policy.yaml
   statements:
     - effect: Allow
       action:
         - sso:GetRoleCredentials
         - sts:GetCallerIdentity
         - cloudformation:Describe*
         - cloudformation:List*
         - logs:*
         - codepipeline:Get*
         - codepipeline:List*
         - codeartifact:Describe*
         - codeartifact:List*
     - effect: Deny          # explicit Deny always wins
       action:
         - logs:StartQuery
   ```

   Globs (`*`, `?`) are supported; anything not matched by an `Allow` is denied.
   The file is **fail-closed** — invalid YAML denies everything rather than
   falling back to permissive — and is auto-seeded on first run with exactly the
   operations the app uses, so it works out of the box and you delete lines to
   scope down. Remove a service and the corresponding widget shows a clear
   "missing permission" tile instead of making the call.

SSO credential resolution is part of the hierarchy too. The AWS SDK can refresh
role credentials before a service call, so every service-call path also requires
`sso:GetRoleCredentials` to be allowed before the service request can proceed.
If you remove that action from `policy.yaml`, set-account and later widget
refreshes stop locally before a service call is issued.

Current compiled registry:

```text
sso:GetRoleCredentials
sts:GetCallerIdentity
cloudformation:ListStacks
cloudformation:DescribeStackResources
cloudformation:DescribeStackEvents
logs:DescribeLogGroups
logs:StartQuery
logs:GetQueryResults
logs:StopQuery
logs:FilterLogEvents
codepipeline:ListPipelines
codepipeline:ListPipelineExecutions
codepipeline:ListActionExecutions
codebuild:BatchGetBuilds
codeartifact:ListPackages
codeartifact:ListPackageVersions
codeartifact:DescribePackageVersion
lambda:ListFunctions
logs:GetLogEvents
logs:DescribeLogStreams
resourcegroupstaggingapi:GetResources
```

> Note: this is a client-side safety rail, not a substitute for least-privilege
> IAM. Always pair it with a read-only role.

### The AWS CLI Table widget and the registry

The `aws-cli` widget runs a user-supplied read-only `aws` command, which by
nature cannot be pre-registered. For that one path the compiled registry gate is
replaced by the structural read-only guard as the floor: the command's
`service:Operation` (derived from the CLI names, e.g. `ec2 describe-instances`
→ `ec2:DescribeInstances`) must pass the read-only classifier **and** be
allowed by `policy.yaml`. Because the auto-seeded policy only lists registry
operations, every CLI action outside the registry is **deny-by-default** — you
opt in per service by adding a line such as `- ec2:Describe*` in Settings. The
command is tokenized and spawned as an argument vector (never a shell),
`--profile`/`--output` are rejected, and the child runs under the tile's
account context via `AWS_PROFILE`/`AWS_REGION`. Every run and every denial is
audited like SDK calls. Like pipelines, commands can be pinned: each pin saves
the command together with the profile/account/region it was created under and
always reruns in that context, side by side on the widget's Pinned tab.

## Widgets

The dashboard widgets are compiled into the binary, with drill-down views for
stack details, pipeline execution details, and CodeBuild logs:

| Widget | What it shows |
| --- | --- |
| `cfn-stacks` | Searchable non-deleted CloudFormation stacks in the active region, with status and resource count |
| `log-tail` | Lambda function browser with ARN/update/log-group details, log streams, and events |
| `cloudwatch-logs` | Search CloudWatch log groups, browse their streams, and view events |
| `errors-by-stack` | CloudWatch errors grouped by stack over a selected time window, with in-widget filtering |
| `resource-lookup` | Reverse-lookup: find the CloudFormation stack that owns a resource |
| `pipeline-runs` | Recent CodePipeline executions with expandable detail, plus pinned pipelines across accounts and regions |
| `codeartifact-packages` | Latest package versions in CodeArtifact filtered by package prefix |
| `logs-insights` | Your own CloudWatch Logs Insights query against any log group, rendered as a table |
| `aws-cli` | A read-only `aws` CLI command run under the tile's account context, JSON output rendered as a table |

Widget payloads may use stable machine keys such as `latest_version`,
`last_published`, or `execution_id`, but the UI must not show those raw names as
labels. Subtitles, action labels, empty states, and permission messages should
also read like product copy rather than internal API fields. User-facing values
should be formatted for reading too: ISO timestamps render as compact local
times, status constants render as words, and booleans render as Yes/No. Exact
identifiers and names, such as execution IDs, ARNs, pipeline names, log groups,
package versions, and repository names, should stay unchanged. The frontend
formats display labels centrally: separators become spaces, words become title
case, and known cloud acronyms such as AWS, CFN, ID, ARN, SSO, URL, JSON, and
YAML stay uppercase. New widgets should return stable keys and let the UI naming
rule render readable labels and values.

## Getting started

### Prerequisites

- **macOS** with Xcode Command Line Tools (`xcode-select --install`)
- **Node.js** 22 with **npm** 10.9.8, plus **Python 3** for local test servers
- **Rust** 1.91.1 and Cargo (automatically selected by `rust-toolchain.toml`)
- **Tauri CLI** 2.11.4 — `cargo install tauri-cli --version 2.11.4 --locked`
- **AWS CLI v2** with at least one **SSO** profile configured in `~/.aws/config`
  (`aws configure sso`), and a valid session (`aws sso login`)
- A system webview (preinstalled on macOS)

### Run

```bash
git clone <repository-url>
cd cloud-burrito
./scripts/dev.sh          # runs `cargo tauri dev` from src-tauri/
```

### Build a release bundle

```bash
./scripts/build-release.sh
```

The release script selects the host Rust target by default, constrains Cargo to
the committed lockfile, removes local source paths from the binary, builds the
`.app` and `.dmg`, and privacy-scans the packaged application. Pass an explicit
target such as `aarch64-apple-darwin` or `x86_64-apple-darwin` when needed.
All bundles are intentionally produced with Tauri's `--no-sign` option. No
publisher certificate, account, or team identity is read by the build. macOS may
reject downloaded unsigned applications, so building locally from tagged source
is the recommended distribution path.

GitHub Actions prepares unsigned convenience release drafts from `app-vX.Y.Z`
tags; a maintainer reviews and publishes each draft. See
[Release pipeline](#release-pipeline).

## Configuration

Search for your default account and region in the top bar, and set your SSO
session in the Settings panel. Individual widgets can inherit the default or pin
their own account/region.
State lives in your home directory:

| Path | Purpose |
| --- | --- |
| `~/.aws/config` | Your AWS SSO profiles (standard AWS CLI config) |
| `~/.cloud_burrito/settings.json` | App settings (config path, SSO session, default profile/region) |
| `~/.cloud_burrito/policy.yaml` | Read-only allowlist (see [Security model](#security-model)) |
| `~/.cloud_burrito/dashboard.json` | Saved dashboard layout and per-tile config |
| `~/.cloud_burrito/audit.log` | Append-only JSONL log of AWS call preflights and blocked calls |

Authentication uses the standard SSO flow: `aws-config` resolves your profile's
cached SSO token into short-lived role credentials that the SDK auto-refreshes.
When the token expires, the app detects it and prompts you to re-run
`aws sso login`. The default region is `eu-west-1` (allowed regions are
`eu-west-1` and `us-east-1`, adjustable in `src-tauri/src/settings.rs`).

## Architecture

A [Tauri 2](https://tauri.app) application: a Rust backend that does all the work,
and an HTML/CSS/JS dashboard rendered in the OS webview.

- **`src-tauri/` (Rust)** — every AWS call (via `aws-sdk-*` + `aws-config`), the
  read-only policy and structural guard, the audit log, credential/SSO handling,
  and settings/dashboard persistence. The UI reaches it through `#[tauri::command]`
  handlers — there is no separate sidecar process or RPC bridge.
- **`frontend/` (vanilla JS, no build step)** — renders the dashboard, handles
  layout and the settings/policy editor, and calls the backend via `invoke(...)`.
  Opened directly in a browser, it falls back to mock data for an offline preview.

## Project layout

```
frontend/    Vanilla HTML/CSS/JS dashboard (open index.html for the offline demo)
src-tauri/   Tauri 2 (Rust) app — all AWS calls happen in-process
  src/aws/   Credentials/SSO context, config parsing, read-only guard + policy
  src/widgets/  The compiled-in widgets
scripts/     Development, release-build, version, privacy, and security checks
tests/       Browser-mode Playwright tests
info/        Design spec, architecture notes, status page
docs/        Project docs / specs / plans
```

## Development

```bash
npm ci
npx playwright install chromium
npm run test:frontend

cd src-tauri
cargo fmt --all --check
cargo test -p cloud-burrito --locked
cargo clippy -p cloud-burrito --locked -- -D warnings
```

The frontend has no production build step; edit `frontend/*.js`/`*.css` and
reload. Its browser-mode behavior is tested against Chromium with synthetic data.

### Local security check

```bash
./scripts/security-check.sh
```

Run this before publishing or tagging. It checks release/toolchain metadata,
source and untracked-file privacy, script and frontend syntax, Rust formatting,
Rust tests, Clippy, whitespace, and Cargo advisories when `cargo-audit` is installed.
Cargo Audit uses the existing local RustSec database during the gate; run
`cargo audit` separately when you intentionally want to refresh that data.
It also uses optional local scanners such as Gitleaks, Detect-Secrets, and
TruffleHog when available. The built-in privacy gate scans for AWS keys,
credential assignments, private keys, account IDs, absolute local user paths,
concrete project-owner references, personal CODEOWNERS/author fields, and hashed
denylist values for project-private markers. Findings are redacted.
Detect-Secrets and TruffleHog run without live secret verification, so the local
gate makes no AWS or other credential-validation calls.

## Release pipeline

Cloud Burrito releases are tag-driven. The tag must match both app manifests.

After every green `main` build, check the authoritative remote release tag:

```bash
python3 scripts/release-status.py
```

The read-only command reports one of these states and never creates, moves, or
deletes a tag:

| Result | Exit | Meaning |
| --- | ---: | --- |
| `release pending: app-vX.Y.Z` | `0` | The validated version has no remote release tag and may be proposed for release |
| `release tag exists: app-vX.Y.Z` | `10` | The immutable version tag points to `HEAD`; inspect the workflow and draft before reporting completion |
| `version bump required: app-vX.Y.Z already points to another commit` | `20` | New changes require a new version; never move the existing tag |

Use `--json` for machine-readable output. After the user approves a pending
release, validate and push the lightweight tag:

```bash
TAG="app-v$(python3 scripts/check-release-version.py --print-version)"
python3 scripts/check-release-version.py "$TAG"
git tag "$TAG"
git push origin "$TAG"
```

Pushing the tag starts the release automatically. To rerun it manually, dispatch
the workflow from that same immutable tag ref (a branch dispatch is rejected):

```bash
TAG="app-v$(python3 scripts/check-release-version.py --print-version)"
gh workflow run release.yml \
  --ref "$TAG" \
  -f "tag=$TAG"
```

The pipeline has two workflows:

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `CI` | PRs and pushes to `dev`/`main` | Runs browser tests and dependency audits, validates metadata/privacy/syntax/formatting, tests and lints Rust, and checks whitespace |
| `Release` | `app-v*` tags or a manual dispatch on that tag ref | Re-runs the gates, builds explicitly unsigned bundles for both macOS architectures, rejects publisher identities, validates and privacy-scans the app, verifies checksums, then creates a draft GitHub release |

Release privacy is a hard gate. `scripts/check-release-privacy.py` scans tracked
source files and the packaged app bundle for AWS access key IDs, credential
assignments, private keys, standalone 12-digit account IDs, local absolute user
paths, concrete project-owner references, and hashed denylist values for
project-private markers. The workflows also assert that AWS credential
environment variables are empty before any job runs.
All external Actions are pinned to immutable commit SHAs and updated by
Dependabot. Rust build caches reduce repeated compilation without containing
credentials.

The workflow always builds the immutable commit that triggered the tag run and
refuses to continue if the tag moves. A rerun may update an existing draft, but
it cannot overwrite assets on an already published release.

No AWS credentials, AWS API calls, Apple credentials, publisher certificates,
or publisher identities are used anywhere in CI or release. The build script
clears inherited Apple identity variables and passes `--no-sign`; the workflow
also rejects any bundle containing a certificate authority or publisher team.
Release filenames include `_unsigned` so their trust status is unambiguous.

For the repository itself, enable GitHub secret scanning with push protection
and private vulnerability reporting. Protect `main` and `dev` with required
`CI` checks, pull-request review, and force-push/deletion protection. These
GitHub settings are external to the files in this repository.

Draft release downloads can be checked independently:

```bash
shasum -a 256 -c SHA256SUMS.txt
```

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and pull-request
expectations, [CHANGELOG.md](CHANGELOG.md) for release notes, and
[SECURITY.md](SECURITY.md) for private vulnerability reporting. Participation is
governed by the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE) © Cloud Burrito contributors
