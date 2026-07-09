#!/usr/bin/env python3
"""Fail release builds if source or packaged assets contain sensitive data."""

from __future__ import annotations

import argparse
import hashlib
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAX_BYTES = 250 * 1024 * 1024
MAX_FINDINGS_TO_PRINT = 80
MIN_BINARY_STRING = 6
KNOWN_VALUE_HASHES = {
    "known_account_number": {
        "046bf650ef789a1b283efb7e39f51dbf1500b86adbf50bd07476acd1e9d643d3",
    },
    "known_profile_name": {
        "95eef90128d3a873878efb430a243f197b3d8b8709a3d52c24c7483aa96affa6",
    },
    "known_client_marker": {
        "c718ccf49183aeecf51bdff0ba17e41a68c7bc153cd71b158efe8c54cf3e76ae",
    },
    "known_owner_handle": {
        "202ee8445e3ce73df6200e987e833397f37290f4741befec1c776549bb8e4dde",
    },
}
TOKEN_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{1,}")
TOKEN_SPLIT_RE = re.compile(r"[._-]+")


@dataclass(frozen=True)
class Rule:
    name: str
    regex: re.Pattern[str]


def rule_set() -> list[Rule]:
    return [
        Rule("aws_access_key_id", re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b")),
        Rule(
            "aws_credential_assignment",
            re.compile(
                r"\baws_(?:access_key_id|secret_access_key|session_token)\b"
                r"\s*[:=]\s*['\"]?[A-Za-z0-9/+=]{8,}",
                re.IGNORECASE,
            ),
        ),
        Rule(
            "private_key",
            re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----", re.IGNORECASE),
        ),
        Rule(
            "standalone_12_digit_number",
            re.compile(r"(?<![A-Za-z0-9])\d{12}(?![A-Za-z0-9])"),
        ),
        Rule(
            "local_user_path",
            re.compile(r"/Users/(?!runner(?:/|\b))[A-Za-z0-9._-]+(?:/[^\s'\"<>]*)?"),
        ),
    ]


def tracked_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
    )
    paths: list[Path] = []
    for raw_path in result.stdout.split(b"\0"):
        if raw_path:
            path = (ROOT / raw_path.decode("utf-8")).resolve()
            paths.append(path)
    return paths


def walked_files(paths: list[str]) -> list[Path]:
    files: list[Path] = []
    for raw_path in paths:
        path = Path(raw_path)
        if not path.is_absolute():
            path = ROOT / path
        path = path.resolve()
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(p for p in path.rglob("*") if p.is_file())
        else:
            raise SystemExit(f"path does not exist: {raw_path}")
    return files


def is_probably_text(data: bytes) -> bool:
    sample = data[:4096]
    if b"\0" in sample:
        return False
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def binary_strings(data: bytes) -> str:
    runs: list[str] = []
    current = bytearray()
    for byte in data:
        if 32 <= byte <= 126 or byte in (9,):
            current.append(byte)
        else:
            if len(current) >= MIN_BINARY_STRING:
                runs.append(current.decode("ascii", errors="ignore"))
            current.clear()
    if len(current) >= MIN_BINARY_STRING:
        runs.append(current.decode("ascii", errors="ignore"))
    return "\n".join(runs)


def file_text(path: Path) -> str:
    data = path.read_bytes()
    if len(data) > MAX_BYTES:
        raise SystemExit(f"refusing to scan unusually large file: {relative(path)}")
    if is_probably_text(data):
        return data.decode("utf-8")
    return binary_strings(data)


def relative(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def redact(value: str) -> str:
    value = " ".join(value.split())
    if len(value) <= 8:
        return value[:1] + "..." + value[-1:]
    return value[:4] + "..." + value[-4:]


def digest(value: str) -> str:
    return hashlib.sha256(value.lower().encode("utf-8")).hexdigest()


def candidate_terms(token: str) -> set[str]:
    terms = {token}
    parts = [part for part in TOKEN_SPLIT_RE.split(token) if part]
    terms.update(parts)
    return terms


def known_hash_findings(path: Path, line_number: int, line: str) -> list[str]:
    findings: list[str] = []
    seen: set[tuple[str, str]] = set()
    for token_match in TOKEN_RE.finditer(line):
        for term in candidate_terms(token_match.group(0)):
            term_digest = digest(term)
            for name, hashes in KNOWN_VALUE_HASHES.items():
                if term_digest in hashes and (name, term.lower()) not in seen:
                    findings.append(
                        f"{relative(path)}:{line_number}: {name}: {redact(term)}"
                    )
                    seen.add((name, term.lower()))
    return findings


def scan_file(path: Path, rules: list[Rule]) -> list[str]:
    text = file_text(path)
    findings: list[str] = []
    for line_number, line in enumerate(text.splitlines(), start=1):
        for rule in rules:
            for match in rule.regex.finditer(line):
                findings.append(
                    f"{relative(path)}:{line_number}: {rule.name}: "
                    f"{redact(match.group(0))}"
                )
        findings.extend(known_hash_findings(path, line_number, line))
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--path",
        action="append",
        default=[],
        help="scan a built artifact path instead of git-tracked source",
    )
    args = parser.parse_args()

    paths = walked_files(args.path) if args.path else tracked_files()
    rules = rule_set()
    findings: list[str] = []
    for path in paths:
        findings.extend(scan_file(path, rules))

    if findings:
        print("release privacy scan failed:", file=sys.stderr)
        for finding in findings[:MAX_FINDINGS_TO_PRINT]:
            print(f"  {finding}", file=sys.stderr)
        remaining = len(findings) - MAX_FINDINGS_TO_PRINT
        if remaining > 0:
            print(f"  ... and {remaining} more finding(s)", file=sys.stderr)
        return 1

    print(f"release privacy scan ok: {len(paths)} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
