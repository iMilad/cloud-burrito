#!/usr/bin/env python3
"""Keep Tauri's handler, app manifest, and capability command lists aligned."""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def extract(pattern: str, text: str, source: Path) -> str:
    match = re.search(pattern, text, re.DOTALL)
    if not match:
        raise SystemExit(f"could not find command list in {source}")
    return match.group(1)


def main() -> int:
    build_path = ROOT / "src-tauri" / "build.rs"
    lib_path = ROOT / "src-tauri" / "src" / "lib.rs"
    capability_path = ROOT / "src-tauri" / "capabilities" / "default.json"

    build_block = extract(
        r"\.commands\s*\(\s*&\[(.*?)\]\s*\)",
        build_path.read_text(encoding="utf-8"),
        build_path,
    )
    manifest_commands = re.findall(r'"([a-z][a-z0-9_]*)"', build_block)

    handler_block = extract(
        r"tauri::generate_handler!\s*\[(.*?)\]",
        lib_path.read_text(encoding="utf-8"),
        lib_path,
    )
    handler_commands = re.findall(r"commands::([a-z][a-z0-9_]*)", handler_block)

    capability = json.loads(capability_path.read_text(encoding="utf-8"))
    permissions = capability.get("permissions")
    if not isinstance(permissions, list):
        raise SystemExit("capability permissions must be a list")

    unexpected_permissions = [
        permission
        for permission in permissions
        if not isinstance(permission, str)
        or re.fullmatch(r"allow-[a-z][a-z0-9-]*", permission) is None
    ]
    if unexpected_permissions:
        raise SystemExit(
            "capability contains permissions outside the explicit app-command "
            f"allowlist: {unexpected_permissions}"
        )

    capability_commands = [
        permission.removeprefix("allow-").replace("-", "_")
        for permission in permissions
    ]

    command_sets = {
        "build manifest": manifest_commands,
        "invoke handler": handler_commands,
        "capability": capability_commands,
    }
    for name, commands in command_sets.items():
        if len(commands) != len(set(commands)):
            raise SystemExit(f"duplicate command in {name}: {commands}")

    expected = set(manifest_commands)
    mismatches = {
        name: sorted(set(commands) ^ expected)
        for name, commands in command_sets.items()
        if set(commands) != expected
    }
    if mismatches:
        details = "; ".join(f"{name}: {items}" for name, items in mismatches.items())
        raise SystemExit(f"Tauri command lists differ: {details}")

    print(f"Tauri command registry ok: {len(expected)} commands")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
