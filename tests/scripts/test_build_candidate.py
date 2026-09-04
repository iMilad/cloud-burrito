import importlib.util
import contextlib
import hashlib
import io
import json
import subprocess
import tarfile
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import build_candidate as BUILD
from packaging_contract import target_for


class BuildCandidateTests(unittest.TestCase):
    def test_no_shell_locked_offline_and_unsigned_for_every_target(self):
        for target in BUILD.load_matrix()['targets']:
            command = BUILD.build_command(target)
            self.assertIn('--no-sign', command)
            self.assertEqual(command[-3:], ['--', '--locked', '--offline'])
            self.assertNotIn('publish', command)
            self.assertIn(target['target'], command)

    def test_path_remaps_preserve_spaces_and_remove_provider_signing_inputs(self):
        row = target_for('macos-aarch64')
        with TemporaryDirectory(prefix='cloud burrito-é-') as directory:
            root = Path(directory).resolve()
            inherited = {'PATH': '/synthetic/tools', 'APPLE_TEAM_ID': 'synthetic',
                         'AWS_PROFILE': 'synthetic', 'TAURI_SIGNING_PRIVATE_KEY': 'synthetic',  # pragma: allowlist secret - literal synthetic fixture
                         'TAURI_BUNDLER_DMG_IGNORE_CI': 'true',
                         'CARGO_BUILD_RUSTC_WRAPPER': 'unreviewed',
                         'CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER': 'unreviewed',
                         'RUSTFLAGS': 'unreviewed', 'CARGO_HOME': str(root / 'cargo cache')}
            env = BUILD.build_environment(root / 'source tree', root / 'target', row, inherited)
            self.assertNotIn('APPLE_TEAM_ID', env)
            self.assertNotIn('AWS_PROFILE', env)
            self.assertNotIn('TAURI_SIGNING_PRIVATE_KEY', env)
            self.assertNotIn('RUSTFLAGS', env)
            self.assertNotIn('TAURI_BUNDLER_DMG_IGNORE_CI', env)
            self.assertEqual(env['CI'], 'true')
            self.assertNotIn('CARGO_BUILD_RUSTC_WRAPPER', env)
            self.assertNotIn('CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER', env)
            self.assertEqual(env['RUSTUP_AUTO_INSTALL'], '0')
            self.assertEqual(env['CARGO_NET_OFFLINE'], 'true')
            self.assertEqual(env['MACOSX_DEPLOYMENT_TARGET'], '13.0')
            self.assertIn(f'--remap-path-prefix={root / "source tree"}=.', env['CARGO_ENCODED_RUSTFLAGS'].split('\x1f'))

    def test_missing_duplicate_and_symlink_outputs_are_rejected(self):
        with TemporaryDirectory(prefix='cloud-burrito-output-') as directory:
            root = Path(directory)
            with self.assertRaises(BUILD.BuildError): BUILD.only_match(root, '*.dmg')
            (root / 'one.dmg').write_bytes(b'synthetic')
            self.assertEqual(BUILD.only_match(root, '*.dmg').name, 'one.dmg')
            (root / 'two.dmg').write_bytes(b'synthetic')
            with self.assertRaises(BUILD.BuildError): BUILD.only_match(root, '*.dmg')

    def test_input_digest_uses_git_bytes_not_windows_checkout_line_endings(self):
        with TemporaryDirectory(prefix='burrito-blob-') as directory:
            root = Path(directory)
            files = ['src-tauri/Cargo.lock', 'package-lock.json', 'rust-toolchain.toml',
                     'scripts/tool-versions.env', 'packaging/targets.json']
            for name in files:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'synthetic\r\n')
            def git(*args):
                if args[0] == 'status': return ''
                if args[0] == 'rev-parse': return 'a' * 40
                return '\n'.join(files)
            with patch.object(BUILD, 'ROOT', root), patch.object(BUILD, 'git', side_effect=git), \
                 patch.object(BUILD.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'synthetic\n')):
                result = BUILD.candidate_source()
            self.assertEqual(set(result['inputs_sha256'].values()), {hashlib.sha256(b'synthetic\n').hexdigest()})

    def test_source_export_preserves_executable_without_filter_api(self):
        def archive(args, **kwargs):
            with tarfile.open(fileobj=kwargs['stdout'], mode='w') as output:
                item = tarfile.TarInfo('folder/tool'); item.mode = 0o755; item.size = 9
                output.addfile(item, io.BytesIO(b'synthetic'))
            return subprocess.CompletedProcess(args, 0)
        with TemporaryDirectory(prefix='burrito-export-') as directory:
            root = Path(directory)
            with patch.object(BUILD.subprocess, 'run', side_effect=archive):
                BUILD.export_source(root, 'a' * 40)
            self.assertEqual((root / 'folder/tool').read_bytes(), b'synthetic')
            self.assertTrue((root / 'folder/tool').stat().st_mode & 0o111)

    def test_app_zip_normalizes_copy_without_changing_built_app(self):
        with TemporaryDirectory(prefix='burrito-zip-policy-') as directory:
            root = Path(directory); app = root / 'Cloud Burrito.app'
            app.mkdir(); binary = app / 'program'; binary.write_bytes(b'synthetic')
            binary.chmod(0o775)
            def archive(command, **kwargs):
                copied = Path(command[-2])
                self.assertEqual((copied / 'program').stat().st_mode & 0o777, 0o755)
                for option in ('--norsrc', '--noextattr', '--noacl', '--keepParent'):
                    self.assertIn(option, command)
            with patch.object(BUILD, 'run', side_effect=archive):
                BUILD.stage_app_zip(app, root / 'output.zip')
            self.assertEqual(binary.stat().st_mode & 0o777, 0o775)

    def test_wrong_native_host_fails_before_build(self):
        with patch.object(BUILD.platform, 'system', return_value='Darwin'):
            with self.assertRaises(BUILD.BuildError): BUILD.check_host(target_for('windows-x86_64'))
        with patch.object(BUILD, 'git', return_value=' M synthetic-file'):
            with self.assertRaises(BUILD.BuildError): BUILD.candidate_source()

    def test_missing_rust_target_fails_before_native_tools_run(self):
        with patch.object(BUILD, 'run', return_value='aarch64-apple-darwin'), \
             patch.object(BUILD.shutil, 'which') as which:
            with self.assertRaisesRegex(BUILD.BuildError, 'Rust target is not installed'):
                BUILD.check_native_tools(target_for('macos-x86_64'), SCRIPTS.parent, {'PATH': ''})
            which.assert_not_called()

    def test_missing_native_inspector_fails_before_compilation(self):
        with patch.object(BUILD, 'run', return_value='x86_64-pc-windows-msvc'), \
             patch.object(BUILD.shutil, 'which', side_effect=lambda name, **kw: None if name in ('7z', '7zz') else name):
            with self.assertRaisesRegex(BUILD.BuildError, 'artifact inspection'):
                BUILD.check_native_tools(target_for('windows-x86_64'), SCRIPTS.parent, {'PATH': ''})

    def test_preflight_keeps_existing_outputs_and_never_compiles(self):
        with TemporaryDirectory(prefix='burrito-preflight-') as directory, contextlib.ExitStack() as stack:
            root = Path(directory).resolve()
            output = root / 'candidate'; output.mkdir()
            sentinel = output / 'existing'; sentinel.write_bytes(b'synthetic candidate')
            bundle = root / 'src-tauri/target/aarch64-apple-darwin/release/bundle'
            bundle.mkdir(parents=True)
            (bundle / 'existing').write_bytes(b'synthetic bundle')
            identity = {'source_commit': 'a' * 40}
            def export(destination, revision):
                (destination / 'scripts').mkdir()
                (destination / 'scripts/artifact_inspection.py').touch()
                (destination / 'package-lock.json').write_text('{}')
            def run(command, **kwargs):
                if command[:2] == ['cargo', 'metadata']:
                    self.assertIn('--offline', command)
                    self.assertIn('--locked', command)
                    return json.dumps({'target_directory': str(root / 'src-tauri/target')})
                self.assertNotEqual(command[:3], ['cargo', 'tauri', 'build'])
                return ''
            for name, replacement in [('ROOT', root), ('check_host', lambda row: None),
                                      ('candidate_source', lambda: identity), ('export_source', export),
                                      ('tool_versions', lambda source, env: {}),
                                      ('check_native_tools', lambda *args: None), ('run', run)]:
                stack.enter_context(patch.object(BUILD, name, replacement))
            stack.enter_context(patch('dependency_inventory.from_metadata', return_value={}))
            stage = stack.enter_context(patch.object(BUILD, 'stage_artifacts'))
            captured = stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            BUILD.build(target_for('macos-aarch64'), output, check_only=True)
            report = json.loads(captured.getvalue())
            self.assertEqual(report['status'], 'preflight passed')
            self.assertFalse(report['built'])
            self.assertFalse(report['device_validated'])
            self.assertEqual(sentinel.read_bytes(), b'synthetic candidate')
            self.assertEqual((bundle / 'existing').read_bytes(), b'synthetic bundle')
            self.assertFalse((root / 'dist/build-backups').exists())
            stage.assert_not_called()


if __name__ == '__main__': unittest.main()
