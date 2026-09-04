#!/usr/bin/env python3
"""Assemble or verify a complete local unsigned candidate; never build or publish.

Checksums and inspection reports describe these bytes, not their authenticity.
Verification does not rerun native inspectors or compare against today's Git
checkout. In particular, the NSIS script remains in its original build cache;
only its digest is retained, so its policy cannot be repeated from this set.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

from packaging_contract import artifact_names, load_matrix

MAX_ARTIFACT = 1024 * 1024 * 1024
MAX_JSON = 2 * 1024 * 1024
MANIFEST = 'candidate-manifest.json'
SUMS = 'SHA256SUMS'
LIMITATIONS = [
    'Unsigned metadata and checksums establish consistency, not publisher authenticity or trustworthy provenance.',
    'Recorded successful native inspections are descriptive claims; this verifier does not rerun them.',
    'The generated NSIS script remains in its original local build cache; its recorded hash alone cannot repeat installer policy inspection.',
    'Source cleanliness was checked at build time; assembly permits a prior matching source and does not inspect the current checkout.',
    'Installation, launch, upgrade, uninstall and device compatibility require separate P5 evidence.',
]


class AssemblyError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise AssemblyError(message)


def hex_digest(value, length=64):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{' + str(length) + '}', value) is not None


def safe_relative(value):
    return (isinstance(value, str) and 0 < len(value) <= 4096 and
            not any(ord(c) < 32 for c in value) and '\\' not in value and ':' not in value and
            all(part not in ('', '.', '..') for part in value.split('/')))


def no_redirect(path):
    for part in [path, *path.parents]:
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(info.st_mode) and not getattr(info, 'st_file_attributes', 0) & 0x400,
                'Symlinks and reparse points are not accepted')


def directory(path):
    path = Path(path).absolute()
    require('..' not in path.parts, 'Directory traversal is not accepted')
    no_redirect(path)
    require(path.is_dir(), 'Expected a regular candidate directory')
    return path


def regular(path, maximum):
    no_redirect(path)
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= maximum,
            'Candidate input must be a nonempty bounded regular file')
    require(info.st_nlink == 1, 'Hardlinked candidate inputs are not accepted')
    return info


def fingerprint(path, maximum=MAX_ARTIFACT, destination=None):
    before = regular(path, maximum)
    digest = hashlib.sha256()
    with path.open('rb') as source:
        opened = os.fstat(source.fileno())
        require((before.st_dev, before.st_ino, before.st_size) == (opened.st_dev, opened.st_ino, opened.st_size),
                'Candidate input changed before reading')
        remaining = before.st_size
        while remaining:
            block = source.read(min(1024 * 1024, remaining))
            require(bool(block), 'Candidate input ended early')
            remaining -= len(block)
            digest.update(block)
            if destination is not None:
                destination.write(block)
        require(not source.read(1), 'Candidate input grew during reading')
        after = os.fstat(source.fileno())
    current = regular(path, maximum)
    identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
    require(identity(before) == identity(after) == identity(current), 'Candidate input changed during reading')
    return {'size_bytes': before.st_size, 'sha256': digest.hexdigest()}


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'Duplicate JSON field')
        result[key] = value
    return result


def read_json(path):
    regular(path, MAX_JSON)
    with path.open('rb') as source:
        data = source.read(MAX_JSON + 1)
    require(len(data) <= MAX_JSON, 'JSON input exceeds its limit')
    try:
        value = json.loads(data, object_pairs_hook=unique_object,
                           parse_constant=lambda _: (_ for _ in ()).throw(AssemblyError('Nonfinite JSON number')))
    except (ValueError, UnicodeError, RecursionError) as error:
        raise AssemblyError('Malformed candidate JSON') from error
    require(isinstance(value, dict), 'Candidate JSON must be an object')
    return value


def source_identity(manifest, matrix):
    identity = {key: manifest.get(key) for key in ('source_commit', 'source_tree', 'inputs_sha256')}
    require(hex_digest(identity['source_commit'], 40) and hex_digest(identity['source_tree'], 40),
            'Full source commit and tree identities are required')
    hashes = identity['inputs_sha256']
    require(isinstance(hashes, dict) and 1 <= len(hashes) <= 10000, 'Build input hashes are required')
    require(all(safe_relative(name) and hex_digest(value) for name, value in hashes.items()), 'Unsafe build input identity')
    required = {'src-tauri/Cargo.lock', 'package-lock.json', 'rust-toolchain.toml',
                'scripts/tool-versions.env', 'packaging/targets.json', 'src-tauri/tauri.conf.json'}
    required.update(row['bundle_config'] for row in matrix['targets'])
    require(required <= set(hashes), 'Protected build input hashes are missing')
    digest = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    require(manifest.get('candidate_input_sha256') == digest, 'Candidate source-input digest mismatch')
    require(manifest.get('source_clean') is True and
            manifest.get('source_export') == 'immutable tracked Git tree; no local ignored inputs',
            'A clean immutable source export must be recorded')
    return {**identity, 'candidate_input_sha256': digest}


def inspection_assertions(item, row, version):
    """Require the native inspector's structured claims, never turn them into proof."""
    details = item.get('inspection')
    require(isinstance(details, dict) and details, 'Artifact lacks actual inspection details')
    require(hex_digest(item.get('payload_sha256')), 'Artifact payload digest is missing')
    kind = item['format']
    if row['platform'] == 'macos':
        require(details.get('architecture') == row['binary_arch'] and
                details.get('identifier') == 'app.cloudburrito.desktop' and details.get('minimum_os') == '13.0' and
                details.get('publisher_identity') is False, 'Mac inspection claims are incomplete')
        if kind == 'dmg':
            require(details.get('container_verified') is True and details.get('matches_app_zip') is True,
                    'DMG container and app comparison were not recorded')
    else:
        executable = details.get('executable')
        require(isinstance(executable, dict) and executable.get('machine') == row['binary_arch'],
                'Application architecture inspection is missing')
        if row['platform'] == 'linux':
            require(hex_digest(details.get('executable_sha256')), 'Linux application executable digest is missing')
        if kind == 'nsis':
            require(details.get('script_install_mode') == 'currentUser' and details.get('custom_hooks') is False and
                    hex_digest(details.get('generated_script_sha256')), 'NSIS script inspection is missing')
            require(executable.get('publisher_certificate') is False and
                    isinstance(details.get('installer'), dict) and details['installer'].get('publisher_certificate') is False,
                    'Unsigned Windows inspection was not recorded')
        elif kind == 'deb':
            control = details.get('control', {})
            require(isinstance(control, dict) and control.get('Architecture') == 'amd64' and
                    control.get('Version') == version and details.get('maintainer_hooks') is False,
                    'Debian control inspection is missing')
        elif kind == 'appimage':
            require(isinstance(details.get('runtime'), dict) and details['runtime'].get('machine') == 'x86-64' and
                    details.get('extraction') == 'external archive tool', 'AppImage static extraction was not recorded')


def collect(input_dirs, matrix, assembled=False):
    require(len(input_dirs) == (1 if assembled else 4), 'Exactly four target directories are required')
    directories = [directory(path) for path in input_dirs]
    require(len(set(directories)) == len(directories), 'Duplicate target directory')
    catalog, groups = {}, []
    for parent in directories:
        names = set()
        for path in parent.iterdir():
            require(len(names) < 16 and safe_relative(path.name) and '/' not in path.name, 'Unexpected directory contents')
            require(path.name not in catalog, 'Duplicate candidate filename')
            regular(path, MAX_JSON if path.name.startswith('build-manifest-') else MAX_ARTIFACT)
            names.add(path.name)
            catalog[path.name] = path
        groups.append(names)
    manifest_names = {f"build-manifest-{row['id']}.json": row for row in matrix['targets']}
    require(set(manifest_names) <= set(catalog), 'A required target build manifest is missing')
    records, common, version = [], None, None
    expected = set(manifest_names)
    for name, row in manifest_names.items():
        path = catalog[name]
        before = fingerprint(path, MAX_JSON)
        manifest = read_json(path)
        require(before == fingerprint(path, MAX_JSON), 'Build manifest changed while parsing')
        require(manifest.get('schema_version') == 1 and manifest.get('matrix_revision') == matrix['matrix_revision'] and
                manifest.get('target') == row['target'], 'Build manifest target or matrix mismatch')
        current_version = manifest.get('version')
        require(isinstance(current_version, str) and re.fullmatch(r'\d+\.\d+\.\d+', current_version), 'Invalid candidate version')
        identity = source_identity(manifest, matrix)
        if common is None:
            common, version = identity, current_version
        require(identity == common and version == current_version, 'Candidate contains mixed source identities or versions')
        inspection = manifest.get('inspection', {})
        require(isinstance(inspection, dict) and inspection.get('schema_version') == 1 and
                inspection.get('target') == row['target'] and inspection.get('version') == version and
                inspection.get('status') == 'passed', 'Successful native target inspection is required')
        wanted = {item['filename'].format(version=version): item['format'] for item in row['artifacts']}
        items = inspection.get('artifacts')
        require(isinstance(items, list) and len(items) == len(wanted), 'Incomplete inspected artifact set')
        seen = set()
        for item in items:
            require(isinstance(item, dict), 'Malformed inspected artifact')
            filename = item.get('filename')
            require(isinstance(filename, str) and filename in wanted and filename not in seen and
                    item.get('format') == wanted[filename], 'Unexpected, duplicate or mismatched artifact identity')
            seen.add(filename)
            require(filename in catalog and catalog[filename].parent == path.parent, 'Artifact is absent from its target directory')
            inspection_assertions(item, row, version)
            actual = fingerprint(catalog[filename])
            require(type(item.get('size_bytes')) is int and item['size_bytes'] == actual['size_bytes'] and
                    item.get('sha256') == actual['sha256'], 'Artifact differs from its recorded inspection')
            records.append({'filename': filename, 'role': 'artifact', 'target': row['target'],
                            'format': wanted[filename], **actual})
        if row['platform'] == 'macos':
            require(len({item['payload_sha256'] for item in items}) == 1, 'Mac DMG and ZIP payload claims differ')
        elif row['platform'] == 'linux':
            require(len({item['inspection']['executable_sha256'] for item in items}) == 1,
                    'Linux Debian and AppImage application executable claims differ')
        if not assembled:
            require(set(next(group for group in groups if name in group)) == set(wanted) | {name},
                    'Each target directory must contain only its artifacts and build manifest')
        records.append({'filename': name, 'role': 'build-manifest', 'target': row['target'], **before})
        expected.update(wanted)
    require(set(artifact_names(version, matrix)) == {record['filename'] for record in records if record['role'] == 'artifact'},
            'Candidate does not contain the complete matrix artifact set')
    require(set(catalog) == expected | ({MANIFEST, SUMS} if assembled else set()), 'Missing or extra candidate files')
    summary = {'schema_version': 1, 'matrix_revision': matrix['matrix_revision'], 'version': version, **common,
               'status': 'complete-recorded-inspections', 'targets': [row['target'] for row in matrix['targets']],
               'files': sorted(records, key=lambda record: record['filename']), 'limitations': LIMITATIONS}
    return summary, catalog


def checksums(summary, manifest_fingerprint):
    values = {record['filename']: record['sha256'] for record in summary['files']}
    values[MANIFEST] = manifest_fingerprint['sha256']
    return ''.join(f'{values[name]}  {name}\n' for name in sorted(values))


def verify(path, matrix=None):
    matrix = matrix or load_matrix()
    summary, catalog = collect([path], matrix, assembled=True)
    require(read_json(catalog[MANIFEST]) == summary, 'Candidate manifest does not match preserved build evidence')
    sums = catalog[SUMS]
    regular(sums, MAX_JSON)
    with sums.open('rb') as stream:
        content = stream.read(MAX_JSON + 1)
    require(content == checksums(summary, fingerprint(catalog[MANIFEST], MAX_JSON)).encode(),
            'Checksum file is incomplete, duplicated, altered or inconsistent')
    return summary


def assemble(input_dirs, output, matrix=None):
    matrix = matrix or load_matrix()
    output = Path(output).absolute()
    require('..' not in output.parts, 'Output traversal is not accepted')
    no_redirect(output)
    require(not output.exists(), 'Output must be a new directory; existing files are never overwritten')
    directory(output.parent)
    summary, catalog = collect(input_dirs, matrix)
    require(not any(parent in output.parents for parent in {path.parent for path in catalog.values()}),
            'Output cannot be nested inside an input directory')
    output.mkdir()
    # Exclusive writes preserve existing outputs. If a source changes mid-copy,
    # the new directory remains incomplete and cannot pass independent verify.
    for record in summary['files']:
        source = catalog[record['filename']]
        with (output / record['filename']).open('xb') as stream:
            actual = fingerprint(source, MAX_JSON if record['role'] == 'build-manifest' else MAX_ARTIFACT, stream)
        require(actual == {key: record[key] for key in ('size_bytes', 'sha256')}, 'Input changed after preflight')
        os.chmod(output / record['filename'], 0o755 if source.stat().st_mode & 0o111 else 0o644)
    with (output / MANIFEST).open('x', encoding='utf-8', newline='\n') as stream:
        json.dump(summary, stream, indent=2, sort_keys=True)
        stream.write('\n')
    with (output / SUMS).open('x', encoding='utf-8', newline='\n') as stream:
        stream.write(checksums(summary, fingerprint(output / MANIFEST, MAX_JSON)))
    return verify(output, matrix)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    pack = commands.add_parser('assemble', help='Copy four target directories into one new complete candidate')
    pack.add_argument('--input-dir', type=Path, action='append', required=True)
    pack.add_argument('--output', type=Path, required=True)
    check = commands.add_parser('verify', help='Check all files against preserved unsigned build evidence')
    check.add_argument('directory', type=Path)
    args = parser.parse_args()
    try:
        result = assemble(args.input_dir, args.output) if args.command == 'assemble' else verify(args.directory)
    except (OSError, ValueError, TypeError, KeyError, RecursionError):
        print('Candidate rejected: incomplete, unsafe, changed or inconsistent local evidence. No authenticity claim was established.', file=sys.stderr)
        return 1
    print(f"Candidate consistency verified: {result['version']}, 4 targets, 7 artifacts; unsigned evidence is not publisher authentication.")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
