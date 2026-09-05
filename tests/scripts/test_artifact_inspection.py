from __future__ import annotations

import importlib.util
import hashlib
import io
import json
import re
import stat
import struct
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("artifact_inspection", ROOT / "scripts/artifact_inspection.py")
INSPECT = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = INSPECT
SPEC.loader.exec_module(INSPECT)


def fake_pe(*, machine=0x8664, version=(0, 2, 9), certificate=False, level="asInvoker"):
    """Structural unit fixture only; never passed off as a built installer."""
    data = bytearray(2048)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3c, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 132, machine, 1, 0, 0, 0, 240, 0)
    optional = 152
    struct.pack_into("<H", data, optional, 0x20b)
    directory = optional + 112
    struct.pack_into("<II", data, directory + 16, 0x1000, 1024)
    struct.pack_into("<II", data, directory + 32, 1900 if certificate else 0, 12 if certificate else 0)
    section = optional + 240
    data[section:section + 8] = b".rsrc\0\0\0"
    struct.pack_into("<IIII", data, section + 8, 1024, 0x1000, 1024, 512)
    base = 512
    # Two direct resource leaves are sufficient for the metadata parser.
    struct.pack_into("<HH", data, base + 12, 0, 2)
    struct.pack_into("<II", data, base + 16, 16, 32)
    struct.pack_into("<II", data, base + 24, 24, 48)
    struct.pack_into("<IIII", data, base + 32, 0x1080, 32, 0, 0)
    version_data = struct.pack("<IIII", 0xfeef04bd, 0x10000, version[0] << 16 | version[1], version[2] << 16)
    data[base + 128:base + 144] = version_data
    manifest = f'<assembly><requestedExecutionLevel level="{level}" uiAccess="false"/></assembly>'.encode()
    struct.pack_into("<IIII", data, base + 48, 0x1100, len(manifest), 0, 0)
    data[base + 256:base + 256 + len(manifest)] = manifest
    return bytes(data)


def fake_elf(glibc=b"GLIBC_2.35"):
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", data, 16, 3)
    struct.pack_into("<H", data, 18, 62)
    struct.pack_into("<H", data, 52, 64)
    return bytes(data) + glibc


def tar_bytes(entries):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w:gz") as archive:
        for name, contents, mode in entries:
            item = tarfile.TarInfo(name)
            item.size, item.mode = len(contents), mode
            archive.addfile(item, io.BytesIO(contents))
    return stream.getvalue()


def ar_bytes(entries):
    data = bytearray(b"!<arch>\n")
    for name, contents in entries:
        data.extend(f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(contents):<10}`\n".encode())
        data.extend(contents)
        if len(contents) % 2:
            data.extend(b"\n")
    return bytes(data)


class ArtifactInspectionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def test_zip_preserves_executable_and_relative_link_inventory(self):
        archive = self.root / "fixture.zip"
        with zipfile.ZipFile(archive, "w") as stream:
            executable = zipfile.ZipInfo("Synthetic.app/Contents/MacOS/synthetic")
            executable.external_attr = (stat.S_IFREG | 0o755) << 16
            stream.writestr(executable, b"synthetic-data")
            link = zipfile.ZipInfo("Synthetic.app/Contents/current")
            link.external_attr = (stat.S_IFLNK | 0o777) << 16
            stream.writestr(link, "MacOS/synthetic")
        INSPECT.extract_zip(archive, self.root / "extracted")
        before, _ = INSPECT.inventory(self.root / "extracted")
        binary = self.root / "extracted/Synthetic.app/Contents/MacOS/synthetic"
        self.assertTrue(binary.stat().st_mode & 0o111)
        binary.write_bytes(b"different")
        after, _ = INSPECT.inventory(self.root / "extracted")
        self.assertNotEqual(before, after)

    def test_directory_permissions_participate_in_payload_identity(self):
        INSPECT.unpack([("folder", 0o750, "dir", b""), ("folder/file", 0o644, "file", b"synthetic")], self.root / "payload")
        directory = self.root / "payload/folder"
        self.assertEqual(directory.stat().st_mode & 0o777, 0o750)
        before, _ = INSPECT.inventory(self.root / "payload")
        directory.chmod(0o755)
        after, _ = INSPECT.inventory(self.root / "payload")
        self.assertNotEqual(before, after)

    def test_privacy_rules_are_loaded_from_the_selected_source_export(self):
        (self.root / "scripts").mkdir()
        (self.root / "scripts/check-release-privacy.py").write_text(
            "def rule_set(): return []\ndef scan_file(path, rules): return ['synthetic rejection']\n")
        payload = self.root / "payload"
        payload.write_bytes(b"ordinary synthetic contents")
        with self.assertRaisesRegex(INSPECT.InspectionError, "privacy rules"):
            INSPECT.privacy([payload], source_root=self.root)

    def test_archive_traversal_collisions_privilege_and_link_children_fail(self):
        cases = [
            [("../escape", 0o644, "file", b"x")],
            [("a", 0o644, "file", b"x"), ("A", 0o644, "file", b"y")],
            [("a", 0o4755, "file", b"x")],
            [("a", 0o777, "link", b"../outside")],
            [("a", 0o777, "link", b"inside"), ("a/file", 0o644, "file", b"x")],
        ]
        for index, members in enumerate(cases):
            with self.subTest(index=index), self.assertRaises(INSPECT.InspectionError):
                INSPECT.unpack(members, self.root / str(index))
        self.assertFalse((self.root.parent / "escape").exists())

    def test_tar_hardlinks_are_not_extracted(self):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w") as archive:
            item = tarfile.TarInfo("link")
            item.type, item.linkname = tarfile.LNKTYPE, "other"
            archive.addfile(item)
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.extract_tar(stream.getvalue(), self.root / "out")

    def test_pe_architecture_version_certificate_and_privilege_checks(self):
        self.assertEqual(INSPECT.pe(fake_pe(), amd64=True, version="0.2.9", installer=True)["machine"], "AMD64")
        for value in [fake_pe(machine=0x14c), fake_pe(version=(0, 2, 8)), fake_pe(certificate=True), fake_pe(level="requireAdministrator")]:
            with self.subTest(value=value[:8]), self.assertRaises(INSPECT.InspectionError):
                INSPECT.pe(value, amd64=True, version="0.2.9", installer=True)
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.pe(fake_pe()[:400], amd64=True, version="0.2.9")

    def stock_nsis_script(self, directory=None):
        directory = directory or self.root
        directory.mkdir(parents=True, exist_ok=True)
        fixtures = ROOT / 'tests/scripts/fixtures/tauri-nsis-2.9.4'
        template = (fixtures / 'installer.nsi').read_bytes()
        self.assertEqual(hashlib.sha256(template).hexdigest(),
                         '20f4ecc730defb71f1342eaeaec4021df13be3d843abba0effe88ea5835fa079')
        text = template.decode()
        includes = re.findall(r'(?m)^[ \t]*!include[^\r\n]*', text)
        includes = [line for line in includes if '{{installer_hooks}}' not in line]
        includes = [line.replace('{{this}}', str(directory / 'English.nsh')) for line in includes]
        guards = re.findall(r'(?m)^[ \t]*!ifmacrodef NSIS_HOOK_[^\n]*\n[^\n]*\n[ \t]*!endif', text)
        self.assertEqual(len(guards), 4)
        generated = '!define INSTALLMODE "currentUser"\n!define VERSION "0.2.9"\n'
        generated += '\n'.join([*includes, *guards]) + '\n'
        script = directory / 'installer.nsi'
        script.write_text(generated, encoding='utf-8-sig')
        for name in INSPECT.NSIS_GENERATED_INCLUDES:
            (directory / name).write_bytes(b'\xef\xbb\xbf' + (fixtures / name).read_bytes())
        return script, generated

    def test_pinned_stock_nsis_guards_and_includes_are_accepted(self):
        script, generated = self.stock_nsis_script()
        INSPECT.inspect_nsis_script(script, '0.2.9')
        script.write_bytes(b'\xef\xbb\xbf' + generated.replace('\n', '\r\n').encode())
        INSPECT.inspect_nsis_script(script, '0.2.9')

    def test_windows_inspection_continues_to_payload_for_stock_generated_script(self):
        script, _ = self.stock_nsis_script(self.root / 'nsis/x64')
        installer = self.root / 'synthetic-setup.exe'
        installer.write_bytes(fake_pe(machine=0x14c))
        def extracted(path, destination, expected_type):
            self.assertEqual(expected_type, 'Nsis')
            destination.mkdir()
            (destination / 'cloud-burrito.exe').write_bytes(fake_pe())
        with mock.patch.object(INSPECT, 'extract_external', side_effect=extracted) as extraction:
            payload_hash, details = INSPECT.inspect_windows(installer, self.root, '0.2.9', self.root)
        extraction.assert_called_once()
        self.assertEqual(len(payload_hash), 64)
        self.assertFalse(details['custom_hooks'])
        self.assertEqual(details['generated_script_sha256'], INSPECT.digest(script))

    def test_nsis_rejects_defined_unconditional_changed_or_duplicate_hooks(self):
        script, generated = self.stock_nsis_script()
        for changed in [
            generated + '!macro NSIS_HOOK_PREINSTALL\n!macroend\n',
            generated + '!define NSIS_HOOK_PREINSTALL\n',
            generated + '!insertmacro NSIS_HOOK_PREINSTALL\n',
            generated.replace('!ifmacrodef NSIS_HOOK_PREINSTALL', '!if 1'),
            generated.replace('!insertmacro NSIS_HOOK_PREINSTALL', '!insertmacro NSIS_HOOK_PREINSTALL extra'),
            generated + '!ifmacrodef NSIS_HOOK_PREINSTALL\n!insertmacro NSIS_HOOK_PREINSTALL\n!endif\n',
            generated + '!macro nsis_hook_preinstall\n!macroend\n',
        ]:
            with self.subTest(tail=changed[-90:]):
                script.write_text(changed, encoding='utf-8-sig')
                with self.assertRaises(INSPECT.InspectionError):
                    INSPECT.inspect_nsis_script(script, '0.2.9')

    def test_nsis_custom_includes_user_data_and_modified_stock_includes_are_rejected(self):
        script, generated = self.stock_nsis_script()
        for changed in [generated + '!include "custom.nsh"\n',
                        generated + '!include /NONFATAL "custom.nsh"\n',
                        generated.replace('"utils.nsh"', '"../utils.nsh"'),
                        generated.replace('English.nsh', 'English_custom.nsh'),
                        generated + 'Delete "$PROFILE\\.cloud_burrito\\config"\n',
                        generated + 'Delete "$PROFILE\\.aws\\config"\n']:
            script.write_text(changed, encoding='utf-8-sig')
            with self.subTest(tail=changed[-90:]), self.assertRaises(INSPECT.InspectionError):
                INSPECT.inspect_nsis_script(script, '0.2.9')
        script.write_text(generated, encoding='utf-8-sig')
        (self.root / 'utils.nsh').write_text('!macro NSIS_HOOK_PREINSTALL\n!macroend\n', encoding='utf-8-sig')
        with self.assertRaisesRegex(INSPECT.InspectionError, 'upstream bytes'):
            INSPECT.inspect_nsis_script(script, '0.2.9')

    def test_elf_baseline_and_architecture(self):
        self.assertEqual(INSPECT.elf(fake_elf())["maximum_glibc_reference"], "2.35")
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.elf(fake_elf(b"GLIBC_2.36"))
        wrong = bytearray(fake_elf())
        struct.pack_into("<H", wrong, 18, 183)
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.elf(bytes(wrong))

    def test_appimage_offset_uses_elf_extent_and_squashfs_bounds(self):
        stub = bytearray(fake_elf(b""))
        stub[8:11] = b"AI\x02"
        filesystem = bytearray(96)
        filesystem[:4] = b"hsqs"
        struct.pack_into("<I", filesystem, 12, 131072)
        struct.pack_into("<H", filesystem, 28, 4)
        struct.pack_into("<Q", filesystem, 40, 96)
        image = bytes(stub + filesystem)
        self.assertEqual(INSPECT.appimage_filesystem(image), bytes(filesystem))
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.appimage_filesystem(image[:-1])
        with self.assertRaises(INSPECT.InspectionError):
            INSPECT.appimage_filesystem(image + bytes(filesystem))

    def test_utf16_privacy_and_personal_paths_fail_without_exposing_values(self):
        path = self.root / "payload.bin"
        values = ["account=" + "123456" + "789012", "C:" + "\\Users\\" + "synthetic-person\\build",
                  "/" + "home/" + "synthetic-person/build"]
        for value in values:
            for encoding in ("utf-16le", "utf-16be"):
                path.write_bytes(b"\0\x01" + value.encode(encoding) + b"\0\0")
                with self.assertRaises(INSPECT.InspectionError) as raised:
                    INSPECT.privacy([path])
                self.assertNotIn(value, str(raised.exception))
        path.write_bytes(("sentinel=" + "0" * 12).encode("utf-16le"))
        INSPECT.privacy([path])

    def test_debian_control_payload_and_hooks(self):
        control = b"Package: cloud-burrito\nVersion: 0.2.9\nArchitecture: amd64\nMaintainer: Cloud Burrito contributors\nDepends: libwebkit2gtk-4.1-0, libgtk-3-0\nDescription: Synthetic fixture\n"
        desktop = b"[Desktop Entry]\nType=Application\nExec=cloud-burrito\nName=Cloud Burrito\n"
        payload = tar_bytes([("usr/bin/cloud-burrito", fake_elf(), 0o755),
                             ("usr/share/applications/cloud-burrito.desktop", desktop, 0o644)])
        for hooks in (False, True):
            directory = self.root / str(hooks)
            directory.mkdir()
            entries = [("control", control, 0o644)]
            if hooks:
                entries.append(("postinst", b"synthetic hook", 0o755))
            package = directory / "fixture.deb"
            package.write_bytes(ar_bytes([("debian-binary", b"2.0\n"), ("control.tar.gz", tar_bytes(entries)), ("data.tar.gz", payload)]))
            if hooks:
                with self.assertRaises(INSPECT.InspectionError):
                    INSPECT.inspect_deb(package, directory, "0.2.9")
            else:
                payload_hash, inspected = INSPECT.inspect_deb(package, directory, "0.2.9")
                self.assertEqual(len(payload_hash), 64)
                self.assertFalse(inspected["maintainer_hooks"])

    def test_missing_native_tool_is_a_failure_not_an_inspection_pass(self):
        with mock.patch.object(INSPECT.shutil, "which", return_value=None):
            with self.assertRaisesRegex(INSPECT.InspectionError, "unavailable"):
                INSPECT.run(["synthetic-native-tool", "--version"])

    def test_inspection_tool_output_is_bounded_while_running(self):
        with mock.patch.object(INSPECT, "MAX_TOOL_OUTPUT", 32):
            with self.assertRaisesRegex(INSPECT.InspectionError, "output"):
                INSPECT.run([sys.executable, "-c", "print('synthetic' * 100)"])

    def test_external_archive_link_escape_rejected_before_extraction(self):
        listing = b"Type = Nsis\n----------\nPath = child\nSize = 1\nSymbolic Link = ../../outside\n"
        with mock.patch.object(INSPECT, "run", return_value=(0, listing)) as runner:
            with self.assertRaises(INSPECT.InspectionError):
                INSPECT.extract_external(self.root / "fixture", self.root / "out", "Nsis")
            self.assertEqual(runner.call_count, 1)

    def test_ambiguous_external_format_metadata_does_not_prove_nsis(self):
        listing = b"Type = zip\r\nComment = synthetic\r\nType = Nsis\r\n----------\r\nPath = file\r\nSize = 1\r\n"
        with mock.patch.object(INSPECT, "run", return_value=(0, listing)) as runner:
            with self.assertRaisesRegex(INSPECT.InspectionError, "unambiguously"):
                INSPECT.extract_external(self.root / "fixture", self.root / "out", "Nsis")
            self.assertEqual(runner.call_count, 1)

    def test_extra_outputs_and_contract_changes_fail_before_native_inspection(self):
        matrix = json.loads((ROOT / "packaging/targets.json").read_text())
        row = matrix["targets"][0]
        (self.root / "src-tauri").mkdir()
        (self.root / "packaging").mkdir()
        (self.root / "packaging/targets.json").write_text(json.dumps(matrix))
        (self.root / "src-tauri/tauri.conf.json").write_text("{}")
        (self.root / row["bundle_config"]).write_text("{}")
        artifacts = self.root / "assets"
        artifacts.mkdir()
        for item in row["artifacts"]:
            (artifacts / item["filename"].format(version="0.2.9")).write_bytes(b"synthetic")
        (artifacts / "unexpected").write_bytes(b"x")
        with self.assertRaisesRegex(INSPECT.InspectionError, "extra"):
            INSPECT.inspect_target(row, artifacts, "0.2.9", source_root=self.root)
        altered = dict(row, architecture="unexpected")
        with self.assertRaisesRegex(INSPECT.InspectionError, "contract"):
            INSPECT.inspect_target(altered, artifacts, "0.2.9", source_root=self.root)


if __name__ == "__main__":
    unittest.main()
