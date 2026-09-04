"""Offline regression checks for the exact local command capability."""

from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "check-tauri-commands.py"
SPEC = importlib.util.spec_from_file_location("check_tauri_commands", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class CommandRegistryTests(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory(prefix="cloud-burrito-registry-fixture-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.previous_root = CHECK.ROOT
        CHECK.ROOT = self.root
        self.addCleanup(setattr, CHECK, "ROOT", self.previous_root)
        self.write_fixture()

    def write_fixture(self, extra_commands=()):
        commands = sorted(CHECK.EXPECTED_COMMANDS | set(extra_commands))
        app = self.root / "src-tauri"
        (app / "src").mkdir(parents=True, exist_ok=True)
        (app / "capabilities").mkdir(exist_ok=True)
        (app / "build.rs").write_text(
            ".commands(&[" + ",".join(json.dumps(name) for name in commands) + "])",
            encoding="utf-8",
        )
        (app / "src" / "lib.rs").write_text(
            "tauri::generate_handler![" + ",".join("commands::" + name for name in commands) + "]",
            encoding="utf-8",
        )
        self.capability = {
            "identifier": "default", "windows": ["main"],
            "permissions": ["allow-" + name.replace("_", "-") for name in commands],
        }
        self.save_capability()
        (app / "tauri.conf.json").write_text(
            json.dumps({"app": {"security": {"capabilities": ["default"]}}}), encoding="utf-8",
        )

    def save_capability(self):
        (self.root / "src-tauri" / "capabilities" / "default.json").write_text(
            json.dumps(self.capability), encoding="utf-8",
        )

    def test_exact_local_registry_passes(self):
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(CHECK.main(), 0)
        self.assertIn("15 commands", output.getvalue())

    def test_adding_an_unreviewed_command_to_every_list_is_rejected(self):
        self.write_fixture(extra_commands=["synthetic_unreviewed_command"])
        with self.assertRaisesRegex(SystemExit, "Tauri command lists differ"):
            CHECK.main()

    def test_remote_other_window_and_extra_capability_are_rejected(self):
        for key, value, reason in [
            ("windows", ["*"], "main window"),
            ("webviews", ["*"], "main window"),
            ("remote", {"urls": ["https://example.invalid"]}, "local-only"),
            ("local", False, "local-only"),
        ]:
            with self.subTest(field=key):
                self.write_fixture()
                self.capability[key] = value
                self.save_capability()
                with self.assertRaisesRegex(SystemExit, reason):
                    CHECK.main()
        self.write_fixture()
        (self.root / "src-tauri" / "tauri.conf.json").write_text(
            json.dumps({"app": {"security": {"capabilities": ["default", "synthetic-extra"]}}}),
            encoding="utf-8",
        )
        with self.assertRaisesRegex(SystemExit, "reviewed default capability"):
            CHECK.main()


if __name__ == "__main__":
    unittest.main()
