"""Static guard for the deliberately inactive candidate example; no CI execution.

This is a small contract check, not a YAML parser or a claim that runner/action
semantics have been exercised. The future activation change needs real review.
"""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
TEMPLATE = ROOT / 'packaging/native-candidate.yml.example'


def jobs_in(path):
    source = path.read_text(encoding='utf-8')
    return dict(re.findall(r'^  ([a-z][a-z-]+):\n(.*?)(?=^  [a-z][a-z-]+:\n|\Z)',
                          source.split('\njobs:\n', 1)[1], re.M | re.S))


class ActiveReleaseContractTests(unittest.TestCase):
    def test_secret_scan_cannot_skip_installation_history_or_failure(self):
        source = jobs_in(ROOT / '.github/workflows/ci.yml')['secrets']
        self.assertIn('fetch-depth: 0', source)
        self.assertIn('persist-credentials: false', source)
        self.assertIn('source scripts/tool-versions.env', source)
        self.assertIn('go install "github.com/zricethezav/gitleaks/v8@v${GITLEAKS_VERSION}"', source)
        self.assertIn('gitleaks" git .', source)
        self.assertIn('--config .gitleaks.toml --redact --no-banner --log-opts="--all"', source)
        self.assertLess(source.index('go install'), source.index('gitleaks" git .'))
        self.assertNotRegex(source, r'(?m)^\s*(?:if|continue-on-error):')
        self.assertNotIn('||', source)
        self.assertRegex((ROOT / 'scripts/tool-versions.env').read_text(),
                         r'(?m)^GITLEAKS_VERSION=\d+\.\d+\.\d+$')

    def test_active_gates_run_node_units_before_browser_tests(self):
        for filename, job in [('ci.yml', 'frontend'), ('release.yml', 'validate')]:
            with self.subTest(workflow=filename):
                source = jobs_in(ROOT / '.github/workflows' / filename)[job]
                self.assertIn('run: npm run test:unit', source)
                self.assertLess(source.index('run: npm ci'), source.index('run: npm run test:unit'))
                self.assertLess(source.index('run: npm run test:unit'), source.index('run: npm run test:frontend'))

    def test_cold_cache_locked_target_fetch_precedes_offline_build(self):
        source = jobs_in(ROOT / '.github/workflows/release.yml')['build-macos']
        provision = source.split('- name: Provision locked application dependencies\n', 1)[1].split('\n      - name:', 1)[0]
        self.assertIn('working-directory: src-tauri', provision)
        self.assertIn('run: cargo fetch --locked --target "${{ matrix.target }}"', provision)
        self.assertNotIn('if:', provision, 'A cold cache must not skip locked dependency provisioning')
        self.assertLess(source.index('cargo fetch --locked'), source.index('./scripts/build-release.sh'))
        self.assertIn('--offline', (ROOT / 'scripts/build_candidate.py').read_text())

    def test_release_stages_and_rechecks_exact_candidate_outputs_and_receipts(self):
        jobs = jobs_in(ROOT / '.github/workflows/release.yml')
        build, publish = jobs['build-macos'], jobs['publish']
        self.assertIn('--output "$CANDIDATE_OUTPUT"', build)
        self.assertIn('scripts/stage_release_assets.py stage', build)
        self.assertIn('--input-dir "$CANDIDATE_OUTPUT"', build)
        self.assertIn('--source-commit "$GITHUB_SHA" --version "$version"', build)
        self.assertNotIn('ditto ', build)
        self.assertNotIn('/release/bundle/', build)
        self.assertIn('path: release-assets/*', build)
        self.assertIn('scripts/stage_release_assets.py verify', publish)
        self.assertIn('--version "${TAG_NAME#app-v}" --write-combined', publish)
        self.assertLess(publish.index('scripts/stage_release_assets.py verify'), publish.index('gh release upload'))
        self.assertEqual(set(re.findall(r'^          - target: (\S+)', build, re.M)),
                         {'aarch64-apple-darwin', 'x86_64-apple-darwin'})
        self.assertIn('--draft', publish)


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
