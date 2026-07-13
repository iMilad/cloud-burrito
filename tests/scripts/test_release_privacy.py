from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "check-release-privacy.py"
SPEC = importlib.util.spec_from_file_location("check_release_privacy", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"could not load {SCRIPT}")
RELEASE_PRIVACY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = RELEASE_PRIVACY
SPEC.loader.exec_module(RELEASE_PRIVACY)


class ReleasePrivacyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)
        self.rules = RELEASE_PRIVACY.rule_set()

    def scan_bytes(self, data: bytes) -> list[str]:
        path = self.root / "fixture.bin"
        path.write_bytes(data)
        return RELEASE_PRIVACY.scan_file(path, self.rules)

    def test_binary_all_zero_account_sentinel_is_ignored(self) -> None:
        sentinel = ("0" * 12).encode("ascii")
        findings = self.scan_bytes(b"\0prefix=" + sentinel + b"\0")
        self.assertEqual(findings, [])

    def test_binary_nonzero_account_id_is_rejected(self) -> None:
        account_id = ("123456" + "789012").encode("ascii")
        findings = self.scan_bytes(b"\0account=" + account_id + b"\0")
        self.assertTrue(
            any("standalone_12_digit_number" in finding for finding in findings)
        )

    def test_binary_near_zero_account_id_is_rejected(self) -> None:
        account_id = ("0" * 11 + "1").encode("ascii")
        findings = self.scan_bytes(b"\0account=" + account_id + b"\0")
        self.assertTrue(
            any("standalone_12_digit_number" in finding for finding in findings)
        )

    def test_binary_formatted_nonzero_account_id_is_rejected(self) -> None:
        account_id = ("1234" + "-" + "5678" + "-" + "9012").encode("ascii")
        findings = self.scan_bytes(b"\0account=" + account_id + b"\0")
        self.assertTrue(
            any("formatted_12_digit_number" in finding for finding in findings)
        )

    def test_text_all_zero_value_remains_rejected(self) -> None:
        findings = self.scan_bytes(("account=" + "0" * 12 + "\n").encode("ascii"))
        self.assertTrue(
            any("standalone_12_digit_number" in finding for finding in findings)
        )

    def test_known_hash_denylist_remains_active_for_binary_strings(self) -> None:
        sentinel = "0" * 12
        hashes = {"known_account_number": {RELEASE_PRIVACY.digest(sentinel)}}
        with mock.patch.object(RELEASE_PRIVACY, "KNOWN_VALUE_HASHES", hashes):
            findings = self.scan_bytes(
                b"\0value=" + sentinel.encode("ascii") + b"\0"
            )
        self.assertFalse(
            any("standalone_12_digit_number" in finding for finding in findings)
        )
        self.assertTrue(
            any("known_account_number" in finding for finding in findings)
        )


if __name__ == "__main__":
    unittest.main()
