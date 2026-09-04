#!/usr/bin/env python3
"""Build one local unsigned candidate. Never install, launch, fetch or publish it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import uuid

from packaging_contract import ROOT, load_matrix, target_for


class BuildError(ValueError):
    pass


def run(args, *, cwd=ROOT, env=None, capture=False):
    result = subprocess.run(args, cwd=cwd, env=env, check=False,
                            stdout=subprocess.PIPE if capture else None,
                            stderr=subprocess.PIPE if capture else None)
    if result.returncode:
        # Compiler logs may have local paths; keep them local, out of manifests.
        raise BuildError(f"{Path(args[0]).name} failed with exit {result.returncode}")
    return result.stdout.decode('utf-8').strip() if capture else None


def git(*args):
    return run(['git', '--no-pager', *args], capture=True)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def release_version(root=ROOT):
    config = json.loads((root / 'src-tauri/tauri.conf.json').read_text())
    version = config.get('version', '')
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise BuildError('Invalid release version')
    return version


def candidate_source():
    if git('status', '--porcelain=v1', '--untracked-files=all'):
        raise BuildError('Candidate requires a clean tracked/untracked source tree; commit the work first')
    revision, tree = git('rev-parse', 'HEAD'), git('rev-parse', 'HEAD^{tree}')
    if not re.fullmatch(r'[0-9a-f]{40}', revision) or not re.fullmatch(r'[0-9a-f]{40}', tree):
        raise BuildError('Expected full local Git object IDs')
    files = git('ls-files').splitlines()
    protected = ['src-tauri/Cargo.lock', 'package-lock.json', 'rust-toolchain.toml',
                 'scripts/tool-versions.env', 'packaging/targets.json']
    protected += [name for name in files if name.startswith(('frontend/', 'src-tauri/icons/'))
                  or re.fullmatch(r'src-tauri/tauri(?:\.[a-z]+)?\.conf\.json', name)]
    hashes = {}
    for name in sorted(set(protected)):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise BuildError('Build input must be a tracked regular file')
        hashes[name] = sha256(path)
    identity = {'source_commit': revision, 'source_tree': tree, 'inputs_sha256': hashes}
    identity['candidate_input_sha256'] = hashlib.sha256(
        json.dumps(identity, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    return identity


def export_source(destination, revision):
    # Export only Git's immutable tracked tree, excluding ignored payload files,
    # .git, credentials, node_modules and build outputs from the local checkout.
    with tempfile.TemporaryFile() as archive:
        process = subprocess.run(['git', 'archive', '--format=tar', revision], cwd=ROOT, stdout=archive)
        if process.returncode:
            raise BuildError('Tracked source export failed')
        archive.seek(0)
        with tarfile.open(fileobj=archive) as tar:
            for member in tar.getmembers():
                path = Path(member.name)
                if path.is_absolute() or '..' in path.parts or member.issym() or member.islnk() or not (member.isdir() or member.isfile()):
                    raise BuildError('Source export contains a link or unsafe member')
            tar.extractall(destination, filter='data')


def build_environment(source_root, target_dir, row, inherited=None):
    env = dict(os.environ if inherited is None else inherited)
    for key in list(env):
        if key.startswith(('APPLE_', 'AWS_', 'TAURI_SIGNING_', 'TAURI_PRIVATE_', 'CARGO_PROFILE_')) or key in (
            'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'MACOSX_DEPLOYMENT_TARGET',
            'TAURI_CONFIG', 'TAURI_BUNDLER_SIGN', 'TAURI_BUNDLER_SIGNING_IDENTITY',
            'TAURI_BUNDLER_DMG_IGNORE_CI',
            'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTDOCFLAGS',
        ):
            env.pop(key, None)
    cargo_home = Path(env.get('CARGO_HOME') or Path.home() / '.cargo').absolute()
    rust_home = Path(env.get('RUSTUP_HOME') or Path.home() / '.rustup').absolute()
    # Unit separators preserve one rustc argument even with spaces/Unicode.
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join([
        f'--remap-path-prefix={source_root}=.', f'--remap-path-prefix={ROOT}=.',
        f'--remap-path-prefix={cargo_home}=.cargo', f'--remap-path-prefix={rust_home}=.rustup',
    ])
    env['CARGO_TARGET_DIR'] = str(target_dir)
    env['CARGO_NET_OFFLINE'] = 'true'
    env['AWS_EC2_METADATA_DISABLED'] = 'true'
    env['CI'] = 'true'
    if row['platform'] == 'macos':
        env['MACOSX_DEPLOYMENT_TARGET'] = '13.0'
    return env


def build_command(row):
    return ['cargo', 'tauri', 'build', '--ci', '--no-sign', '--target', row['target'],
            '--bundles', ','.join(row['bundle_targets']), '--', '--locked', '--offline']


def check_host(row):
    if platform.system() != row['host_os']:
        raise BuildError(f"Target {row['id']} needs its declared native {row['host_os']} build host")
    if row['platform'] == 'linux':
        values = {}
        for line in Path('/etc/os-release').read_text().splitlines():
            if '=' in line:
                key, value = line.split('=', 1); values[key] = value.strip('"')
        if (values.get('ID'), values.get('VERSION_ID'), platform.machine()) != ('ubuntu', '22.04', 'x86_64'):
            raise BuildError('Linux candidate requires the declared Ubuntu 22.04 x64 build baseline')


def tool_versions(source_root):
    pins = dict(line.split('=', 1) for line in (source_root / 'scripts/tool-versions.env').read_text().splitlines()
                if line and not line.startswith('#'))
    tauri = run(['cargo', 'tauri', '--version'], cwd=source_root, capture=True)
    if tauri != f"tauri-cli {pins['TAURI_CLI_VERSION']}":
        raise BuildError('Installed Tauri CLI differs from the repository pin')
    rust = run(['rustc', '--version'], cwd=source_root, capture=True)
    pin = re.search(r'channel\s*=\s*"([^"]+)"', (source_root / 'rust-toolchain.toml').read_text())[1]
    if rust.split()[1] != pin:
        raise BuildError('Installed Rust differs from the repository pin')
    versions = {'rustc': rust, 'cargo': run(['cargo', '--version'], cwd=source_root, capture=True),
                'tauri': tauri, 'python': platform.python_version(), 'os': platform.system(),
                'os_release': platform.release(), 'host_architecture': platform.machine()}
    if platform.system() == 'Darwin':
        versions['sdk'] = run(['xcrun', '--show-sdk-version'], capture=True)
        versions['clang'] = run(['xcrun', 'clang', '--version'], capture=True).splitlines()[0]
    return versions


def only_match(directory, pattern):
    matches = sorted(directory.glob(pattern))
    if len(matches) != 1 or matches[0].is_symlink() or not matches[0].is_file():
        raise BuildError(f'Expected exactly one {pattern} artifact in its dedicated bundle directory')
    return matches[0]


def stage_artifacts(row, release_dir, output, version):
    bundle = release_dir / 'bundle'
    for artifact in row['artifacts']:
        destination = output / artifact['filename'].format(version=version)
        fmt = artifact['format']
        if fmt == 'app.zip':
            app = bundle / 'macos/Cloud Burrito.app'
            if app.is_symlink() or not app.is_dir():
                raise BuildError('Expected app bundle is missing')
            run(['ditto', '-c', '-k', '--keepParent', str(app), str(destination)])
        else:
            directory, pattern = {'dmg': ('dmg', '*.dmg'), 'nsis': ('nsis', '*-setup.exe'),
                                  'deb': ('deb', '*.deb'), 'appimage': ('appimage', '*.AppImage')}[fmt]
            source = only_match(bundle / directory, pattern)
            shutil.copy2(source, destination)


def build(row, output, helper_inventory=None):
    check_host(row)
    identity = candidate_source()
    if output.exists():
        raise BuildError('Output directory must be new; existing candidates are never overwritten')
    output.parent.mkdir(parents=True, exist_ok=True)
    # The compiler cache is reusable, but native bundle output is always fresh.
    target_dir = ROOT / 'src-tauri/target'
    if target_dir.is_symlink() or target_dir.resolve() != target_dir:
        raise BuildError('Compiler target directory must not be redirected')
    release_dir = target_dir / row['target'] / 'release'
    with tempfile.TemporaryDirectory(prefix='cloud-burrito-candidate-') as temporary:
        source_root = Path(temporary) / 'source'; source_root.mkdir()
        export_source(source_root, identity['source_commit'])
        if not (source_root / 'scripts/artifact_inspection.py').is_file():
            raise BuildError('Format-aware inspection is required before candidate builds (P4-07)')
        versions = tool_versions(source_root)
        env = build_environment(source_root, target_dir, row)
        run([sys.executable, str(source_root / 'scripts/check-release-version.py')], cwd=source_root)
        run([sys.executable, str(ROOT / 'scripts/check-release-privacy.py')], cwd=ROOT)
        run([sys.executable, str(source_root / 'scripts/check-tauri-commands.py')], cwd=source_root)
        metadata = json.loads(run(['cargo', 'metadata', '--no-deps', '--format-version=1', '--offline', '--locked'],
                                  cwd=source_root / 'src-tauri', env=env, capture=True))
        if Path(metadata['target_directory']).resolve() != target_dir.resolve():
            raise BuildError('Cargo target directory differs from the verified native helper cache')
        helper_report = None
        if row['platform'] != 'macos':
            checker = source_root / 'scripts/check-native-helper-cache.py'
            if not checker.is_file() or not helper_inventory:
                raise BuildError('Pre-provisioned, verified native helper inventory is required; automatic downloads are disabled')
            helper_report = json.loads(run([sys.executable, str(checker), '--target', row['target'],
                '--cargo-target-dir', str(target_dir), '--inventory', str(helper_inventory), '--json'], cwd=source_root, capture=True))
        output.mkdir()
        previous = release_dir / 'bundle'
        if previous.exists():
            if previous.is_symlink():
                raise BuildError('Bundle output must not be a symlink')
            backup = ROOT / 'dist/build-backups' / (row['id'] + '-' + uuid.uuid4().hex[:10])
            backup.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(previous), str(backup))
        run(build_command(row), cwd=source_root / 'src-tauri', env=env)
        version = release_version(source_root)
        stage_artifacts(row, release_dir, output, version)
        # P4-07 supplies format-aware inspectors. A partial implementation must
        # never call uninspected files a completed candidate.
        from artifact_inspection import inspect_target
        inspection = inspect_target(row, output, version, source_root=source_root, build_dir=release_dir)
        if candidate_source() != identity:
            raise BuildError('Source changed during build; candidate is not accepted')
        manifest = {'schema_version': 1, 'matrix_revision': load_matrix()['matrix_revision'],
                    'version': version, 'target': row['target'], **identity,
                    'source_clean': True, 'source_export': 'immutable tracked Git tree; no local ignored inputs',
                    'tool_versions': versions, 'build_arguments': build_command(row),
                    'path_remapping': ['source=.', 'cargo=.cargo', 'rustup=.rustup'],
                    'network_policy': 'Cargo offline; verified native helper cache required; helper network isolation is separate',
                    'native_helpers': helper_report, 'inspection': inspection}
        (output / f"build-manifest-{row['id']}.json").write_text(json.dumps(manifest, indent=2) + '\n')
        print(f"Inspected local candidate: {row['id']} at source {identity['source_commit'][:12]}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('target', help='Target ID or triple from packaging/targets.json')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--helper-inventory', type=Path)
    parser.add_argument('--plan', action='store_true', help='Print declared build plan without building')
    args = parser.parse_args()
    try:
        row = target_for(args.target)
        if args.plan:
            print(json.dumps({'target': row, 'command': build_command(row), 'publication': False}, indent=2))
            return 0
        output = args.output or ROOT / 'dist/candidates' / git('rev-parse', '--short=12', 'HEAD') / row['id']
        build(row, output.absolute(), args.helper_inventory)
        return 0
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f'Candidate not accepted: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
