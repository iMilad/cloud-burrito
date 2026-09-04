#!/usr/bin/env python3
"""Keep secret findings fatal except cryptographically proved benchmark hashes.

This is a report postprocessor, not a scanner exclusion. Only whole hexadecimal
fingerprints in the generated benchmark metadata schema are eligible. No secret
values, source bytes, or Git object contents are printed; all checks are local.
"""
import argparse
from functools import lru_cache
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
MAX_METADATA_BYTES = 64 * 1024
MAX_REPORT_BYTES = 8 * 1024 * 1024
MAX_SOURCE_BYTES = 2 * 1024 * 1024
MAX_HISTORY_COMMITS = 128
BENCHMARK = re.compile(r"docs/roadmap/benchmarks/p3-[a-z0-9-]+\.json\Z")
SOURCE = re.compile(r"(?:frontend/(?:app|mock-data|row-filter-worker)\.js|frontend/(?:index\.html|styles\.css)|src-tauri/src/(?:[a-z0-9_]+/)*[a-z0-9_]+\.rs)\Z")
REVISION = re.compile(r"[0-9a-f]{40}\Z")
HEADER = re.compile(r'\{\n  "schema_version": 1,\n  "recorded_at_utc": "[0-9T:.+Z-]{1,64}",\n  "metadata": ')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate metadata key")
        result[key] = value
    return result


class Fingerprints:
    def __init__(self, root=ROOT):
        self.root = Path(root).resolve()

    def local_file(self, relative, limit):
        path = self.root / relative
        # No candidate path can point outside the checkout or through a symlink.
        if path.is_symlink() or path.resolve() != path or not path.is_file():
            raise ValueError("Not a local regular source file")
        with path.open("rb") as stream:
            value = stream.read(limit + 1)
        if len(value) > limit:
            raise ValueError("Source exceeds verification budget")
        return value

    def git(self, *args):
        try:
            result = subprocess.run(
                ["git", "--no-pager", *args], cwd=self.root,
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5, check=True,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        return result.stdout if len(result.stdout) <= MAX_SOURCE_BYTES else None

    @lru_cache(maxsize=128)
    def commit_exists(self, revision):
        return bool(REVISION.fullmatch(revision)) and self.git("cat-file", "-t", revision) == b"commit\n"

    @lru_cache(maxsize=128)
    def current_digest(self, source):
        if not SOURCE.fullmatch(source) or Path(source).name.startswith(("benchmarks", "test_")):
            return None
        try:
            return hashlib.sha256(self.local_file(source, MAX_SOURCE_BYTES)).hexdigest()
        except (OSError, ValueError):
            return None

    @lru_cache(maxsize=128)
    def source_digests(self, source):
        if not SOURCE.fullmatch(source) or Path(source).name.startswith(("benchmarks", "test_")):
            return frozenset()
        known = set()
        revisions = self.git("log", "--all", "--no-show-signature", "--format=%H",
                             f"--max-count={MAX_HISTORY_COMMITS}", "--", source)
        if revisions is not None:
            for revision in revisions.decode("ascii", errors="replace").splitlines():
                if not REVISION.fullmatch(revision):
                    continue
                ref = f"{revision}:{source}"
                size = self.git("cat-file", "-s", ref)
                if size is None or not size.strip().isdigit() or int(size) > MAX_SOURCE_BYTES:
                    continue
                contents = self.git("cat-file", "blob", ref)
                if contents is not None and len(contents) == int(size):
                    known.add(hashlib.sha256(contents).hexdigest())
        return frozenset(known)

    @lru_cache(maxsize=32)
    def metadata_lines(self, filename):
        if not BENCHMARK.fullmatch(filename):
            return {}
        path = self.root / filename
        try:
            if path.is_symlink() or path.resolve() != path or not path.is_file():
                return {}
            with path.open("rb") as stream:
                prefix = stream.read(MAX_METADATA_BYTES).decode("utf-8")
            header = HEADER.match(prefix)
            if not header:
                return {}
            metadata, end = json.JSONDecoder(object_pairs_hook=unique_object).raw_decode(prefix, header.end())
            if not isinstance(metadata, dict) or not prefix[end:].startswith(",\n"):
                return {}
            # Require the generator's exact canonical layout, which makes line
            # numbers unambiguous without parsing the large measurement arrays.
            canonical = json.dumps(metadata, indent=2).replace("\n", "\n  ")
            if prefix[header.end():end] != canonical:
                return {}
            revision = metadata.get("source_revision")
            sources = metadata.get("production_sha256")
            if not isinstance(revision, str) or not self.commit_exists(revision) or not isinstance(sources, dict) or len(sources) > 128:
                return {}
            candidates = {}
            inside_sources = False
            for line_number, line in enumerate(prefix[:end].splitlines(), 1):
                if line == '    "production_sha256": {':
                    inside_sources = True
                    continue
                if inside_sources and line in ('    },', '    }'):
                    inside_sources = False
                    continue
                if inside_sources:
                    match = re.fullmatch(r'      "([^"\\]+)": "([0-9a-f]{64})",?', line)
                    if match and sources.get(match[1]) == match[2] and SOURCE.fullmatch(match[1]):
                        candidates[line_number] = (match[2], match[1])
                elif line == f'    "source_revision": "{revision}",':
                    candidates[line_number] = (revision, None)
            return candidates
        except (OSError, ValueError, TypeError, RecursionError):
            return {}

    def verified(self, filename, finding):
        if finding.get("type") != "Hex High Entropy String":
            return False
        line = finding.get("line_number")
        if type(line) is not int or line < 1:
            return False
        candidate = self.metadata_lines(filename).get(line)
        if candidate is None:
            return False
        digest, source = candidate
        # detect-secrets records SHA-1 of the exact matched token. A second
        # finding on the same line, a substring or another detector stays fatal.
        if finding.get("hashed_secret") != hashlib.sha1(digest.encode("ascii")).hexdigest():
            return False
        return source is None or digest == self.current_digest(source) or digest in self.source_digests(source)


def check_report(report, fingerprints, output=sys.stdout):
    if not isinstance(report, dict) or not isinstance(report.get("results"), dict):
        raise ValueError("Invalid scanner report")
    remaining = verified = 0
    for filename, findings in report["results"].items():
        if not isinstance(filename, str) or not isinstance(findings, list):
            raise ValueError("Invalid scanner findings")
        for finding in findings:
            if not isinstance(finding, dict):
                raise ValueError("Invalid scanner finding")
            if fingerprints.verified(filename, finding):
                verified += 1
                continue
            remaining += 1
            detector = json.dumps(str(finding.get("type", "unknown detector")))[1:-1]
            label = json.dumps(filename)[1:-1]
            line = finding.get("line_number") if type(finding.get("line_number")) is int else "?"
            print(f"detect-secrets: {detector} in {label}:{line}", file=output)
    if verified:
        print(f"detect-secrets: verified {verified} public benchmark source fingerprints", file=output)
    if remaining:
        print(f"detect-secrets found {remaining} potential secret(s)", file=output)
        return 1
    print("detect-secrets: no unverified findings", file=output)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    try:
        with args.report.open("rb") as stream:
            data = stream.read(MAX_REPORT_BYTES + 1)
        if len(data) > MAX_REPORT_BYTES:
            raise ValueError("Report exceeds budget")
        return check_report(json.loads(data), Fingerprints())
    except (OSError, ValueError, TypeError, RecursionError):
        print("detect-secrets: report could not be safely verified", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
