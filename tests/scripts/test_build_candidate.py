import importlib.util
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
                         'AWS_PROFILE': 'synthetic', 'TAURI_SIGNING_PRIVATE_KEY': 'synthetic',
                         'TAURI_BUNDLER_DMG_IGNORE_CI': 'true',
                         'RUSTFLAGS': 'unreviewed', 'CARGO_HOME': str(root / 'cargo cache')}
            env = BUILD.build_environment(root / 'source tree', root / 'target', row, inherited)
            self.assertNotIn('APPLE_TEAM_ID', env)
            self.assertNotIn('AWS_PROFILE', env)
            self.assertNotIn('TAURI_SIGNING_PRIVATE_KEY', env)
            self.assertNotIn('RUSTFLAGS', env)
            self.assertNotIn('TAURI_BUNDLER_DMG_IGNORE_CI', env)
            self.assertEqual(env['CI'], 'true')
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

    def test_wrong_native_host_fails_before_build(self):
        with patch.object(BUILD.platform, 'system', return_value='Darwin'):
            with self.assertRaises(BUILD.BuildError): BUILD.check_host(target_for('windows-x86_64'))
        with patch.object(BUILD, 'git', return_value=' M synthetic-file'):
            with self.assertRaises(BUILD.BuildError): BUILD.candidate_source()


if __name__ == '__main__': unittest.main()
