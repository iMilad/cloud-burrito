#!/usr/bin/env python3
"""Report whether the validated app version is ready for a release tag."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")
COMMAND_TIMEOUT_SECONDS = 20

RELEASE_PENDING = 0
ERROR = 2
RELEASE_TAG_EXISTS = 10
VERSION_BUMP_REQUIRED = 20


class ReleaseStatusError(RuntimeError):
    """Raised when release status cannot be determined safely."""


@dataclass(frozen=True)
class ReleaseStatus:
    state: str
    tag: str
    version: str
    head: str
    tag_commit: str | None

    @property
    def exit_code(self) -> int:
        return {
            "release-pending": RELEASE_PENDING,
            "release-tag-exists": RELEASE_TAG_EXISTS,
            "version-bump-required": VERSION_BUMP_REQUIRED,
        }[self.state]

    def message(self) -> str:
        if self.state == "release-pending":
            return f"release pending: {self.tag}"
        if self.state == "release-tag-exists":
            return f"release tag exists: {self.tag}"
        return f"version bump required: {self.tag} already points to another commit"


def run(command: list[str], *, root: Path) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["GIT_TERMINAL_PROMPT"] = "0"
    env["GCM_INTERACTIVE"] = "Never"
    try:
        return subprocess.run(
            command,
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
            timeout=COMMAND_TIMEOUT_SECONDS,
            env=env,
        )
    except subprocess.TimeoutExpired as exc:
        raise ReleaseStatusError(
            f"{command[0]} timed out after {COMMAND_TIMEOUT_SECONDS} seconds"
        ) from exc
    except OSError as exc:
        raise ReleaseStatusError(f"could not run {command[0]}: {exc}") from exc


def command_error(label: str, result: subprocess.CompletedProcess[str]) -> None:
    detail = result.stderr.strip() or result.stdout.strip() or f"exit {result.returncode}"
    raise ReleaseStatusError(f"{label} failed: {detail}")


def validated_version(root: Path) -> str:
    checker = root / "scripts" / "check-release-version.py"
    result = run([sys.executable, str(checker), "--print-version"], root=root)
    if result.returncode != 0:
        command_error("release metadata validation", result)
    version = result.stdout.strip()
    if not VERSION_RE.fullmatch(version):
        raise ReleaseStatusError(
            f"release metadata returned an invalid version: {version!r}"
        )
    return version


def head_commit(root: Path) -> str:
    result = run(["git", "rev-parse", "--verify", "HEAD^{commit}"], root=root)
    if result.returncode != 0:
        command_error("HEAD resolution", result)
    return result.stdout.strip()


def parse_remote_tag(output: str, tag: str) -> str:
    direct_ref = f"refs/tags/{tag}"
    peeled_ref = f"{direct_ref}^{{}}"
    refs: dict[str, str] = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 2 or fields[1] not in {direct_ref, peeled_ref}:
            raise ReleaseStatusError(f"unexpected remote tag response: {line!r}")
        refs[fields[1]] = fields[0]
    if peeled_ref in refs:
        return refs[peeled_ref]
    if direct_ref in refs:
        return refs[direct_ref]
    raise ReleaseStatusError(f"remote returned no usable ref for {tag}")


def remote_tag_commit(root: Path, remote: str, tag: str) -> str | None:
    direct_ref = f"refs/tags/{tag}"
    peeled_ref = f"{direct_ref}^{{}}"
    result = run(
        [
            "git",
            "ls-remote",
            "--exit-code",
            "--tags",
            "--",
            remote,
            direct_ref,
            peeled_ref,
        ],
        root=root,
    )
    if result.returncode == 2 and not result.stdout.strip():
        return None
    if result.returncode != 0:
        raise ReleaseStatusError(
            f"remote tag lookup for {tag} failed with exit {result.returncode}"
        )
    return parse_remote_tag(result.stdout, tag)


def classify(
    version: str,
    head: str,
    tag_commit: str | None,
) -> ReleaseStatus:
    tag = f"app-v{version}"
    if tag_commit is None:
        state = "release-pending"
    elif tag_commit == head:
        state = "release-tag-exists"
    else:
        state = "version-bump-required"
    return ReleaseStatus(
        state=state,
        tag=tag,
        version=version,
        head=head,
        tag_commit=tag_commit,
    )


def main(argv: list[str] | None = None, *, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--remote",
        default="origin",
        help="Git remote or URL whose release tag is authoritative (default: origin)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="print machine-readable status details",
    )
    args = parser.parse_args(argv)
    root = root.resolve()

    try:
        version = validated_version(root)
        head = head_commit(root)
        tag = f"app-v{version}"
        tag_commit = remote_tag_commit(root, args.remote, tag)
        status = classify(version, head, tag_commit)
    except ReleaseStatusError as exc:
        print(f"release status error: {exc}", file=sys.stderr)
        return ERROR

    if args.json:
        print(json.dumps(asdict(status), sort_keys=True))
    else:
        print(status.message())
    return status.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
