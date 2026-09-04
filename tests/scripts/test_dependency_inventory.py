import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[2] / 'scripts/dependency_inventory.py'
SPEC = importlib.util.spec_from_file_location('dependency_inventory', SCRIPT)
INVENTORY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INVENTORY)


def metadata():
    return {'packages': [
        {'id': 'path+file:///synthetic/private/app#0.2.9', 'name': 'synthetic-app', 'version': '0.2.9', 'source': None, 'license': 'MIT'},
        {'id': 'registry-private-id', 'name': 'synthetic-lib', 'version': '1.2.3', 'source': 'registry+https://synthetic.invalid/private', 'license': '(MIT OR Apache-2.0) AND Unicode-3.0'},
        {'id': 'other-platform-id', 'name': 'other-platform', 'version': '1.0.0', 'source': None, 'license': 'MIT'}],
        'workspace_members': ['path+file:///synthetic/private/app#0.2.9'],
        'resolve': {'nodes': [{'id': 'registry-private-id'}, {'id': 'path+file:///synthetic/private/app#0.2.9'}]}}


def lock():
    return {'lockfileVersion': 3, 'packages': {
        '': {'name': 'synthetic-app'},
        'node_modules/@synthetic/tool': {'version': '1.2.3', 'dev': True, 'license': 'Apache-2.0', 'resolved': 'https://registry.npmjs.org/@synthetic/tool/-/tool.tgz'},
        'node_modules/runtime-lib': {'version': '2.0.0', 'license': 'MIT', 'resolved': 'https://synthetic.invalid/private.tar.gz'},
    }}


class DependencyInventoryTests(unittest.TestCase):
    def test_target_graph_and_node_dev_scopes_are_explicit_and_deterministic(self):
        report = INVENTORY.from_metadata(metadata(), lock())
        self.assertEqual(report['counts'], {'cargo': 2, 'npm': 2, 'npm_dev_only': 1, 'license_review_pending': 0})
        self.assertEqual(report['scope']['cargo_metadata_packages_outside_graph'], 1)
        self.assertEqual(report['review_status'], 'review_pending')
        self.assertEqual([row['name'] for row in report['packages']], ['synthetic-app', 'synthetic-lib', '@synthetic/tool', 'runtime-lib'])
        self.assertEqual(report['packages'][0]['source_category'], 'workspace')
        self.assertTrue(report['packages'][2]['dev_only'])
        self.assertFalse(report['packages'][3]['dev_only'])
        shuffled = metadata(); shuffled['packages'].reverse(); shuffled['resolve']['nodes'].reverse()
        self.assertEqual(report, INVENTORY.from_metadata(shuffled, lock()))

    def test_no_paths_contacts_urls_or_raw_license_file_text_escape(self):
        cargo = metadata()
        package = cargo['packages'][1]
        package.update({'license': 'SEE LICENSE IN /synthetic/private/license.txt',
                        'license_file': '/synthetic/private/license.txt', 'manifest_path': '/synthetic/private/Cargo.toml',
                        'authors': ['synthetic-person@example.invalid'], 'repository': 'https://synthetic.invalid/repo'})
        npm = lock()
        npm['packages']['node_modules/runtime-lib']['license'] = 'LicenseRef-SYNTHETIC_PRIVATE_MARKER'
        with patch('builtins.open', side_effect=AssertionError('no file reads')):
            report = INVENTORY.from_metadata(cargo, npm)
        text = json.dumps(report)
        for marker in ['synthetic/private', 'example.invalid', 'https:', 'registry-private-id', 'SYNTHETIC_PRIVATE_MARKER', 'license.txt']:
            self.assertNotIn(marker, text)
        self.assertEqual(report['counts']['license_review_pending'], 2)
        self.assertEqual(report['packages'][1]['license_review_reason'], 'license_file_not_inspected')
        self.assertIsNone(report['packages'][1]['license'])

    def test_missing_license_and_linked_version_remain_reviewable(self):
        cargo = metadata(); cargo['packages'][0].pop('license')
        npm = lock(); npm['packages']['node_modules/local-tool'] = {'link': True, 'resolved': '/synthetic/private/local-tool'}
        report = INVENTORY.from_metadata(cargo, npm)
        self.assertEqual(report['packages'][0]['license_review_reason'], 'missing_expression')
        linked = next(row for row in report['packages'] if row['name'] == 'local-tool')
        self.assertIsNone(linked['version'])
        self.assertEqual(linked['source_category'], 'path')
        self.assertEqual(linked['license_review'], 'pending')

    def test_spdx_expression_structure_and_unknown_ids_fail_closed(self):
        for value in ['MIT', '(MIT OR Apache-2.0) AND Unicode-DFS-2016', 'Apache-2.0 WITH LLVM-exception']:
            self.assertEqual(INVENTORY.license_expression(value), value)
        for value in ['MIT OR', '(MIT', 'MIT Apache-2.0', 'MIT WITH unknown-exception', 'LicenseRef-private',
                      'MIT; /private/path', 'https://synthetic.invalid/license', {'type': 'MIT'}, '(' * 20 + 'MIT' + ')' * 20]:
            self.assertIsNone(INVENTORY.license_expression(value))

    def test_incomplete_graph_or_invalid_identities_fail_without_echoing_metadata(self):
        variants = []
        cargo = metadata(); cargo['resolve'] = None; variants.append(cargo)
        cargo = metadata(); cargo['packages'].pop(1); variants.append(cargo)
        cargo = metadata(); cargo['resolve']['nodes'][0]['dependencies'] = ['missing-package']; variants.append(cargo)
        cargo = metadata(); cargo['packages'][0]['name'] = '/synthetic/private/invalid'; variants.append(cargo)
        cargo = metadata(); cargo['packages'][0]['version'] = 'https://synthetic.invalid/private'; variants.append(cargo)
        for cargo in variants:
            with self.assertRaises(INVENTORY.InventoryError) as raised:
                INVENTORY.from_metadata(cargo, lock())
            self.assertNotIn('synthetic', str(raised.exception))
        npm = lock(); npm['lockfileVersion'] = 1
        with self.assertRaises(INVENTORY.InventoryError): INVENTORY.from_metadata(metadata(), npm)

    def test_limits_and_invalid_node_scope_are_explicit_errors(self):
        with patch.object(INVENTORY, 'MAX_PACKAGES', 2):
            with self.assertRaises(INVENTORY.InventoryError): INVENTORY.from_metadata(metadata(), lock())
        npm = lock(); npm['packages']['node_modules/runtime-lib']['dev'] = 'true'
        with self.assertRaises(INVENTORY.InventoryError): INVENTORY.from_metadata(metadata(), npm)


if __name__ == '__main__':
    unittest.main()
