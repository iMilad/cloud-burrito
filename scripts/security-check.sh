#!/usr/bin/env bash
# Local public-release safety checks. This script does not invoke installers or
# make AWS/credential-validation calls; optional scanners run when available.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export PYTHONPYCACHEPREFIX="${PYTHONPYCACHEPREFIX:-${TMPDIR:-/tmp}/cloud-burrito-pycache}"

echo "== release metadata =="
python3 scripts/check-release-version.py

echo "== source privacy scan =="
python3 scripts/check-release-privacy.py

untracked_paths=()
while IFS= read -r -d '' path; do
  untracked_paths+=("$path")
done < <(git ls-files --others --exclude-standard -z)
if (( ${#untracked_paths[@]} > 0 )); then
  echo "== untracked privacy scan =="
  untracked_args=()
  for path in "${untracked_paths[@]}"; do
    untracked_args+=(--path "$path")
  done
  python3 scripts/check-release-privacy.py "${untracked_args[@]}"
else
  echo "untracked privacy scan ok: no non-ignored untracked files"
fi

echo "== script syntax =="
python3 -m py_compile \
  scripts/check-release-privacy.py \
  scripts/check-release-version.py \
  scripts/check-tauri-commands.py
bash -n scripts/*.sh

echo "== Tauri command registry =="
python3 scripts/check-tauri-commands.py

echo "== frontend syntax =="
node --check frontend/app.js
node --check frontend/mock-data.js

echo "== whitespace =="
git diff --check
untracked_whitespace=0
if (( ${#untracked_paths[@]} > 0 )); then
  for path in "${untracked_paths[@]}"; do
    findings="$(git diff --no-index --check -- /dev/null "$path" 2>&1 || true)"
    if [[ -n "$findings" ]]; then
      printf '%s\n' "$findings" >&2
      untracked_whitespace=1
    fi
  done
fi
if (( untracked_whitespace != 0 )); then
  exit 1
fi

echo "== Rust format =="
(
  cd src-tauri
  cargo fmt --all --check
)

echo "== Rust tests =="
(
  cd src-tauri
  cargo test -p cloud-burrito --locked --offline
)

echo "== Rust lint =="
(
  cd src-tauri
  cargo clippy -p cloud-burrito --locked --offline -- -D warnings
)

if command -v cargo-audit >/dev/null 2>&1; then
  echo "== Rust dependency audit =="
  (
    cd src-tauri
    cargo audit --no-fetch --stale
  )
else
  echo "skip: cargo-audit not installed"
fi

if command -v gitleaks >/dev/null 2>&1; then
  echo "== gitleaks current tree =="
  gitleaks detect --source . --no-git --redact --verbose

  echo "== gitleaks git history =="
  gitleaks detect --source . --redact --verbose
else
  echo "skip: gitleaks not installed"
fi

if command -v detect-secrets >/dev/null 2>&1; then
  echo "== detect-secrets current tree =="
  (
    tmp_detect="$(mktemp "${TMPDIR:-/tmp}/cloud-burrito-detect-secrets.XXXXXX")"
    trap 'rm -f "$tmp_detect"' EXIT
    detect-secrets scan \
      --all-files \
      --no-verify \
      --exclude-files '(^|/)(\.git|node_modules|src-tauri/(target|gen)|playwright-report|test-results)(/|$)' \
      >"$tmp_detect"
    node - "$tmp_detect" <<'NODE'
const fs = require("fs");
const file = process.argv[2];
const report = JSON.parse(fs.readFileSync(file, "utf8"));
const results = report.results || {};
let count = 0;
for (const [filename, findings] of Object.entries(results)) {
  for (const finding of findings) {
    count += 1;
    const type = finding.type || "unknown detector";
    const line = finding.line_number || "?";
    console.log(`detect-secrets: ${type} in ${filename}:${line}`);
  }
}
if (count > 0) {
  console.error(`detect-secrets found ${count} potential secret(s)`);
  process.exit(1);
}
console.log("detect-secrets: no findings");
NODE
  )
else
  echo "skip: detect-secrets not installed"
fi

if command -v trufflehog >/dev/null 2>&1; then
  echo "== trufflehog filesystem scan =="
  tmp_json="$(mktemp "${TMPDIR:-/tmp}/cloud-burrito-trufflehog.XXXXXX")"
  tmp_exclude="$(mktemp "${TMPDIR:-/tmp}/cloud-burrito-trufflehog-exclude.XXXXXX")"
  trap 'rm -f "$tmp_json" "$tmp_exclude"' EXIT
  printf '^src-tauri/target/\n^src-tauri/gen/\n' >"$tmp_exclude"
  set +e
  trufflehog filesystem . \
    --no-update \
    --no-verification \
    --exclude-paths "$tmp_exclude" \
    --json >"$tmp_json"
  status=$?
  set -e
  if [[ -s "$tmp_json" ]]; then
    node - "$tmp_json" <<'NODE'
const fs = require("fs");
const file = process.argv[2];
const lines = fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean);
for (const line of lines) {
  let item;
  try {
    item = JSON.parse(line);
  } catch {
    console.log("trufflehog: unparseable finding");
    continue;
  }
  const detector = item.DetectorName || item.detector_name || "unknown";
  const source = item.SourceName || item.source_name || "unknown";
  const redacted = item.Redacted || item.redacted || "[redacted]";
  console.log(`trufflehog: ${detector} in ${source}: ${redacted}`);
}
NODE
    exit 1
  fi
  exit "$status"
else
  echo "skip: trufflehog not installed"
fi
