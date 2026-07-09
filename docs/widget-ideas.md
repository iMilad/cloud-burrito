# Widget ideas — making Cloud Burrito useful beyond one company

Date: 2026-07-06. Status: option 3 implemented on 2026-07-07 (`logs-insights` +
`aws-cli` widgets — see `docs/superpowers/specs/2026-07-07-byo-service-widgets-design.md`).

## The concern

The current widget set (Pipeline Runs, Lambda Logs, CFN Stacks, CodeArtifact,
Resource Lookup, Errors by Stack) mirrors one team's workflow. A company that
uses GitHub Actions + Terraform + ECS would only find part of it useful.

## First observation

Nothing in the app is hardcoded to one company. It is tied to a set of AWS
**services**, not to an employer. Every CDK / CodePipeline shop gets full value
today. The real question is service coverage, not "generic vs personal".

## Options

### 1. Release as-is, positioned for CDK / CodePipeline teams
- Effort: zero. Niche tools win by being sharp, not generic.
- The pin cards, inline CodeBuild logs, and multi-account switching already
  beat the AWS console for this niche.
- Risk: smaller audience.

### 2. Add "everyone has this" widgets  ← best value per effort
Universal AWS things, independent of CI or IaC choice:
- **CloudWatch alarms** status board (every company has alarms)
- **CloudWatch Logs Insights query** widget — the user writes the query, so it
  is generic by nature
- **Cost** widget — daily spend, month-to-date (everyone cares about the bill)
- ECS / EKS service health, or a simple EC2 / RDS inventory

Fits the existing architecture unchanged: one typed SDK crate per service,
policy gate, `renderTable`. No new concepts, just more widgets.

### 3. One truly generic "bring your own service" widget
- **Logs Insights query widget** — already generic, safe, cheap. Covers most
  "I want my own thing on the dashboard" needs. (Overlaps with option 2.)
- **Generic AWS CLI widget** — user supplies a read-only CLI command, app runs
  it on a schedule and renders the JSON as a table. Very flexible; policy.yaml
  and the compiled registry gate already give the safety story. Downsides:
  requires the AWS CLI installed, and mapping arbitrary JSON to columns is
  fiddly. Keep as a later escape hatch.
- **Generic SDK call in Rust** — skip. aws-sdk-rust is typed per service; there
  is no cheap "call any API" path, so this degenerates into adding crates per
  service anyway.

### 4. Real plugin system (widget SDK)
Users write widgets in JS with a manifest declaring the AWS calls they need.
Most generic, most work: sandboxing, stable API, docs, versioning. Premature
while there are zero external users. Build only when someone asks.

### 5. Open source + "how to write a widget" doc
Genericity from contributors instead of an engine. The widget pattern (build
tile, fetch, render, policy entry) is already repeatable. Cheap, but only pays
off if the project gets attention.

## Extra ideas

- **Multi-account is the real differentiator.** Most dashboards assume one
  account. SSO profile switching + per-account pinned widgets is the killer
  feature. A "status board" — the same widget pinned across N accounts side by
  side — is generic for any AWS org and is ~80% built already.
- **Read-only security widgets** are universal: IAM access keys older than 90
  days, Access Analyzer findings, ACM certificates about to expire.
- **Think in personas, not services.** A dev wants pipelines + logs; an ops
  person wants alarms + health; a lead wants cost + security. Ship widget
  "starter sets" per persona — same code, better story.

## Recommendation

1. Release as-is with honest positioning (option 1).
2. Then add the universal widgets: alarms, Logs Insights query, cost
   (option 2). The Logs Insights widget quietly delivers most of option 3.
3. Keep the generic CLI widget as a later escape hatch.
4. No plugin SDK until a stranger asks for one.
