from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "release-status.py"
SPEC = importlib.util.spec_from_file_location("release_status", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {SCRIPT}")
RELEASE_STATUS = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RELEASE_STATUS
SPEC.loader.exec_module(RELEASE_STATUS)


class ReleaseStatusCliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        temp_root = Path(self.temp_dir.name)
        self.repo = temp_root / "repo"
        self.remote = temp_root / "remote.git"
        self.repo.mkdir()
        (self.repo / "scripts").mkdir()
        (self.repo / "scripts" / "check-release-version.py").write_text(
            "import sys\n"
            "if '--print-version' in sys.argv:\n"
            "    print('0.2.6')\n"
            "else:\n"
            "    print('release version ok: 0.2.6')\n",
            encoding="utf-8",
        )
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release-test@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgSign", "false")
        (self.repo / "tracked.txt").write_text("initial\n", encoding="utf-8")
        self.git("add", "tracked.txt")
        self.git("commit", "-m", "Initial")
        self.run_command(["git", "init", "--bare", str(self.remote)])
        self.git("remote", "add", "origin", str(self.remote))
        self.git("push", "-u", "origin", "main")

    def run_command(
        self,
        command: list[str],
        *,
        cwd: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            command,
            cwd=cwd or self.repo,
            check=True,
            capture_output=True,
            text=True,
        )

    def git(self, *args: str) -> subprocess.CompletedProcess[str]:
        return self.run_command(["git", *args])

    def status(
        self,
        *args: str,
        remote: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        argv = ["--remote", str(remote or self.remote), *args]
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            returncode = RELEASE_STATUS.main(argv, root=self.repo)
        return subprocess.CompletedProcess(
            args=argv,
            returncode=returncode,
            stdout=stdout.getvalue(),
            stderr=stderr.getvalue(),
        )

    def test_missing_tag_is_pending(self) -> None:
        result = self.status()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "release pending: app-v0.2.6\n")
        self.assertEqual(result.stderr, "")

    def test_lightweight_tag_at_head_is_reported(self) -> None:
        self.git("tag", "app-v0.2.6")
        self.git("push", "origin", "app-v0.2.6")
        result = self.status()
        self.assertEqual(result.returncode, 10)
        self.assertEqual(result.stdout, "release tag exists: app-v0.2.6\n")
        self.assertEqual(result.stderr, "")

    def test_annotated_tag_at_head_is_reported(self) -> None:
        self.git("tag", "-a", "app-v0.2.6", "-m", "Release")
        self.git("push", "origin", "app-v0.2.6")
        result = self.status()
        self.assertEqual(result.returncode, 10)
        self.assertEqual(result.stdout, "release tag exists: app-v0.2.6\n")
        self.assertEqual(result.stderr, "")

    def test_existing_tag_at_older_commit_requires_version_bump(self) -> None:
        self.git("tag", "app-v0.2.6")
        self.git("push", "origin", "app-v0.2.6")
        (self.repo / "tracked.txt").write_text("next\n", encoding="utf-8")
        self.git("add", "tracked.txt")
        self.git("commit", "-m", "Next")
        result = self.status()
        self.assertEqual(result.returncode, 20)
        self.assertEqual(
            result.stdout,
            "version bump required: app-v0.2.6 already points to another commit\n",
        )
        self.assertEqual(result.stderr, "")

    def test_json_output_is_machine_readable(self) -> None:
        result = self.status("--json")
        self.assertEqual(result.returncode, 0)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["state"], "release-pending")
        self.assertEqual(payload["tag"], "app-v0.2.6")
        self.assertIsNone(payload["tag_commit"])

    def test_metadata_failure_is_reported_without_traceback(self) -> None:
        (self.repo / "scripts" / "check-release-version.py").write_text(
            "raise SystemExit('invalid metadata')\n",
            encoding="utf-8",
        )
        result = self.status()
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("release status error: release metadata validation failed", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_unreachable_remote_is_reported_without_traceback(self) -> None:
        missing_remote = Path(self.temp_dir.name) / "missing.git"
        result = self.status(remote=missing_remote)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("release status error: remote tag lookup", result.stderr)
        self.assertNotIn(str(missing_remote), result.stderr)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
