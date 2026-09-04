"""Synthetic before/after helper bytes only; never execute a native helper."""
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
spec = importlib.util.spec_from_file_location('helper_provenance', ROOT / 'scripts/check-native-helper-cache.py')
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class HelperProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.target = self.root / 'target'
        self.cache = self.target / '.tauri'
        self.cache.mkdir(parents=True)
        self.inventory_path = self.root / 'inventory.json'
        self.inventory = {'schema_version': 1, 'tauri_cli': '2.11.4', 'tauri_bundler': '2.9.4',
                          'target': 'x86_64-unknown-linux-gnu', 'files': []}
        for name in sorted(helper.APPIMAGE_REQUIRED):
            value = ('synthetic helper: ' + name).encode()
            (self.cache / name).write_bytes(value)
            self.inventory['files'].append({'path': name, 'size': len(value), 'sha256': hashlib.sha256(value).hexdigest()})
        self.inventory_path.write_text(json.dumps(self.inventory))

    def preflight(self):
        return helper.verify_cache('ubuntu-x86_64', self.target, self.inventory_path)

    def transform(self):
        path = self.cache / helper.LINUXDEPLOY
        value = path.read_bytes()
        path.write_bytes(value[:8] + b'\0\0\0' + value[11:])

    def test_preflight_computes_exact_future_digest_without_writing(self):
        before = {path.name: path.read_bytes() for path in self.cache.iterdir()}
        report = self.preflight()
        patch_info = report['expected_mutations'][0]
        value = before[helper.LINUXDEPLOY]
        self.assertEqual(patch_info['after_sha256'], hashlib.sha256(value[:8] + b'\0\0\0' + value[11:]).hexdigest())
        self.assertEqual(patch_info['original_bytes_8_10'], value[8:11].hex())
        self.assertEqual(report['inventory_sha256'], hashlib.sha256(self.inventory_path.read_bytes()).hexdigest())
        self.assertEqual(before, {path.name: path.read_bytes() for path in self.cache.iterdir()})

    def test_post_build_captures_actual_hashes_for_every_required_helper(self):
        report = self.preflight()
        self.transform()
        post = helper.verify_after(self.target, self.inventory_path, report)
        self.assertEqual(post['phase'], 'after-build')
        self.assertEqual(len(post['observed_files']), 5)
        self.assertFalse(post['network_enforced'])
        self.assertFalse(post['executed'])
        for entry in post['observed_files']:
            self.assertEqual(entry['sha256'], hashlib.sha256((self.cache / entry['path']).read_bytes()).hexdigest())

    def test_unchanged_nonzero_linuxdeploy_is_rejected_after_actual_build(self):
        report = self.preflight()
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, report)

    def test_already_zero_reviewed_input_can_remain_unchanged(self):
        self.transform()
        value = (self.cache / helper.LINUXDEPLOY).read_bytes()
        for entry in self.inventory['files']:
            if entry['path'] == helper.LINUXDEPLOY:
                entry['sha256'] = hashlib.sha256(value).hexdigest()
        self.inventory_path.write_text(json.dumps(self.inventory))
        report = self.preflight()
        mutation = report['expected_mutations'][0]
        self.assertEqual(mutation['before_sha256'], mutation['after_sha256'])
        self.assertEqual(helper.verify_after(self.target, self.inventory_path, report)['files'], 5)

    def test_changed_other_helper_or_changed_inventory_is_rejected(self):
        report = self.preflight()
        self.transform()
        path = self.cache / 'linuxdeploy-plugin-gtk.sh'
        original = path.read_bytes()
        path.write_bytes(b'X' * len(original))
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, report)
        path.write_bytes(original)
        self.inventory_path.write_text(json.dumps(self.inventory, indent=2))
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, report)

    def test_forged_expected_digest_cannot_allow_changes_outside_three_bytes(self):
        report = self.preflight()
        self.transform()
        path = self.cache / helper.LINUXDEPLOY
        value = bytearray(path.read_bytes())
        value[-1] ^= 1
        path.write_bytes(value)
        forged = copy.deepcopy(report)
        forged['expected_mutations'][0]['after_sha256'] = hashlib.sha256(value).hexdigest()
        with self.assertRaisesRegex(ValueError, 'outside'):
            helper.verify_after(self.target, self.inventory_path, forged)

    def test_wrong_saved_original_bytes_and_extra_transforms_are_rejected(self):
        report = self.preflight()
        self.transform()
        forged = copy.deepcopy(report)
        forged['expected_mutations'][0]['original_bytes_8_10'] = 'ffffff'
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, forged)
        forged = copy.deepcopy(report)
        forged['expected_mutations'].append(copy.deepcopy(forged['expected_mutations'][0]))
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, forged)

    def test_wrong_target_or_post_report_cannot_replace_preflight(self):
        report = self.preflight()
        self.transform()
        post = helper.verify_after(self.target, self.inventory_path, report)
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, post)
        report['target'] = 'x86_64-pc-windows-msvc'
        with self.assertRaises(ValueError):
            helper.verify_after(self.target, self.inventory_path, report)

    def test_windows_post_build_allows_no_helper_changes(self):
        inventory = {**self.inventory, 'target': 'x86_64-pc-windows-msvc', 'files': []}
        for name in sorted(helper.NSIS_REQUIRED):
            path = self.cache / name
            path.parent.mkdir(parents=True, exist_ok=True)
            value = ('synthetic NSIS helper: ' + name).encode()
            path.write_bytes(value)
            inventory['files'].append({'path': name, 'size': len(value), 'sha256': hashlib.sha256(value).hexdigest()})
        self.inventory_path.write_text(json.dumps(inventory))
        fixture_sha1 = hashlib.sha1((self.cache / helper.PLUGIN_PATH).read_bytes()).hexdigest()
        with patch.object(helper, 'PLUGIN_SHA1', fixture_sha1):
            report = helper.verify_cache('windows-x86_64', self.target, self.inventory_path)
            self.assertEqual(report['expected_mutations'], [])
            post = helper.verify_after(self.target, self.inventory_path, report)
            self.assertEqual(len(post['observed_files']), len(helper.NSIS_REQUIRED))
            (self.cache / 'NSIS/makensis.exe').write_bytes(b'changed synthetic NSIS executable')
            with self.assertRaises(ValueError):
                helper.verify_after(self.target, self.inventory_path, report)


if __name__ == '__main__':
    unittest.main()
