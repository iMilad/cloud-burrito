"""Static P4-05 policy checks; no package is built, installed or executed."""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from packaging_contract import load_matrix, target_for
from platform_config import check_platform, validate_config


class LinuxPackagingPolicy(unittest.TestCase):
    def setUp(self):
        self.row = target_for('ubuntu-x86_64', load_matrix(ROOT / 'packaging/targets.json'))
        self.config = json.loads((ROOT / self.row['bundle_config']).read_text())

    def reject(self, mutate):
        config = copy.deepcopy(self.config)
        mutate(config)
        with self.assertRaises(ValueError):
            validate_config(config, self.row, ROOT)

    def test_current_overlay_matches_pinned_shape_and_matrix(self):
        self.assertEqual(check_platform('linux', ROOT)['id'], self.row['id'])

    def test_both_declared_package_formats_are_required(self):
        self.reject(lambda c: c['bundle'].update(targets=['deb']))
        self.reject(lambda c: c['bundle'].update(targets=['deb', 'rpm', 'appimage']))

    def test_dependencies_remain_generated_without_install_hooks(self):
        for key, value in [('depends', ['libwebkit2gtk-4.1-dev']), ('postInstallScript', 'install.sh'), ('postRemoveScript', 'remove.sh')]:
            with self.subTest(key=key):
                self.reject(lambda c: c['bundle']['linux']['deb'].update({key: value}))

    def test_no_invented_maintainer_or_extra_payload(self):
        self.reject(lambda c: c['bundle']['linux']['deb'].update(maintainer='Invented Maintainer'))
        self.reject(lambda c: c['bundle']['linux']['deb'].update(files={'/etc/example': 'example'}))

    def test_no_media_framework_updater_or_global_helper_cache(self):
        self.reject(lambda c: c['bundle']['linux']['appimage'].update(bundleMediaFramework=True))
        self.reject(lambda c: c['bundle'].update(createUpdaterArtifacts=True))
        self.reject(lambda c: c['bundle'].update(useLocalToolsDir=False))

    def test_unknown_keys_and_boolean_coercion_fail(self):
        self.reject(lambda c: c['bundle']['linux'].update(appImage={'bundleMediaFramework': False}))
        self.reject(lambda c: c['bundle']['linux']['appimage'].update(bundleMediaFramework=0))


if __name__ == '__main__':
    unittest.main()
