"""Synthetic receipt consistency tests; no native build, execution or remote IO."""
from pathlib import Path
import shutil
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import assemble_candidate as ASSEMBLY
import stage_release_assets as RELEASE
import test_candidate_assembly as FIXTURES


class ReleaseAssetTests(unittest.TestCase):
    setUp = FIXTURES.CandidateAssemblyTests.setUp
    details = FIXTURES.CandidateAssemblyTests.details
    edit_manifest = FIXTURES.CandidateAssemblyTests.edit_manifest

    def stage(self, index=0, commit=None):
        row = self.matrix['targets'][index]
        output = self.root / ('stage-' + row['id'])
        RELEASE.stage(row['id'], self.inputs[index], output,
                      commit or self.identity['source_commit'], self.version)
        return output

    def transfer(self):
        output = self.root / 'transferred'
        output.mkdir()
        for index in [0, 1]:
            for path in self.stage(index).iterdir():
                shutil.copyfile(path, output / path.name)
        return output

    def test_staging_preserves_every_inspected_byte_and_build_receipt(self):
        output = self.stage()
        originals = {p.name: p.read_bytes() for p in self.inputs[0].iterdir()}
        self.assertEqual({p.name for p in output.iterdir()}, set(originals) | {'SHA256SUMS-aarch64.txt'})
        for name, data in originals.items():
            self.assertEqual((output / name).read_bytes(), data)
        lines = (output / 'SHA256SUMS-aarch64.txt').read_text().splitlines()
        self.assertEqual(len(lines), 3)
        self.assertTrue(any('build-manifest-macos-aarch64.json' in line for line in lines))

    def test_fan_in_verifies_two_targets_then_checksums_artifacts_and_receipts(self):
        output = self.transfer()
        result = RELEASE.verify(output, self.identity['source_commit'], self.version, write_combined=True)
        self.assertEqual(len(result['targets']), 2)
        sums = (output / RELEASE.SUMS).read_text().splitlines()
        self.assertEqual(len(sums), 6)
        for line in sums:
            value, name = line.split('  ')
            self.assertEqual(value, ASSEMBLY.fingerprint(output / name)['sha256'])
        self.assertEqual(RELEASE.verify(output, self.identity['source_commit'], self.version), result)
        with self.assertRaises(ASSEMBLY.AssemblyError):
            ASSEMBLY.verify(output)  # A macOS release must never claim all four targets.

    def test_wrong_source_uninspected_artifact_and_foreign_platform_fail(self):
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.stage(commit='e' * 40)
        self.assertFalse((self.root / 'stage-macos-aarch64').exists())
        self.edit_manifest(0, lambda item: item['inspection'].update(status='pending'))
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.stage()
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.stage(2)

    def test_modified_archive_is_rejected_before_staging(self):
        archive = next(self.inputs[0].glob('*.zip'))
        archive.write_bytes(b'repacked archive with changed metadata')
        with self.assertRaises(ASSEMBLY.AssemblyError):
            self.stage()
        self.assertFalse((self.root / 'stage-macos-aarch64').exists())

    def test_missing_target_receipt_checksum_or_extra_transfer_is_rejected(self):
        output = self.transfer()
        for name in ['build-manifest-macos-aarch64.json', 'SHA256SUMS-x86_64.txt']:
            path = output / name
            contents = path.read_bytes()
            path.unlink()
            with self.subTest(name=name), self.assertRaises(ASSEMBLY.AssemblyError):
                RELEASE.verify(output, self.identity['source_commit'], self.version)
            path.write_bytes(contents)
        (output / 'unexpected').write_text('synthetic')
        with self.assertRaises(ASSEMBLY.AssemblyError):
            RELEASE.verify(output, self.identity['source_commit'], self.version)

    def test_edited_receipt_repacked_archive_and_checksum_lines_fail_after_transfer(self):
        output = self.transfer()
        for path in [output / 'build-manifest-macos-aarch64.json', next(output.glob('*.zip')),
                     output / 'SHA256SUMS-aarch64.txt']:
            original = path.read_bytes()
            path.write_bytes(original + b'\n')
            with self.subTest(name=path.name), self.assertRaises(ASSEMBLY.AssemblyError):
                RELEASE.verify(output, self.identity['source_commit'], self.version)
            path.write_bytes(original)

    def test_mutation_while_copying_never_gets_an_accepted_checksum_file(self):
        original = RELEASE.fingerprint
        def mutate(path, maximum=RELEASE.MAX_ARTIFACT, destination=None):
            if destination is not None:
                path.write_bytes(b'X' * path.stat().st_size)
            return original(path, maximum, destination)
        with patch.object(RELEASE, 'fingerprint', side_effect=mutate), self.assertRaises(ASSEMBLY.AssemblyError):
            self.stage()
        self.assertFalse((self.root / 'stage-macos-aarch64/SHA256SUMS-aarch64.txt').exists())

    def test_combined_checksum_cannot_be_edited(self):
        output = self.transfer()
        RELEASE.verify(output, self.identity['source_commit'], self.version, write_combined=True)
        path = output / RELEASE.SUMS
        path.write_bytes(path.read_bytes() + b'\n')
        with self.assertRaises(ASSEMBLY.AssemblyError):
            RELEASE.verify(output, self.identity['source_commit'], self.version, write_combined=True)


if __name__ == '__main__':
    unittest.main()
