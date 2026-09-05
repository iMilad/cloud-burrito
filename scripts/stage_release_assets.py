#!/usr/bin/env python3
"""Preserve inspected macOS candidate bytes and receipts for a draft release.

No archive is rebuilt and no package is executed or published. The receipts
and checksums establish consistency, not authenticity or device validation.
The complete four-platform candidate assembler retains its separate contract.
"""
import argparse
import os
from pathlib import Path
import sys

from assemble_candidate import (MAX_ARTIFACT, MAX_JSON, collect, directory,
                                fingerprint, hex_digest, no_redirect, require)
from packaging_contract import load_matrix, target_for

SUMS = 'SHA256SUMS.txt'


def sum_name(row):
    return f"SHA256SUMS-{row['id'].removeprefix('macos-')}.txt"


def checksums(records):
    return ''.join(f"{record['sha256']}  {record['filename']}\n"
                   for record in sorted(records, key=lambda record: record['filename'])).encode()


def require_checksums(path, expected):
    before = fingerprint(path, MAX_JSON)
    with path.open('rb') as stream:
        content = stream.read(MAX_JSON + 1)
    require(before == fingerprint(path, MAX_JSON) and content == expected,
            'Checksums differ from inspected bytes and preserved receipts')


def checked(parent, rows, commit, version, *, with_checksums=False, with_combined=False):
    matrix = load_matrix()
    require(hex_digest(commit, 40), 'Expected a full workflow source commit')
    require(rows and all(row['platform'] == 'macos' for row in rows), 'Only declared macOS targets belong to this release')
    extras = [sum_name(row) for row in rows] if with_checksums else []
    if with_combined:
        extras.append(SUMS)
    result, catalog = collect([parent], matrix, assembled=True,
                              target_ids=[row['id'] for row in rows], extra_files=extras)
    require(result['source_commit'] == commit and result['version'] == version,
            'Release candidate differs from the workflow source or version')
    if with_checksums:
        for row in rows:
            expected = checksums([item for item in result['files'] if item['target'] == row['target']])
            path = catalog[sum_name(row)]
            require_checksums(path, expected)
    if with_combined:
        path = catalog[SUMS]
        require_checksums(path, checksums(result['files']))
    return result, catalog


def stage(target, input_dir, output, commit, version):
    row = target_for(target)
    summary, catalog = checked(input_dir, [row], commit, version)
    output = Path(output).absolute()
    require('..' not in output.parts, 'Output traversal is not accepted')
    no_redirect(output)
    require(not output.exists(), 'Release staging output must be new')
    directory(output.parent)
    require(not any(path.parent in output.parents for path in catalog.values()), 'Output cannot be inside an input directory')
    output.mkdir()
    for record in summary['files']:
        source = catalog[record['filename']]
        with (output / record['filename']).open('xb') as stream:
            actual = fingerprint(source, MAX_JSON if record['role'] == 'build-manifest' else MAX_ARTIFACT, stream)
        require(actual == {key: record[key] for key in ('size_bytes', 'sha256')}, 'Candidate changed while staging')
        os.chmod(output / record['filename'], 0o644)
    with (output / sum_name(row)).open('xb') as stream:
        stream.write(checksums(summary['files']))
    return checked(output, [row], commit, version, with_checksums=True)[0]


def verify(input_dir, commit, version, *, write_combined=False):
    rows = [row for row in load_matrix()['targets'] if row['platform'] == 'macos']
    combined = Path(input_dir) / SUMS
    summary, _ = checked(input_dir, rows, commit, version,
                         with_checksums=True, with_combined=combined.exists())
    if write_combined and not combined.exists():
        with combined.open('xb') as stream:
            stream.write(checksums(summary['files']))
    if write_combined:
        return checked(input_dir, rows, commit, version, with_checksums=True, with_combined=True)[0]
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    pack = commands.add_parser('stage')
    pack.add_argument('--target', required=True)
    pack.add_argument('--output', type=Path, required=True)
    check = commands.add_parser('verify')
    check.add_argument('--write-combined', action='store_true')
    for command in (pack, check):
        command.add_argument('--input-dir', type=Path, required=True)
        command.add_argument('--source-commit', required=True)
        command.add_argument('--version', required=True)
    args = parser.parse_args()
    try:
        result = (stage(args.target, args.input_dir, args.output, args.source_commit, args.version)
                  if args.command == 'stage' else
                  verify(args.input_dir, args.source_commit, args.version, write_combined=args.write_combined))
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        print('Release assets rejected: incomplete, changed or inconsistent candidate evidence.', file=sys.stderr)
        return 1
    print(f"Verified preserved release bytes and receipts: {result['version']}, {len(result['targets'])} macOS target(s); unsigned evidence only.")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
