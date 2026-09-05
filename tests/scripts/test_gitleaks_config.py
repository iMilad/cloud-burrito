"""Keep the public-preference exception narrower than the secret detector."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(shutil.which("gitleaks"), "gitleaks is not installed")
class GitleaksPreferenceExceptionTests(unittest.TestCase):
    def test_only_exact_public_value_in_its_source_file_is_exempt(self):
        preference = ".".join(("cb", "presentation", "v1"))
        cases = (
            ("frontend/studio.js", preference, False),
            ("frontend/app.js", preference, True),
            ("frontend/studio.js", preference + "-synthetic-secret", True),
        )
        for relative_path, value, should_find in cases:
            with self.subTest(path=relative_path, value=value), tempfile.TemporaryDirectory() as tmp:
                source = Path(tmp) / "source"
                file = source / relative_path
                file.parent.mkdir(parents=True)
                file.write_text(f'const preferenceKey = "{value}";\n')
                report = Path(tmp) / "report.json"
                result = subprocess.run(
                    ["gitleaks", "dir", ".", "--config", str(ROOT / ".gitleaks.toml"),
                     "--redact", "--no-banner", "--report-format", "json", "--report-path", str(report)],
                    cwd=source, capture_output=True, text=True, check=False,
                )
                self.assertIn(result.returncode, (0, 1), "Scanner failed; diagnostics withheld")
                findings = json.loads(report.read_text()) if report.exists() else []
                self.assertEqual(bool(findings), should_find)
                self.assertEqual(result.returncode, int(should_find))


if __name__ == "__main__":
    unittest.main()
