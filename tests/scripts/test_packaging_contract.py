import copy
import importlib.util
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / 'scripts/packaging_contract.py'
SPEC = importlib.util.spec_from_file_location('packaging_contract', SCRIPT)
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)


class ContractTests(unittest.TestCase):
    def test_declared_artifacts_and_target_lookup(self):
        matrix = CONTRACT.load_matrix()
        self.assertEqual(len(CONTRACT.artifact_names('0.2.9', matrix)), 7)
        self.assertEqual(CONTRACT.target_for('windows-x86_64')['target'], 'x86_64-pc-windows-msvc')
        with self.assertRaises(ValueError):
            CONTRACT.target_for('aarch64-pc-windows-msvc')
        with self.assertRaises(ValueError):
            CONTRACT.artifact_names('../candidate')

    def test_missing_duplicate_unsafe_and_false_device_claims_fail(self):
        base = CONTRACT.load_matrix()
        variants = []
        data = copy.deepcopy(base); data['targets'].pop(); variants.append(data)
        data = copy.deepcopy(base); data['targets'][1] = copy.deepcopy(data['targets'][0]); variants.append(data)
        data = copy.deepcopy(base); data['targets'][0]['artifacts'][0]['filename'] = '../escape.dmg'; variants.append(data)
        data = copy.deepcopy(base); data['targets'][0]['status'] = 'device-validated'; variants.append(data)
        data = copy.deepcopy(base); data['targets'][0]['bundle_config'] = '../outside.json'; variants.append(data)
        data = copy.deepcopy(base); data['targets'][3]['artifacts'].pop(); variants.append(data)
        data = copy.deepcopy(base); data['targets'][0]['bundle_targets'] = ['updater']; variants.append(data)
        with TemporaryDirectory(prefix='cloud-burrito-contract-') as directory:
            path = Path(directory) / 'matrix.json'
            for data in variants:
                path.write_text(json.dumps(data))
                with self.assertRaises(ValueError):
                    CONTRACT.load_matrix(path)


if __name__ == '__main__':
    unittest.main()
