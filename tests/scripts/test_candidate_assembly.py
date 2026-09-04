"""Synthetic recorded-inspection fixtures; no native payload is executed.

These deliberately tiny text artifacts exercise consistency, not native format
inspection or provenance. The separate inspector suite covers actual formats.
"""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import assemble_candidate as ASSEMBLY


def sha(data):
    return hashlib.sha256(data).hexdigest()


class CandidateAssemblyTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='burrito assembly-é-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.matrix = ASSEMBLY.load_matrix()
        self.version = '0.2.9'
        required = ['src-tauri/Cargo.lock', 'package-lock.json', 'rust-toolchain.toml',
                    'scripts/tool-versions.env', 'packaging/targets.json', 'src-tauri/tauri.conf.json']
        required += [row['bundle_config'] for row in self.matrix['targets']]
        self.identity = {'source_commit': 'a' * 40, 'source_tree': 'b' * 40,
                         'inputs_sha256': {name: sha(('synthetic input ' + name).encode()) for name in required}}
        self.identity['candidate_input_sha256'] = sha(json.dumps(self.identity, sort_keys=True, separators=(',', ':')).encode())
        self.inputs, self.manifests = [], []
        for row in self.matrix['targets']:
            folder = self.root / row['id']
            folder.mkdir()
            self.inputs.append(folder)
            artifacts = []
            for declared in row['artifacts']:
                name = declared['filename'].format(version=self.version)
                data = ('synthetic artifact bytes: ' + name).encode()
                (folder / name).write_bytes(data)
                details = self.details(row, declared['format'])
                artifacts.append({'filename': name, 'format': declared['format'], 'size_bytes': len(data),
                                  'sha256': sha(data), 'payload_sha256': sha(row['target'].encode()), 'inspection': details})
            manifest = {'schema_version': 1, 'matrix_revision': self.matrix['matrix_revision'],
                        'target': row['target'], 'version': self.version, **copy.deepcopy(self.identity),
                        'source_clean': True, 'source_export': 'immutable tracked Git tree; no local ignored inputs',
                        'tool_versions': {'synthetic': True},
                        'inspection': {'schema_version': 1, 'target': row['target'], 'version': self.version,
                                       'status': 'passed', 'artifacts': artifacts}}
            path = folder / f"build-manifest-{row['id']}.json"
            path.write_text(json.dumps(manifest, indent=2) + '\n')
            self.manifests.append(path)
        self.output = self.root / 'complete'

    def details(self, row, kind):
        if row['platform'] == 'macos':
            value = {'architecture': row['binary_arch'], 'identifier': 'app.cloudburrito.desktop',
                     'minimum_os': '13.0', 'publisher_identity': False}
            if kind == 'dmg':
                value.update(container_verified=True, matches_app_zip=True)
            return value
        if kind == 'nsis':
            return {'script_install_mode': 'currentUser', 'custom_hooks': False, 'generated_script_sha256': 'c' * 64,
                    'executable': {'machine': 'AMD64', 'publisher_certificate': False},
                    'installer': {'machine': '0x14c', 'publisher_certificate': False}}
        value = {'executable': {'machine': 'x86-64', 'maximum_glibc_reference': '2.35'},
                 'executable_sha256': sha(b'synthetic shared Linux application executable')}
        if kind == 'deb':
            value.update(control={'Architecture': 'amd64', 'Version': self.version}, maintainer_hooks=False)
        else:
            value.update(runtime={'machine': 'x86-64'}, extraction='external archive tool')
        return value

    def assemble(self):
        return ASSEMBLY.assemble(self.inputs, self.output, self.matrix)

    def edit_manifest(self, index, update):
        path = self.manifests[index]
        data = json.loads(path.read_text())
        update(data)
        path.write_text(json.dumps(data))

    def reject(self):
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.assemble()
        self.assertFalse(self.output.exists())

    def test_complete_set_preserves_manifests_and_has_exact_checksums(self):
        before = {path.name: path.read_bytes() for path in self.manifests}
        result = self.assemble()
        expected = set(before) | set(ASSEMBLY.artifact_names(self.version, self.matrix)) | {ASSEMBLY.MANIFEST, ASSEMBLY.SUMS}
        self.assertEqual({path.name for path in self.output.iterdir()}, expected)
        self.assertEqual(len(expected), 13)
        for name, data in before.items():
            self.assertEqual((self.output / name).read_bytes(), data)
        sums = (self.output / ASSEMBLY.SUMS).read_text().splitlines()
        self.assertEqual(len(sums), 12)
        for line in sums:
            digest, name = line.split('  ')
            self.assertEqual(digest, sha((self.output / name).read_bytes()))
        self.assertEqual(ASSEMBLY.verify(self.output, self.matrix), result)

    def test_prior_matching_source_and_unsigned_limitation_are_explicit(self):
        result = self.assemble()
        self.assertEqual(result['source_commit'], 'a' * 40)
        self.assertIn('not publisher authenticity', ' '.join(result['limitations']))
        self.assertIn('does not inspect the current checkout', ' '.join(result['limitations']))
        self.assertIn('NSIS script remains', ' '.join(result['limitations']))
        mtimes = {path.name: path.stat().st_mtime_ns for path in self.output.iterdir()}
        ASSEMBLY.verify(self.output, self.matrix)
        self.assertEqual(mtimes, {path.name: path.stat().st_mtime_ns for path in self.output.iterdir()})

    def test_missing_target_directory_is_rejected(self):
        self.inputs.pop()
        self.reject()

    def test_duplicate_target_directory_is_rejected(self):
        self.inputs[-1] = self.inputs[0]
        self.reject()

    def test_missing_manifest_is_rejected(self):
        self.manifests[-1].unlink()
        self.reject()

    def test_missing_artifact_is_rejected(self):
        artifact = next(self.inputs[0].glob('*.dmg'))
        artifact.unlink()
        self.reject()

    def test_extra_input_and_extra_subdirectory_are_rejected(self):
        extra = self.inputs[0] / 'unreviewed.txt'
        extra.write_text('synthetic')
        self.reject()
        extra.unlink()
        extra.mkdir()
        self.reject()

    def test_mixed_source_identity_even_with_valid_input_digest_is_rejected(self):
        def change(data):
            data['source_commit'] = 'd' * 40
            identity = {key: data[key] for key in ('source_commit', 'source_tree', 'inputs_sha256')}
            data['candidate_input_sha256'] = sha(json.dumps(identity, sort_keys=True, separators=(',', ':')).encode())
        self.edit_manifest(1, change)
        self.reject()

    def test_invalid_input_digest_is_rejected(self):
        self.edit_manifest(0, lambda data: data.update(candidate_input_sha256='d' * 64))
        self.reject()

    def test_unsafe_build_input_path_is_rejected(self):
        self.edit_manifest(0, lambda data: data['inputs_sha256'].update({'../outside': 'd' * 64}))
        self.reject()

    def test_mixed_version_matrix_or_target_is_rejected(self):
        path = self.manifests[1]
        original = path.read_bytes()
        for field, value in [('version', '0.2.10'), ('matrix_revision', 'unreviewed'), ('target', 'x86_64-unknown-linux-gnu')]:
            with self.subTest(field=field):
                path.write_bytes(original)
                self.edit_manifest(1, lambda data: data.update({field: value}))
                self.reject()

    def test_uninspected_or_placeholder_artifacts_are_rejected(self):
        path = self.manifests[0]
        original = path.read_bytes()
        for update in (lambda d: d['inspection'].update(status='pending'),
                       lambda d: d['inspection']['artifacts'][0].update(inspection={}),
                       lambda d: d['inspection'].update(artifacts=[]),
                       lambda d: d.update(source_clean=False)):
            path.write_bytes(original)
            self.edit_manifest(0, update)
            self.reject()

    def test_wrong_format_and_duplicate_inspection_item_are_rejected(self):
        path = self.manifests[0]
        original = path.read_bytes()
        self.edit_manifest(0, lambda data: data['inspection']['artifacts'][0].update(format='nsis'))
        self.reject()
        path.write_bytes(original)
        self.edit_manifest(0, lambda data: data['inspection']['artifacts'].__setitem__(1, copy.deepcopy(data['inspection']['artifacts'][0])))
        self.reject()

    def test_path_escape_in_artifact_reference_never_reads_outside(self):
        outside = self.root / 'untouched'
        outside.write_text('synthetic outside file')
        self.edit_manifest(0, lambda data: data['inspection']['artifacts'][0].update(filename='../untouched'))
        self.reject()
        self.assertEqual(outside.read_text(), 'synthetic outside file')

    def test_modified_artifact_and_false_size_are_rejected(self):
        artifact = next(self.inputs[0].glob('*.dmg'))
        old = artifact.read_bytes()
        artifact.write_bytes(b'X' * len(old))
        self.reject()
        artifact.write_bytes(old)
        self.edit_manifest(0, lambda data: data['inspection']['artifacts'][0].update(size_bytes=True))
        self.reject()

    def test_mismatched_mac_payload_claim_is_rejected(self):
        self.edit_manifest(0, lambda data: data['inspection']['artifacts'][0].update(payload_sha256='d' * 64))
        self.reject()

    def test_missing_or_mismatched_linux_executable_digest_is_rejected(self):
        path = self.manifests[3]
        original = path.read_bytes()
        for value in (None, 'e' * 64):
            with self.subTest(value=value):
                path.write_bytes(original)
                self.edit_manifest(3, lambda data: data['inspection']['artifacts'][0]['inspection'].update(executable_sha256=value))
                self.reject()

    def test_missing_native_detail_or_signed_windows_claim_is_rejected(self):
        self.edit_manifest(2, lambda data: data['inspection']['artifacts'][0]['inspection'].update(generated_script_sha256=None))
        self.reject()

    def test_symlink_artifact_and_redirected_input_directory_are_rejected(self):
        artifact = next(self.inputs[0].glob('*.dmg'))
        outside = self.root / 'outside'
        artifact.rename(outside)
        try:
            artifact.symlink_to(outside)
        except OSError:
            self.skipTest('Fixture symlinks unavailable on this host')
        self.reject()
        artifact.unlink()
        outside.rename(artifact)
        alias = self.root / 'alias'
        alias.symlink_to(self.inputs[0], target_is_directory=True)
        self.inputs[0] = alias
        self.reject()

    def test_duplicate_json_field_is_rejected(self):
        path = self.manifests[0]
        text = path.read_text()
        path.write_text(text.replace('{', '{"version":"0.2.9",', 1))
        self.reject()

    def test_existing_output_is_never_modified(self):
        self.output.mkdir()
        existing = self.output / 'keep.txt'
        existing.write_text('keep existing bytes')
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.assemble()
        self.assertEqual(existing.read_text(), 'keep existing bytes')
        self.assertEqual(len(list(self.output.iterdir())), 1)

    def test_output_nested_in_input_is_rejected(self):
        self.output = self.inputs[0] / 'nested'
        self.reject()

    def test_changed_input_during_copy_never_gets_a_complete_checksum_file(self):
        original = ASSEMBLY.fingerprint
        changed = False
        def change(path, maximum=ASSEMBLY.MAX_ARTIFACT, destination=None):
            nonlocal changed
            if destination is not None and not changed:
                changed = True
                path.write_bytes(b'X' * path.stat().st_size)
            return original(path, maximum, destination)
        with patch.object(ASSEMBLY, 'fingerprint', side_effect=change), self.assertRaises(ASSEMBLY.AssemblyError):
            self.assemble()
        self.assertFalse((self.output / ASSEMBLY.SUMS).exists())

    def test_verifier_rejects_modified_artifact_and_extra_output(self):
        self.assemble()
        extra = self.output / 'unexpected'
        extra.write_text('synthetic')
        with self.assertRaises(ASSEMBLY.AssemblyError):
            ASSEMBLY.verify(self.output, self.matrix)
        extra.unlink()
        artifact = next(self.output.glob('*.dmg'))
        artifact.write_bytes(b'X' * artifact.stat().st_size)
        with self.assertRaises(ASSEMBLY.AssemblyError):
            ASSEMBLY.verify(self.output, self.matrix)

    def test_verifier_rejects_missing_duplicate_and_edited_checksum_lines(self):
        self.assemble()
        path = self.output / ASSEMBLY.SUMS
        original = path.read_text()
        variants = ['\n'.join(original.splitlines()[1:]) + '\n', original + original.splitlines()[0] + '\n',
                    '0' * 64 + original[64:]]
        for text in variants:
            with self.subTest(text=text[:20]):
                path.write_text(text)
                with self.assertRaises(ASSEMBLY.AssemblyError):
                    ASSEMBLY.verify(self.output, self.matrix)

    def test_verifier_rejects_altered_manifest_even_if_checksum_is_updated(self):
        self.assemble()
        path = self.output / ASSEMBLY.MANIFEST
        summary = json.loads(path.read_text())
        summary['source_commit'] = 'e' * 40
        path.write_text(json.dumps(summary))
        (self.output / ASSEMBLY.SUMS).write_text(ASSEMBLY.checksums(summary, ASSEMBLY.fingerprint(path)))
        with self.assertRaises(ASSEMBLY.AssemblyError):
            ASSEMBLY.verify(self.output, self.matrix)


if __name__ == '__main__':
    unittest.main()
