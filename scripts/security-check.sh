#!/usr/bin/env bash
# Local public-release safety checks. This script never installs tools or calls
# network services; optional scanners run only when already available.
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
python3 -m py_compile scripts/check-release-privacy.py scripts/check-release-version.py

echo "== frontend syntax =="
node --check frontend/app.js
node --check frontend/mock-data.js

echo "== whitespace =="
git diff --check

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
  detect-secrets scan --all-files
else
  echo "skip: detect-secrets not installed"
fi

if command -v trufflehog >/dev/null 2>&1; then
  echo "== trufflehog verified filesystem scan =="
  tmp_json="$(mktemp "${TMPDIR:-/tmp}/cloud-burrito-trufflehog.XXXXXX")"
  trap 'rm -f "$tmp_json"' EXIT
  set +e
  trufflehog filesystem . --only-verified --json >"$tmp_json"
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
