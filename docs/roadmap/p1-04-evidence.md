# P1-04 — Verified CLI handoff and supervised execution

Status: **implemented and locally validated**, 2026-09-04.
Version remains `0.2.9`. The completed P1-01–03 work and roadmap were committed
locally as `a879851` before this unit. The user authorized local staging and
commits. No push, release, AWS request or real AWS CLI execution was performed.
Full Phase 1 remains in progress.

## Result and compatibility

P1-03 deliberately disabled desktop CLI execution until it could share the
verified connection. P1-04 replaces that block with explicit credential handoff
and a constrained process runner. The existing 18-operation parser and policy
rules remain; no new AWS service or command is added.

The command handler asks the already-verified session's frozen provider for its
exact credential set. It requires a complete temporary key, secret and session
token with the same expiry, then rechecks the context before execution. The
child receives those values only through its environment. The app never writes
them to arguments, configuration files, a temporary credential file or audit
records. No SSO resolution or ambient credential discovery occurs at handoff.

Active and pinned contexts retain separate identities. While a CLI request is
pending, the command checks its context every 100 ms. A superseded selection,
changed configuration, expired session or newer provider revision cancels the
affected request and awaits runner cleanup before returning a fenced result.
An unrelated topbar change does not cancel an independently pinned request.
The final context check still rejects a late result. Broader frontend tile and
detail ownership remains P1-05.

The launcher supports native executables and a narrow Unix Python wrapper form:
the shebang names one absolute Python interpreter without arguments. It verifies
that the interpreter resolves to a native executable, preserves its absolute
virtual-environment path, and launches it with `-I`, the canonical wrapper path,
and the validated CLI arguments. This covers the wrapper form found by read-only
inspection of the current Homebrew installation. No wrapper or interpreter was
executed during validation. Shell, batch and environment/PATH-based interpreter
wrappers are rejected. Native image/shebang inspection is not verification of
the CLI version, its packages or publisher; a trusted CLI v2 installation remains
a prerequisite.

## Child environment contract

The runner starts from `env_clear()`. Discovery uses absolute, nonempty PATH
entries and platform installation candidates; the child does not inherit PATH.
The selected program and wrapper/interpreter paths are absolute. Windows native
targets are checked again after canonicalization to reject batch-file targets.

| Setting | Treatment |
| --- | --- |
| AWS temporary key, secret and token | Exact STS-verified values from the captured session |
| AWS region and default region | Captured request region |
| HOME, USERPROFILE and temporary directories | A new owned temporary directory; also the working directory |
| AWS config/shared credentials and legacy Boto config paths | Platform null file |
| Metadata credentials, configured endpoint overrides, pager and automatic prompts | Disabled explicitly |
| Proxy variables | Narrow allowlist; lowercase wins on Unix; Windows case aliases must agree |
| AWS/Requests CA bundle, SSL certificate file/directory | Explicit allowlist; only absolute paths accepted |
| Windows SystemRoot/WINDIR | Preserved for native runtime requirements |
| Python user-site loading and text encoding | User-site loading disabled; explicit UTF-8 encoding; wrapper additionally uses isolated mode |
| Other AWS/provider/profile, model, loader, Python search and debugging variables | Not inherited |

The temporary home prevents loading personal CLI aliases, custom models and
home-based configuration. AWS documents the [alias execution surface](https://docs.aws.amazon.com/cli/latest/userguide/cli-usage-alias.html)
and [configuration/model environment settings](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-envvars.html).
The selected installation and explicitly preserved proxy/CA configuration remain
trusted inputs. Relative certificate paths now produce a clear error instead of
being interpreted against the isolated working directory.

## Process lifetime and diagnostics

- The execution deadline is 30 seconds, shortened to the captured credential
  lifetime if less remains. Credentials and cancellation are checked again
  immediately before spawning.
- Both pipes are drained concurrently. Stdout is capped at 2 MiB and stderr at
  256 KiB while reading, including output without newlines. One extra byte may
  be read to detect overflow; it is not appended to the retained buffer.
- Cancellation, timeout, overflow or read failure requests termination and
  awaits the direct child. A supervisor task retains cleanup ownership when the
  caller future is dropped. Missing-pipe failures use that ownership path too.
- A deadline triggers termination; it does not guarantee the OS confirms exit
  within 30 seconds. Cleanup can remain pending after a failed kill attempt.
  A failed wait reports that exit could not be confirmed. While the caller is
  still awaiting the request, that failure remains visible as `CliCleanupFailed`
  even if its account context was superseded, and a stable audit event retains
  the affected account/region without child diagnostics. This is not a process
  tree, Windows Job Object or Unix process-group guarantee.
- The app writes no credentials into the temporary home. It attempts to remove
  only its own empty directory after cleanup. Files created by an external CLI
  or an OS removal failure can leave the directory behind; native validation
  must check that behavior.
- Request/output Debug implementations omit credentials, arguments, raw output
  and status text. Nonzero child exits use a stable diagnostic rather than raw
  stderr. Runner errors are bounded, stripped of controls and checked against
  captured credentials. Successful output is checked both before and after JSON
  decoding, including object keys, so escaped credentials cannot reach a table.

## Validation

| Check | Observed result |
| --- | --- |
| Full Rust suite, locked/offline, normal parallel execution | **128 passed; 0 failed; 0 ignored; 0 filtered** |
| Production frontend auth handlers with synthetic bridge | **5 passed; 0 failed** |
| Release helper suite | **13 passed** |
| Exact STS-to-child credentials, inherited/pinned separation and refreshed credentials | Pass with controlled synthetic SSO/STS/process responses |
| Denied commands/policy and missing, invalid or expired authority | Zero process work; provider calls also absent where denied before verification |
| Context invalidation while a child is pending | Cancellation observed; command waits for the scripted cleanup outcome; stale data discarded |
| Cleanup failure after context replacement | Failure stays visible and audited; new active account preserved |
| Environment poisoning, proxy precedence, CA paths, native/Python discovery and command plan | Pass with synthetic values/files and no executable launch |
| Exact stream caps, overflow, endless output, concurrent pipes and exit/EOF ordering | Pass through the production supervisor with fake children and async streams |
| Timeout, cancellation, caller drop, read/kill/wait failures and isolated-home ownership | Pass with controlled lifecycle signals; no real OS process |
| Credential disclosure in errors, raw/escaped JSON strings and keys, and Debug | Synthetic credential markers withheld; audit contains no handoff credentials |
| All-target Clippy with warnings denied | Pass |
| Rust formatting, frontend syntax and diff whitespace | Pass |
| Version and Tauri command registry | Unchanged version 0.2.9 and 14 commands |
| Repository security script | Exit 0, with the qualifications below |

The security script's privacy scan covered 163 tracked files plus this new
evidence file. Gitleaks found no leaks in the current tree or local history;
Detect Secrets and TruffleHog reported no findings. TruffleHog completed but
reported sandbox restrictions on its temporary-process PID cleanup. The offline
dependency audit used its cached database, could not open the package-index
lock, and retained **20 allowed warnings** for unmaintained or unsound dependency
versions. These are not new dependency changes in P1-04, and this cached result
does not establish fresh public-release readiness. The release validation ledger
keeps that review open.

Reproduction from the repository root:

```sh
bash scripts/security-check.sh
npm run test:unit
```

For the complete Rust lint target set, from `src-tauri`:

```sh
cargo clippy -p cloud-burrito --locked --offline --all-targets -- -D warnings
```

The command fixtures exercise production handoff and result fences with synthetic
SSO/STS responses and controlled process completions. Runner fixtures exercise
the production supervisor using fake child controls, async pipes, explicit
cancellation/deadline signals and isolated temporary files. Native AWS/process
adapters still reject use in unit tests. Tests do not discover or execute the
real AWS CLI, read personal AWS configuration, modify process-wide environment
or access a service transport.

Native CLI/interpreter launch, installed package behavior, OS termination and
installer acceptance on macOS, Windows and Ubuntu remain pending. Dependency
and privacy checks are local evidence, not certification for public release.
No dependency version changes are needed for this unit.

Next: **P1-05 — bind every tile, detail and selector result to its current owner.**
