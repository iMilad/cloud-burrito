#!/usr/bin/env bash
# Run the app in dev mode. Pure Rust now — no sidecar to build first.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/src-tauri"

if [[ "$(uname -s)" == "Darwin" && -z "${SDKROOT:-}" ]]; then
  sdkroot="$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)"
  if [[ -n "$sdkroot" && -d "$sdkroot" ]]; then
    export SDKROOT="$sdkroot"
  fi
fi

cargo tauri dev
