#!/usr/bin/env bash
# Build the same path-sanitized, lockfile-constrained bundle locally and in CI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
# shellcheck source=tool-versions.env
source "$ROOT/scripts/tool-versions.env"

if [[ -z "$TARGET" ]]; then
  echo "could not determine the Rust host target" >&2
  exit 1
fi

actual_tauri_version="$(cargo tauri --version)"
if [[ "$actual_tauri_version" != "tauri-cli $TAURI_CLI_VERSION" ]]; then
  echo "expected tauri-cli $TAURI_CLI_VERSION; got $actual_tauri_version" >&2
  echo "install it with: cargo install tauri-cli --version $TAURI_CLI_VERSION --locked" >&2
  exit 1
fi

export RUSTFLAGS="--remap-path-prefix=${ROOT}=. --remap-path-prefix=${HOME}/.cargo=.cargo"

(
  cd "$ROOT/src-tauri"

  # Release artifacts are intentionally identity-free. Clear any inherited
  # Apple-specific environment before invoking the explicit no-sign path.
  while IFS= read -r variable; do
    unset "$variable"
  done < <(compgen -e APPLE_ || true)
  if compgen -e APPLE_ >/dev/null; then
    echo "could not clear Apple-specific build environment" >&2
    exit 1
  fi

  cargo tauri build --ci --no-sign --target "$TARGET" -- --locked
)

APP="$ROOT/src-tauri/target/$TARGET/release/bundle/macos/Cloud Burrito.app"
python3 "$ROOT/scripts/check-release-privacy.py" --path "$APP"

echo "release bundle ok: $APP"
