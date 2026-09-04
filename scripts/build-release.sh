#!/usr/bin/env bash
# Local build only. Remote release/publication is a separate explicit action.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
if [[ $# -gt 0 ]]; then shift; fi
exec python3 "$ROOT/scripts/build_candidate.py" "$BUILD_TARGET" "$@"
