"""Static guard for the deliberately inactive candidate example; no CI execution.

This is a small contract check, not a YAML parser or a claim that runner/action
semantics have been exercised. The future activation change needs real review.
"""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
TEMPLATE = ROOT / 'packaging/native-candidate.yml.example'


class InactiveCandidateTemplateTests(unittest.TestCase):
    def test_activation_and_dependency_graph_remain_closed(self):
        source = TEMPLATE.read_text(encoding='utf-8')
        jobs = dict(re.findall(r'^  ([a-z][a-z-]+):\n(.*?)(?=^  [a-z][a-z-]+:\n|\Z)',
                               source.split('\njobs:\n', 1)[1], re.M | re.S))
        self.assertEqual(set(jobs), {'activation-prerequisite', 'validate-source',
                                    'build-native', 'inspect-artifacts', 'assemble-candidate'})
        self.assertIn('\n          exit 1\n', jobs['activation-prerequisite'])
        for child, parent in [('validate-source', 'activation-prerequisite'),
                              ('build-native', 'validate-source'),
                              ('inspect-artifacts', 'build-native'),
                              ('assemble-candidate', 'inspect-artifacts')]:
            self.assertRegex(jobs[child], r'(?m)^    needs: ' + parent + '$')
        self.assertNotRegex(source, r'(?m)^\s*(?:continue-on-error|restore-keys):')
        self.assertNotIn('always()', source)
        self.assertRegex(source, r'(?m)^permissions:\n  contents: read$')
        self.assertNotRegex(source, r'(?m)^\s*(?:push|pull_request|schedule|secrets|id-token):')
        for active in (ROOT / '.github/workflows').glob('*'):
            self.assertNotEqual(active.read_bytes(), TEMPLATE.read_bytes(),
                                'The inactive example must not be installed as a workflow')

    def test_pins_matrix_and_complete_assembly_match_repository(self):
        import json
        source = TEMPLATE.read_text(encoding='utf-8')
        pins = set(re.findall(r'uses: ([\w/-]+@[0-9a-f]{40})', source))
        existing = '\n'.join(path.read_text(encoding='utf-8')
                             for path in (ROOT / '.github/workflows').glob('*.yml'))
        self.assertTrue(pins)
        self.assertEqual(len(pins), 4)
        self.assertTrue(pins <= set(re.findall(r'uses: ([\w/-]+@[0-9a-f]{40})', existing)))
        self.assertEqual(len(re.findall(r'uses:', source)),
                         len(re.findall(r'uses: [\w/-]+@[0-9a-f]{40}', source)))
        matrix = json.loads((ROOT / 'packaging/targets.json').read_text(encoding='utf-8'))
        targets = re.findall(r'^            target: (\S+)$', source, re.M)
        self.assertCountEqual(targets, [row['target'] for row in matrix['targets']])
        inputs = re.findall(r'--input-dir "\$INPUTS/p4-build-([^"]+)"', source)
        self.assertCountEqual(inputs, [row['id'] for row in matrix['targets']])
        self.assertEqual(sum(len(row['artifacts']) for row in matrix['targets']), 7)
        self.assertIn('python3 scripts/assemble_candidate.py verify "$OUTPUT"', source)
        for required in ['matrix.target', 'rust-toolchain.toml', 'scripts/tool-versions.env',
                         'src-tauri/Cargo.lock', 'package-lock.json', 'P4_RUNNER_IMAGE_DIGEST',
                         'P4_HELPER_INVENTORY_DIGEST']:
            self.assertIn(required, source)


if __name__ == '__main__':
    unittest.main()
