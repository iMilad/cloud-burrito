"""Tiny offline scanner fixtures: no repository scan, real Git or credentials."""
from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "check-detect-secrets.py"
SPEC = importlib.util.spec_from_file_location("check_detect_secrets", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
REVISION = "a" * 40
SOURCE = "frontend/app.js"
BENCHMARK = "docs/roadmap/benchmarks/p3-synthetic.json"


class FakeGitFingerprints(CHECK.Fingerprints):
    def __init__(self, root):
        super().__init__(root)
        self.git_calls = []
        self.historical_source = b"synthetic historical public source"
        self.revision_exists = True

    def git(self, *args):
        self.git_calls.append(args)
        if args == ("cat-file", "-t", REVISION):
            return b"commit\n" if self.revision_exists else None
        if args[0] == "log":
            return (REVISION + "\n").encode()
        if args == ("cat-file", "-s", f"{REVISION}:{SOURCE}"):
            return str(len(self.historical_source)).encode()
        if args == ("cat-file", "blob", f"{REVISION}:{SOURCE}"):
            return self.historical_source
        return None


class FingerprintTests(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory(prefix="cloud-burrito-fingerprint-fixture-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        source = self.root / SOURCE
        source.parent.mkdir(parents=True)
        source.write_bytes(b"synthetic current public source")
        self.current_digest = hashlib.sha256(source.read_bytes()).hexdigest()
        self.checker = FakeGitFingerprints(self.root)
        self.write_metadata(self.current_digest)

    def write_metadata(self, digest, revision=REVISION, source=SOURCE, extra=None):
        self.document = {
            "schema_version": 1, "recorded_at_utc": "2026-09-04T12:00:00Z",
            "metadata": {"source_revision": revision, "working_tree_dirty": True,
                         "production_sha256": {source: digest}},
            "limitations": [], "measurements": extra or [],
        }
        path = self.root / BENCHMARK
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(self.document, indent=2) + "\n")
        self.checker.metadata_lines.cache_clear()

    def finding(self, digest, filename=BENCHMARK, occurrence=0, detector="Hex High Entropy String"):
        lines = (self.root / filename).read_text().splitlines()
        line = [i for i, text in enumerate(lines, 1) if f'"{digest}"' in text][occurrence]
        return {"type": detector, "line_number": line,
                "hashed_secret": hashlib.sha1(digest.encode()).hexdigest()}

    def test_current_source_and_real_commit_fingerprints_are_verified(self):
        for digest in [REVISION, self.current_digest]:
            self.assertTrue(self.checker.verified(BENCHMARK, self.finding(digest)))
        report = {"results": {BENCHMARK: [self.finding(self.current_digest)]}}
        output = io.StringIO()
        self.assertEqual(CHECK.check_report(report, self.checker, output), 0)
        self.assertIn("verified 1 public benchmark", output.getvalue())
        self.assertNotIn(self.current_digest, output.getvalue())

    def test_historical_source_is_hashed_locally_and_lookups_are_cached(self):
        digest = hashlib.sha256(self.checker.historical_source).hexdigest()
        self.write_metadata(digest)
        finding = self.finding(digest)
        self.assertTrue(self.checker.verified(BENCHMARK, finding))
        calls = len(self.checker.git_calls)
        self.assertTrue(self.checker.verified(BENCHMARK, finding))
        self.assertEqual(len(self.checker.git_calls), calls)
        self.assertTrue(any(f"--max-count={CHECK.MAX_HISTORY_COMMITS}" in call for call in self.checker.git_calls))

    def test_other_detector_mismatched_token_and_ordinary_measurement_stay_fatal(self):
        self.write_metadata(self.current_digest, extra=[{"token": self.current_digest}])
        failures = [
            self.finding(self.current_digest, detector="AWS Access Key"),
            self.finding(self.current_digest, occurrence=1),
            {**self.finding(self.current_digest), "hashed_secret": "b" * 40},
        ]
        for finding in failures:
            self.assertFalse(self.checker.verified(BENCHMARK, finding))
        output = io.StringIO()
        self.assertEqual(CHECK.check_report({"results": {BENCHMARK: failures}}, self.checker, output), 1)
        self.assertIn("3 potential secret(s)", output.getvalue())
        self.assertNotIn(self.current_digest, output.getvalue())

    def test_unknown_digest_revision_and_non_source_path_cannot_be_allowlisted(self):
        for digest, revision, source in [
            (hashlib.sha256(b"synthetic unproved snapshot").hexdigest(), REVISION, SOURCE),
            (self.current_digest, "f" * 40, SOURCE),
            (self.current_digest, REVISION, "frontend/../private-source"),
            (self.current_digest, REVISION, "src-tauri/src/test_secret.rs"),
        ]:
            self.write_metadata(digest, revision, source)
            self.assertFalse(self.checker.verified(BENCHMARK, self.finding(digest)))
        self.assertFalse(self.checker.verified("somewhere-else.json", {"type":"Hex High Entropy String", "line_number":5}))
        self.assertFalse(self.checker.verified("../" + BENCHMARK, {"type":"Hex High Entropy String", "line_number":5}))

    def test_noncanonical_duplicate_or_malformed_metadata_fails_closed(self):
        path = self.root / BENCHMARK
        for transform in [
            lambda text: text.replace('"schema_version": 1', '"schema_version": 2'),
            lambda text: text.replace('    "source_revision":', '    "duplicate": 1,\n    "duplicate": 2,\n    "source_revision":'),
            lambda text: text.replace('"metadata": {', '"metadata": [', 1),
        ]:
            self.write_metadata(self.current_digest)
            path.write_text(transform(path.read_text()))
            self.assertFalse(self.checker.verified(BENCHMARK, self.finding(self.current_digest)))
        for malformed in [None, {}, {"results": []}, {"results": {BENCHMARK: [None]}}]:
            with self.assertRaises(ValueError):
                CHECK.check_report(malformed, self.checker, io.StringIO())

    def test_symlink_source_is_not_read_and_absent_git_proof_remains_fatal(self):
        source = self.root / SOURCE
        target = self.root / "synthetic-non-source"
        target.write_bytes(source.read_bytes())
        source.unlink()
        try:
            source.symlink_to(target)
        except (OSError, NotImplementedError):
            self.skipTest("fixture symlinks unavailable")
        self.assertFalse(self.checker.verified(BENCHMARK, self.finding(self.current_digest)))


if __name__ == "__main__":
    unittest.main()
