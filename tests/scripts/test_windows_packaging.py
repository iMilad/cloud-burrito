"""Static P4-04 policy checks; no installer, network, Cargo or AWS invocation."""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from packaging_contract import load_matrix, target_for
from platform_config import check_platform, schema_accepts, validate_config


class WindowsPackagingPolicy(unittest.TestCase):
    def setUp(self):
        self.row = target_for('windows-x86_64', load_matrix(ROOT / 'packaging/targets.json'))
        self.config = json.loads((ROOT / self.row['bundle_config']).read_text())

    def reject(self, mutate):
        changed = copy.deepcopy(self.config)
        mutate(changed)
        with self.assertRaises(ValueError):
            validate_config(changed, self.row, ROOT)

    def test_current_overlay_matches_pinned_shape_and_matrix(self):
        self.assertEqual(check_platform('windows', ROOT)['id'], self.row['id'])

    def test_install_scope_cannot_request_elevation(self):
        for mode in ('perMachine', 'both', 'current-user'):
            with self.subTest(mode=mode):
                self.reject(lambda c: c['bundle']['windows']['nsis'].update(installMode=mode))

    def test_no_signing_identity_or_custom_signer(self):
        for field, value in [('certificateThumbprint', 'synthetic-thumbprint'), ('signCommand', 'sign.exe'), ('timestampUrl', 'https://example.invalid/timestamp')]:
            with self.subTest(field=field):
                self.reject(lambda c: c['bundle']['windows'].update({field: value}))

    def test_runtime_download_policy_is_explicit_and_camel_case(self):
        for mode in ('skip', 'embedBootstrapper', 'offlineInstaller', 'downloadbootstrapper'):
            with self.subTest(mode=mode):
                self.reject(lambda c: c['bundle']['windows'].update(webviewInstallMode={'type': mode, 'silent': True}))

    def test_cannot_add_hooks_or_override_security_configuration(self):
        self.reject(lambda c: c['bundle']['windows']['nsis'].update(installerHooks='hooks.nsh'))
        self.reject(lambda c: c.update(app={'security': {'csp': None}}))

    def test_no_msi_or_updater_or_global_tool_cache(self):
        self.reject(lambda c: c['bundle'].update(targets=['msi']))
        self.reject(lambda c: c['bundle'].update(createUpdaterArtifacts=True))
        self.reject(lambda c: c['bundle'].update(useLocalToolsDir=False))

    def test_scalar_type_errors_are_not_coerced(self):
        self.reject(lambda c: c['bundle']['windows'].update(allowDowngrades=0))
        self.reject(lambda c: c['bundle']['windows']['nsis'].update(languages='English'))

    def test_schema_preserves_tagged_runtime_alternatives(self):
        schema = json.loads((ROOT / 'packaging/tauri-platform-schema.json').read_text())
        definition = schema['definitions']['WebviewInstallMode']
        self.assertTrue(schema_accepts({'type': 'downloadBootstrapper', 'silent': True}, definition, schema['definitions']))
        self.assertFalse(schema_accepts({'type': 'notARealMode', 'silent': True}, definition, schema['definitions']))


if __name__ == '__main__':
    unittest.main()
