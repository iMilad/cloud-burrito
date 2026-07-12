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


def load_tool_versions() -> dict[str, str]:
    versions: dict[str, str] = {}
    path = ROOT / "scripts" / "tool-versions.env"
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if not separator or not key or not value:
            raise SystemExit(f"invalid tool version line in {path}: {raw_line!r}")
        versions[key] = value
    return versions


def load_cargo_version() -> str:
    cargo_toml = ROOT / "src-tauri" / "Cargo.toml"
    try:
        import tomllib  # type: ignore[attr-defined]
    except ModuleNotFoundError:
        return parse_cargo_version_without_tomllib(cargo_toml)

    with cargo_toml.open("rb") as f:
        return tomllib.load(f)["package"]["version"]


def load_cargo_rust_version() -> str:
    cargo_toml = ROOT / "src-tauri" / "Cargo.toml"
    try:
        import tomllib  # type: ignore[attr-defined]
    except ModuleNotFoundError:
        return parse_cargo_package_field_without_tomllib(cargo_toml, "rust-version")

    with cargo_toml.open("rb") as f:
        return tomllib.load(f)["package"]["rust-version"]


def load_toolchain_version() -> str:
    toolchain_toml = ROOT / "rust-toolchain.toml"
    try:
        import tomllib  # type: ignore[attr-defined]
    except ModuleNotFoundError:
        return parse_toolchain_version_without_tomllib(toolchain_toml)

    with toolchain_toml.open("rb") as f:
        return tomllib.load(f)["toolchain"]["channel"]


def parse_cargo_version_without_tomllib(path: Path) -> str:
    return parse_cargo_package_field_without_tomllib(path, "version")


def parse_cargo_package_field_without_tomllib(path: Path, field: str) -> str:
    in_package = False
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "[package]":
            in_package = True
            continue
        if in_package and line.startswith("["):
            break
        if in_package and line.startswith(f"{field}"):
            _, value = line.split("=", 1)
            return value.strip().strip('"')
    raise RuntimeError(f"package.{field} not found in {path}")


def parse_toolchain_version_without_tomllib(path: Path) -> str:
    in_toolchain = False
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "[toolchain]":
            in_toolchain = True
            continue
        if in_toolchain and line.startswith("["):
            break
        if in_toolchain and line.startswith("channel"):
            _, value = line.split("=", 1)
            return value.strip().strip('"')
    raise RuntimeError(f"toolchain.channel not found in {path}")


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

    cargo_rust_version = load_cargo_rust_version()
    toolchain_version = load_toolchain_version()
    if cargo_rust_version != toolchain_version:
        raise SystemExit(
            "Rust version mismatch: "
            f"src-tauri/Cargo.toml={cargo_rust_version!r}, "
            f"rust-toolchain.toml={toolchain_version!r}"
        )

    tool_versions = load_tool_versions()
    required_tools = {"TAURI_CLI_VERSION", "CARGO_AUDIT_VERSION"}
    if set(tool_versions) != required_tools:
        raise SystemExit(
            "tool version keys differ: "
            f"expected {sorted(required_tools)}, got {sorted(tool_versions)}"
        )

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    tauri_cli_version = tool_versions["TAURI_CLI_VERSION"]
    if f"**Tauri CLI** {tauri_cli_version}" not in readme:
        raise SystemExit(
            f"README Tauri CLI prerequisite must be {tauri_cli_version}"
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
