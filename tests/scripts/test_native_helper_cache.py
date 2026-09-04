"""Synthetic local cache fixtures; no real helper is acquired or executed."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('helper_cache', ROOT / 'scripts/check-native-helper-cache.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class NativeHelperCache(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.target = self.base / 'target'
        self.cache = self.target / '.tauri'
        self.cache.mkdir(parents=True)
        self.inventory_path = self.base / 'reviewed.json'

    def make_inventory(self, platform='linux'):
        names = helper.APPIMAGE_REQUIRED if platform == 'linux' else helper.NSIS_REQUIRED
        entries = []
        for name in sorted(names):
            path = self.cache / name
            path.parent.mkdir(parents=True, exist_ok=True)
            content = ('synthetic fixture: ' + name).encode()
            path.write_bytes(content)
            path.chmod(0o700)
            entries.append({'path': name, 'size': len(content), 'sha256': hashlib.sha256(content).hexdigest()})
        self.inventory = {'schema_version': 1, 'tauri_cli': '2.11.4', 'tauri_bundler': '2.9.4',
                          'target': 'x86_64-unknown-linux-gnu' if platform == 'linux' else 'x86_64-pc-windows-msvc', 'files': entries}
        self.save()

    def save(self):
        self.inventory_path.write_text(json.dumps(self.inventory))

    def verify(self):
        return helper.verify_cache(self.inventory['target'], self.target, self.inventory_path)

    def test_linux_complete_inventory_is_read_only_and_honest(self):
        self.make_inventory()
        before = {p.name: p.read_bytes() for p in self.cache.iterdir()}
        report = self.verify()
        self.assertEqual(report['files'], 5)
        self.assertFalse(report['network_enforced'])
        self.assertFalse(report['executed'])
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.cache.iterdir()})

    def test_missing_gstreamer_or_optional_plugin_blocks_before_download(self):
        self.make_inventory()
        for name in ('linuxdeploy-plugin-gstreamer.sh', 'linuxdeploy-plugin-appimage.AppImage'):
            original = copy.deepcopy(self.inventory)
            self.inventory['files'] = [e for e in self.inventory['files'] if e['path'] != name]
            self.save()
            with self.assertRaises(ValueError):
                self.verify()
            self.inventory = original

    def test_changed_hash_and_size_are_rejected(self):
        self.make_inventory()
        path = self.cache / self.inventory['files'][0]['path']
        original = path.read_bytes()
        for replacement in (b'X' * len(original), original + b'X'):
            path.write_bytes(replacement)
            with self.assertRaises(ValueError):
                self.verify()

    def test_cache_route_and_inventory_target_must_be_explicit(self):
        self.make_inventory()
        with self.assertRaises(ValueError):
            helper.verify_cache(self.inventory['target'], Path('relative-target'), self.inventory_path)
        with self.assertRaises(ValueError):
            helper.verify_cache('windows-x86_64', self.target, self.inventory_path)
        with self.assertRaises(ValueError):
            helper.verify_cache('macos-aarch64', self.target, self.inventory_path)

    def test_versions_and_empty_inventory_are_rejected(self):
        self.make_inventory()
        for field, value in [('tauri_cli', '2.11.5'), ('tauri_bundler', '2.9.5'), ('files', [])]:
            original = copy.deepcopy(self.inventory)
            self.inventory[field] = value
            self.save()
            with self.assertRaises(ValueError):
                self.verify()
            self.inventory = original

    def test_duplicate_and_invalid_sha_and_noninteger_size_are_rejected(self):
        self.make_inventory()
        original = copy.deepcopy(self.inventory)
        self.inventory['files'].append(copy.deepcopy(self.inventory['files'][0]))
        self.save()
        with self.assertRaises(ValueError):
            self.verify()
        for field, value in [('sha256', None), ('size', True), ('sha256', '0' * 64)]:
            self.inventory = copy.deepcopy(original)
            self.inventory['files'][0][field] = value
            self.save()
            with self.assertRaises(ValueError):
                self.verify()

    def test_unsafe_path_spellings_are_rejected(self):
        for name in ('../outside', '/absolute', 'NSIS/../outside', 'NSIS//a', 'NSIS/./a', 'C:/a', 'NSIS/a:stream', 'NSIS\\a', 'NSIS/a\n'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                helper.regular_path(self.cache, name)

    def test_symlinked_file_and_directory_are_rejected(self):
        self.make_inventory()
        name = self.inventory['files'][0]['path']
        path = self.cache / name
        destination = self.base / 'outside'
        path.rename(destination)
        try:
            path.symlink_to(destination)
        except OSError:
            self.skipTest('This host does not permit fixture symlinks')
        with self.assertRaises(ValueError):
            self.verify()
        alias = self.base / 'alias'
        alias.symlink_to(self.target, target_is_directory=True)
        with self.assertRaises(ValueError):
            helper.verify_cache(self.inventory['target'], alias, self.inventory_path)

    def test_nsis_plugin_upstream_hash_mismatch_would_download_and_is_rejected(self):
        self.make_inventory('windows')
        with self.assertRaisesRegex(ValueError, 'redownload'):
            self.verify()

    def test_nsis_fixture_tree_requires_every_file_to_be_reviewed(self):
        self.make_inventory('windows')
        fixture_sha1 = hashlib.sha1((self.cache / helper.PLUGIN_PATH).read_bytes()).hexdigest()
        # Only this synthetic test replaces the upstream hash, never the tool.
        with patch.object(helper, 'PLUGIN_SHA1', fixture_sha1):
            self.assertEqual(self.verify()['files'], len(helper.NSIS_REQUIRED))
            extra = self.cache / 'NSIS/Include/unreviewed.nsh'
            extra.write_text('synthetic unreviewed include')
            with self.assertRaisesRegex(ValueError, 'Every NSIS'):
                self.verify()


if __name__ == '__main__':
    unittest.main()
