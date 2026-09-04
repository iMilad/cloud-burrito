#!/usr/bin/env python3
"""Read-only SHA-256 inventory check; never downloads or executes a helper."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat

from packaging_contract import load_matrix, target_for

NSIS_REQUIRED = {
    'NSIS/makensis.exe', 'NSIS/Bin/makensis.exe',
    'NSIS/Stubs/lzma-x86-unicode', 'NSIS/Stubs/lzma_solid-x86-unicode',
    'NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll',
    'NSIS/Include/MUI2.nsh', 'NSIS/Include/FileFunc.nsh', 'NSIS/Include/x64.nsh',
    'NSIS/Include/nsDialogs.nsh', 'NSIS/Include/WinMessages.nsh',
    'NSIS/Include/Win/COM.nsh', 'NSIS/Include/Win/Propkey.nsh', 'NSIS/Include/Win/RestartManager.nsh',
    'NSIS/Contrib/Language files/English.nsh', 'NSIS/Contrib/Modern UI 2/MUI2.nsh',
} | {f'NSIS/Plugins/x86-unicode/{name}.dll' for name in ('NSISdl', 'StartMenu', 'System', 'nsDialogs')}
APPIMAGE_REQUIRED = {
    'AppRun-x86_64', 'linuxdeploy-x86_64.AppImage', 'linuxdeploy-plugin-gtk.sh',
    'linuxdeploy-plugin-gstreamer.sh', 'linuxdeploy-plugin-appimage.AppImage',
}
PLUGIN_PATH = 'NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll'
PLUGIN_SHA1 = '75197fee3c6a814fe035788d1c34ead39349b860'  # pragma: allowlist secret - public upstream checksum
MAX_FILES = 10000
MAX_BYTES = 1024 * 1024 * 1024


def is_redirect(path):
    """Reject symlinks and Windows junction/reparse points without following them."""
    try:
        info = path.lstat()
    except FileNotFoundError:
        return False
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, 'st_file_attributes', 0) & 0x400)


def regular_path(root, relative):
    parts = PurePosixPath(relative).parts
    if not parts or PurePosixPath(relative).is_absolute() or any(part in ('', '.', '..') for part in relative.split('/')) or '\\' in relative or ':' in relative or any(ord(c) < 32 for c in relative):
        raise ValueError('Unsafe helper path')
    current = root
    for part in parts:
        current = current / part
        if is_redirect(current):
            raise ValueError('Helper redirects are not accepted')
    before = current.stat()
    if not stat.S_ISREG(before.st_mode):
        raise ValueError('Helper is not a regular file')
    return current, before


def verify_cache(target, cargo_target_dir, inventory_path, matrix=None):
    row = target_for(target, matrix or load_matrix())
    if row['platform'] not in ('windows', 'linux'):
        raise ValueError('This preflight is for the Windows and Linux native helpers')
    target_dir = Path(cargo_target_dir)
    if not target_dir.is_absolute() or any(is_redirect(parent) for parent in [target_dir, *target_dir.parents]):
        raise ValueError('An absolute non-symlink Cargo target directory is required')
    cache = target_dir / '.tauri'
    if is_redirect(cache) or not cache.is_dir():
        raise ValueError('The explicit local helper cache is unavailable')
    path = Path(inventory_path)
    if not path.is_file() or path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError('Helper inventory exceeds its read budget')
    with path.open('rb') as stream:
        encoded = stream.read(2 * 1024 * 1024 + 1)
    if len(encoded) > 2 * 1024 * 1024:
        raise ValueError('Helper inventory exceeds its read budget')
    inventory = json.loads(encoded)
    if not isinstance(inventory, dict):
        raise ValueError('Helper inventory must be an object')
    if (inventory.get('schema_version'), inventory.get('tauri_cli'), inventory.get('tauri_bundler'), inventory.get('target')) != (1, '2.11.4', '2.9.4', row['target']):
        raise ValueError('Helper inventory does not match the pinned target')
    entries = inventory.get('files')
    if not isinstance(entries, list) or not 1 <= len(entries) <= MAX_FILES:
        raise ValueError('A nonempty reviewed helper inventory is required')
    listed, total = set(), 0
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {'path', 'size', 'sha256'}:
            raise ValueError('Invalid inventory entry')
        name = entry['path']
        if not isinstance(name, str) or name in listed or not isinstance(entry['sha256'], str) or not re.fullmatch(r'[0-9a-f]{64}', entry['sha256']):
            raise ValueError('Invalid or duplicate helper identity')
        if row['platform'] == 'windows' and not name.startswith('NSIS/') or row['platform'] == 'linux' and name not in APPIMAGE_REQUIRED:
            raise ValueError('Helper path is outside the declared platform cache')
        if type(entry['size']) is not int or not 0 < entry['size'] <= MAX_BYTES:
            raise ValueError('Invalid helper byte size')
        total += entry['size']
        if total > MAX_BYTES:
            raise ValueError('Helper inventory exceeds its byte budget')
        helper, before = regular_path(cache, name)
        if before.st_size != entry['size']:
            raise ValueError('Helper byte size differs from reviewed inventory')
        digest, upstream = hashlib.sha256(), hashlib.sha1()
        with helper.open('rb') as stream:
            opened = os.fstat(stream.fileno())
            if (opened.st_dev, opened.st_ino, opened.st_size) != (before.st_dev, before.st_ino, before.st_size):
                raise ValueError('Helper changed during inspection')
            remaining = entry['size']
            while remaining:
                block = stream.read(min(1024 * 1024, remaining))
                if not block:
                    raise ValueError('Helper ended before its declared size')
                remaining -= len(block)
                digest.update(block)
                upstream.update(block)
            if stream.read(1):
                raise ValueError('Helper grew during inspection')
            after = os.fstat(stream.fileno())
        if (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino) or digest.hexdigest() != entry['sha256']:
            raise ValueError('Helper hash differs from reviewed inventory')
        if name == PLUGIN_PATH and upstream.hexdigest() != PLUGIN_SHA1:
            raise ValueError('NSIS plugin would trigger an upstream redownload')
        listed.add(name)
    required = NSIS_REQUIRED if row['platform'] == 'windows' else APPIMAGE_REQUIRED
    if not required <= listed:
        raise ValueError('Required native helper files are absent from the inventory')
    if row['platform'] == 'windows':
        actual = set()
        for directory, dirs, names in os.walk(cache / 'NSIS', followlinks=False):
            if any(is_redirect(Path(directory) / name) for name in [*dirs, *names]):
                raise ValueError('NSIS cache contains a redirect')
            for name in names:
                actual.add((Path(directory) / name).relative_to(cache).as_posix())
                if len(actual) > MAX_FILES:
                    raise ValueError('NSIS cache exceeds the file budget')
        if actual != listed:
            raise ValueError('Every NSIS toolchain file must match the reviewed inventory')
    return {'target': row['target'], 'files': len(listed), 'bytes': total, 'cache_relative_to_cargo_target': '.tauri',
            'network_enforced': False, 'executed': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--cargo-target-dir', required=True)
    parser.add_argument('--inventory', required=True, help='Previously reviewed SHA-256 inventory; this command never generates or approves one')
    parser.add_argument('--json', action='store_true', help='Emit the machine-readable report (also the default)')
    args = parser.parse_args()
    try:
        result = verify_cache(args.target, args.cargo_target_dir, args.inventory)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
        parser.exit(1, 'Native helper preflight failed. Required local inputs are missing, changed or unreviewed; automatic acquisition is disabled.\n')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
