# P4-08 — Prepare inactive candidate orchestration

Implemented locally on 2026-09-04. No active workflow or existing release path
changed. Nothing was uploaded or executed remotely.

The example outside `.github/workflows` describes source validation, four native
build/inspection rows, complete receipt verification and final assembly. The
first job unconditionally stops; runner labels and helper inventories are
explicitly unconfigured. Activation requires a later reviewed authorized change.
All action references reuse existing pinned commits. Repository permissions are
read-only; exact target/input cache keys have no broad fallback.

Native inspectors run on native build hosts. The fan-in verifies the complete
four-target/seven-artifact set and binds it to the workflow source. It does not
pretend to repeat macOS or Windows native inspection on a generic Linux runner.
A failed row prevents final candidate retention; no tag/release/publish job exists.

Validation: 2 static contract tests pass for the stopped dependency graph,
immutable action references, matrix, cache inputs and complete assembly command.
This does not validate actual remote runner/action behavior.

See [inactive orchestration and activation prerequisites](../packaging/ci-orchestration.md).
