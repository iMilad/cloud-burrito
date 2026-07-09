#!/usr/bin/env python3
"""Validate that release tags and app manifests agree on one version."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TAG_RE = re.compile(r"^app-v(?P<version>\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)$")


def load_cargo_version() -> str:
    cargo_toml = ROOT / "src-tauri" / "Cargo.toml"
    try:
        import tomllib  # type: ignore[attr-defined]
    except ModuleNotFoundError:
        return parse_cargo_version_without_tomllib(cargo_toml)

    with cargo_toml.open("rb") as f:
        return tomllib.load(f)["package"]["version"]


def parse_cargo_version_without_tomllib(path: Path) -> str:
    in_package = False
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "[package]":
            in_package = True
            continue
        if in_package and line.startswith("["):
            break
        if in_package and line.startswith("version"):
            _, value = line.split("=", 1)
            return value.strip().strip('"')
    raise RuntimeError(f"package.version not found in {path}")


def load_tauri_version() -> str:
    with (ROOT / "src-tauri" / "tauri.conf.json").open(encoding="utf-8") as f:
        return json.load(f)["version"]


def normalize_ref_name(value: str | None) -> str:
    if not value:
        return ""
    if value.startswith("refs/tags/"):
        return value.removeprefix("refs/tags/")
    return value


def validate_tag(tag: str, version: str) -> None:
    match = TAG_RE.fullmatch(tag)
    if not match:
        raise SystemExit(f"release tag must look like app-v{version}; got {tag!r}")
    tag_version = match.group("version")
    if tag_version != version:
        raise SystemExit(
            f"release tag {tag!r} does not match app version {version!r}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("tag", nargs="?", help="optional release tag to validate")
    parser.add_argument(
        "--print-version",
        action="store_true",
        help="print the validated app version",
    )
    args = parser.parse_args()

    cargo_version = load_cargo_version()
    tauri_version = load_tauri_version()
    if cargo_version != tauri_version:
        raise SystemExit(
            "version mismatch: "
            f"src-tauri/Cargo.toml={cargo_version!r}, "
            f"src-tauri/tauri.conf.json={tauri_version!r}"
        )

    tag = normalize_ref_name(args.tag)
    if not tag:
        ref_type = os.environ.get("GITHUB_REF_TYPE")
        ref_name = normalize_ref_name(os.environ.get("GITHUB_REF_NAME"))
        if ref_type == "tag" or ref_name.startswith("app-v"):
            tag = ref_name

    if tag:
        validate_tag(tag, cargo_version)

    if args.print_version:
        print(cargo_version)
    else:
        print(f"release version ok: {cargo_version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
