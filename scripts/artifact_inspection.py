"""Inspect local distributables without installing or executing their payloads.

Reports are descriptive unsigned evidence, not publisher authentication. Native
tools must already exist; this module never downloads tools or runs an AppImage.
"""
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import platform
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
MAX_FILE = 250 * 1024 * 1024
MAX_ARCHIVE = 1024 * 1024 * 1024
MAX_TOTAL = 2 * 1024 * 1024 * 1024
MAX_FILES = 20_000
MAX_TOOL_OUTPUT = 8 * 1024 * 1024


class InspectionError(ValueError):
    """A required artifact assertion could not be proved."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise InspectionError(message)


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def read(path: Path, limit: int = MAX_FILE) -> bytes:
    require(path.is_file() and not path.is_symlink(), "Expected a regular inspection input")
    require(path.stat().st_size <= limit, "Inspection input exceeds its byte limit")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "Inspection input grew beyond its byte limit")
    return data


def run(arguments: list[str], *, timeout: int = 90, allow_failure: bool = False) -> tuple[int, bytes]:
    executable = shutil.which(arguments[0])
    require(executable is not None, f"Required inspection tool is unavailable: {arguments[0]}")
    # Drain into a bounded buffer and terminate the utility on overflow. Tool
    # stderr and personal paths never enter exception text or public reports.
    data = bytearray()
    overflow = threading.Event()
    try:
        process = subprocess.Popen([executable, *arguments[1:]], stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError as error:
        raise InspectionError(f"Inspection tool could not start: {arguments[0]}") from error

    def collect() -> None:
        try:
            while chunk := process.stdout.read(65536):
                if len(data) + len(chunk) > MAX_TOOL_OUTPUT:
                    overflow.set()
                    process.kill()
                    return
                data.extend(chunk)
        except (OSError, ValueError):
            overflow.set()

    reader = threading.Thread(target=collect, daemon=True)
    reader.start()
    try:
        status = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        process.kill()
        process.wait()
        raise InspectionError(f"Inspection tool timed out: {arguments[0]}") from error
    finally:
        reader.join(timeout=2)
        if not reader.is_alive():
            process.stdout.close()
    require(not reader.is_alive() and not overflow.is_set(), "Inspection tool output exceeds its limit or did not close")
    require(allow_failure or status == 0, f"Inspection tool rejected input: {arguments[0]}")
    return status, bytes(data)


def safe_name(name: str) -> str:
    require(name and len(name) <= 4096 and not any(ord(c) < 32 for c in name), "Unsafe archive member name")
    require("\\" not in name and ":" not in name and not name.startswith("/"), "Unsafe archive member path")
    parts = PurePosixPath(name).parts
    require(parts and all(part not in ("..", "") for part in parts), "Archive member escapes its root")
    return "/".join(part for part in parts if part != ".")


def link_target(name: str, target: str) -> None:
    require(target and not target.startswith("/") and "\\" not in target and ":" not in target,
            "Unsafe archive link target")
    require(not any(ord(c) < 32 for c in target), "Unsafe archive link target")
    parts = list(PurePosixPath(name).parent.parts)
    for part in PurePosixPath(target).parts:
        if part == "..":
            require(bool(parts), "Archive link escapes its root")
            parts.pop()
        elif part != ".":
            parts.append(part)


def unpack(members: list[tuple[str, int, str, bytes]], destination: Path) -> None:
    """Extract only prevalidated regular files/directories/relative symlinks."""
    require(len(members) <= MAX_FILES, "Archive has too many members")
    seen: set[str] = set()
    links: set[str] = set()
    total = 0
    normalized = []
    for name, mode, kind, data in members:
        name = safe_name(name)
        require(name and name.casefold() not in seen, "Duplicate or case-colliding archive member")
        seen.add(name.casefold())
        require(not mode & (stat.S_ISUID | stat.S_ISGID), "Privileged archive mode is forbidden")
        require(kind in ("file", "dir", "link"), "Special archive member is forbidden")
        total += len(data)
        require(len(data) <= MAX_FILE and total <= MAX_TOTAL, "Extracted data exceeds its budget")
        if kind == "link":
            try:
                link_target(name, data.decode("utf-8"))
            except UnicodeDecodeError as error:
                raise InspectionError("Invalid archive link encoding") from error
            links.add(name.casefold())
        normalized.append((name, mode, kind, data))
    for name, _, _, _ in normalized:
        require(not any(str(parent).casefold() in links for parent in PurePosixPath(name).parents),
                "Archive member is nested beneath a link")
    destination.mkdir(parents=True, exist_ok=False)
    directories = []
    for name, mode, kind, data in sorted(normalized, key=lambda entry: entry[2] == "link"):
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if kind == "dir":
            path.mkdir(exist_ok=True)
            directories.append((path, mode & 0o777))
        elif kind == "link":
            path.symlink_to(data.decode("utf-8"))
        else:
            with path.open("xb") as stream:
                stream.write(data)
            path.chmod(mode & 0o777)
    for path, mode in sorted(directories, key=lambda entry: len(entry[0].parts), reverse=True):
        path.chmod(mode)


def extract_zip(path: Path, destination: Path) -> None:
    members = []
    total = 0
    with zipfile.ZipFile(path) as archive:
        require(len(archive.infolist()) <= MAX_FILES, "ZIP has too many members")
        for item in archive.infolist():
            total += item.file_size
            require(item.file_size <= MAX_FILE and total <= MAX_TOTAL, "ZIP expands beyond budget")
            require(not item.flag_bits & 1, "Encrypted ZIP is unsupported")
            mode = item.external_attr >> 16
            kind = "dir" if item.is_dir() else "link" if stat.S_ISLNK(mode) else "file"
            require(stat.S_IFMT(mode) in (0, stat.S_IFREG, stat.S_IFDIR, stat.S_IFLNK), "ZIP special file is forbidden")
            members.append((item.filename, mode or (0o755 if kind == "dir" else 0o644), kind, b"" if kind == "dir" else archive.read(item)))
    unpack(members, destination)


def extract_tar(data: bytes, destination: Path) -> None:
    members = []
    total = 0
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as archive:
        for item in archive:
            require(len(members) < MAX_FILES, "TAR has too many members")
            total += item.size
            require(item.size <= MAX_FILE and total <= MAX_TOTAL, "TAR expands beyond budget")
            name = item.name.rstrip("/")
            if name in ("", ".", "./") and item.isdir():
                continue
            if item.isfile():
                stream = archive.extractfile(item)
                require(stream is not None, "TAR member is unreadable")
                value = stream.read(MAX_FILE + 1)
                kind = "file"
            elif item.isdir():
                kind, value = "dir", b""
            elif item.issym():
                kind, value = "link", item.linkname.encode("utf-8")
            else:
                raise InspectionError("TAR hardlinks, devices and special files are forbidden")
            members.append((name, item.mode, kind, value))
    unpack(members, destination)


def inventory(root: Path) -> tuple[str, list[Path]]:
    entries = []
    files = []
    total = 0
    for path in sorted(root.rglob("*")):
        require(len(entries) < MAX_FILES, "Payload has too many entries")
        name = path.relative_to(root).as_posix()
        safe_name(name)
        require(not any(part in {".git", ".aws", ".cloud_burrito"} or part.endswith(".dSYM")
                        for part in path.relative_to(root).parts)
                and path.suffix.lower() != ".pdb" and path.name not in {".env", "credentials", "audit.log"},
                "Development or user-data payload is forbidden")
        mode = path.lstat().st_mode
        require(not mode & (stat.S_ISUID | stat.S_ISGID), "Privileged payload mode is forbidden")
        require(not any(parent.is_symlink() for parent in path.parents if parent != root and root in parent.parents), "Payload traverses a link")
        if path.is_symlink():
            target = os.readlink(path)
            link_target(name, target)
            entries.append([name, "link", target])
        elif path.is_dir():
            entries.append([name, "dir", mode & 0o777])
        else:
            require(stat.S_ISREG(mode), "Payload contains a special file")
            total += path.stat().st_size
            require(path.stat().st_size <= MAX_FILE and total <= MAX_TOTAL, "Payload byte budget exceeded")
            files.append(path)
            entries.append([name, "file", mode & 0o777, path.stat().st_size, digest(path)])
    encoded = json.dumps(entries, separators=(",", ":"), ensure_ascii=True).encode()
    return hashlib.sha256(encoded).hexdigest(), files


def privacy(files: list[Path], *, source_root: Path = ROOT) -> None:
    spec = importlib.util.spec_from_file_location("artifact_release_privacy", source_root / "scripts/check-release-privacy.py")
    require(spec is not None and spec.loader is not None, "Privacy rules unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    extra = [
        re.compile(r"(?i)\b[A-Z]:[\\/]Users[\\/](?!runneradmin(?:[\\/]|\b)|Public(?:[\\/]|\b)|Default(?:[\\/]|\b))[A-Za-z0-9._ -]+[\\/]"),
        re.compile(r"/home/(?!runner(?:/|\b))[A-Za-z0-9._-]+/"),
    ]
    for path in files:
        data = read(path)
        require(not module.scan_file(path, module.rule_set()), "Packaged privacy rules rejected an input")
        # UTF-16 ASCII runs cover PE version/PDB strings without retaining a
        # decoded copy of every binary. Existing binary sentinel rules apply.
        runs = []
        for pattern in (rb"(?:[\x20-\x7e]\x00){6,}", rb"(?:\x00[\x20-\x7e]){6,}"):
            for match in re.finditer(pattern, data):
                runs.append(match.group().decode("utf-16le" if match.group()[1] == 0 else "utf-16be"))
        for text in [module.binary_strings(data), *runs]:
            require(not any(pattern.search(text) for pattern in extra), "Packaged personal build path found")
            for line in text.splitlines():
                for rule in module.rule_set():
                    require(not any(not module.is_ignored_binary_sentinel(rule, match.group(), is_binary=True)
                                    for match in rule.regex.finditer(line)), "Packaged binary metadata failed privacy rules")
                require(not module.known_hash_findings(Path("payload"), 1, line), "Packaged binary metadata failed private-marker rules")


def pe(data: bytes, *, amd64: bool, version: str, installer: bool = False) -> dict:
    require(len(data) >= 64 and data[:2] == b"MZ", "Expected PE executable")
    offset = struct.unpack_from("<I", data, 0x3c)[0]
    require(offset + 24 <= len(data) and data[offset:offset + 4] == b"PE\0\0", "Invalid PE header")
    machine, sections, _, _, _, optional_size, _ = struct.unpack_from("<HHIIIHH", data, offset + 4)
    require(not amd64 or machine == 0x8664, "Embedded application is not AMD64")
    optional = offset + 24
    require(optional + optional_size + sections * 40 <= len(data), "Truncated PE sections")
    magic = struct.unpack_from("<H", data, optional)[0]
    require(magic in (0x10b, 0x20b), "Unsupported PE optional header")
    directory = optional + (112 if magic == 0x20b else 96)
    require(directory + 5 * 8 <= optional + optional_size, "Missing PE data directories")
    certificate = struct.unpack_from("<II", data, directory + 4 * 8)
    require(certificate == (0, 0), "Publisher certificate table is forbidden")
    sections_at = optional + optional_size

    def address(rva: int, length: int) -> int:
        for index in range(sections):
            position = sections_at + index * 40
            _, virtual, size, pointer = struct.unpack_from("<IIII", data, position + 8)
            if virtual <= rva and rva + length <= virtual + size:
                found = pointer + rva - virtual
                require(found + length <= len(data), "PE resource exceeds file")
                return found
        raise InspectionError("PE resource is outside a file-backed section")

    resource_rva, resource_size = struct.unpack_from("<II", data, directory + 2 * 8)
    require(resource_rva > 0 and resource_size <= MAX_TOOL_OUTPUT, "PE version resources missing")
    base = address(resource_rva, resource_size)
    blobs: dict[int, list[bytes]] = {}
    seen: set[int] = set()

    def walk(relative: int, kind: int = 0, depth: int = 0) -> None:
        require(depth <= 3 and relative not in seen and relative + 16 <= resource_size, "Invalid PE resource tree")
        seen.add(relative)
        named, ids = struct.unpack_from("<HH", data, base + relative + 12)
        require(named + ids <= 256 and relative + 16 + (named + ids) * 8 <= resource_size, "PE resource tree exceeds budget")
        for index in range(named + ids):
            name, target = struct.unpack_from("<II", data, base + relative + 16 + index * 8)
            current = name if depth == 0 else kind
            if target & 0x80000000:
                walk(target & 0x7fffffff, current, depth + 1)
            else:
                require(target + 16 <= resource_size, "Invalid PE resource data entry")
                rva, size = struct.unpack_from("<II", data, base + target)
                require(size <= MAX_TOOL_OUTPUT, "PE metadata exceeds budget")
                start = address(rva, size)
                blobs.setdefault(current, []).append(data[start:start + size])
    walk(0)
    expected = tuple(int(part) for part in version.split("."))
    versions = []
    for blob in blobs.get(16, []):
        location = blob.find(b"\xbd\x04\xef\xfe")
        if location >= 0 and location + 16 <= len(blob):
            high, low = struct.unpack_from("<II", blob, location + 8)
            versions.append((high >> 16, high & 65535, low >> 16))
    require(versions and all(item == expected for item in versions), "PE file version does not match candidate")
    manifests = []
    for blob in blobs.get(24, []):
        text = blob.decode("utf-16" if blob.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8", errors="replace")
        manifests.extend(re.findall(r'requestedExecutionLevel\s+[^>]*level\s*=\s*[\'"]([^\'"]+)', text))
    require(all(level == "asInvoker" for level in manifests), "Elevated execution manifest is forbidden")
    require(not installer or manifests == ["asInvoker"], "Installer must prove asInvoker execution")
    return {"machine": "AMD64" if machine == 0x8664 else hex(machine), "publisher_certificate": False,
            "file_version": version, "execution_levels": manifests}


def elf(data: bytes) -> dict:
    require(len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01", "Expected little-endian ELF64")
    require(struct.unpack_from("<H", data, 16)[0] in (2, 3), "ELF is not an executable or shared object")
    require(struct.unpack_from("<H", data, 18)[0] == 62, "ELF architecture is not x86-64")
    versions = sorted({tuple(map(int, value.split(b"."))) for value in re.findall(rb"GLIBC_([0-9]+\.[0-9]+)", data)})
    require(not versions or max(versions) <= (2, 35), "ELF requires newer glibc than the declared baseline")
    return {"machine": "x86-64", "maximum_glibc_reference": ".".join(map(str, max(versions))) if versions else None}


def appimage_filesystem(data: bytes) -> bytes:
    """Find the appended filesystem from bounded ELF file-backed extents."""
    require(data[8:11] == b"AI\x02", "Expected a type-2 AppImage")
    phoff, shoff = struct.unpack_from("<QQ", data, 32)
    ehsize, phsize, phcount, shsize, shcount = struct.unpack_from("<HHHHH", data, 52)
    require(ehsize == 64 and phcount <= 4096 and shcount <= 4096, "Invalid AppImage ELF dimensions")
    require((not phcount or phsize == 56) and (not shcount or shsize == 64), "Unexpected AppImage ELF table size")
    end = max(ehsize, phoff + phsize * phcount, shoff + shsize * shcount)
    require(end <= len(data), "AppImage ELF table exceeds file")
    for index in range(phcount):
        position = phoff + index * phsize
        offset = struct.unpack_from("<Q", data, position + 8)[0]
        size = struct.unpack_from("<Q", data, position + 32)[0]
        require(offset + size <= len(data), "AppImage ELF segment exceeds file")
        end = max(end, offset + size)
    for index in range(shcount):
        position = shoff + index * shsize
        kind = struct.unpack_from("<I", data, position + 4)[0]
        if kind != 8:  # SHT_NOBITS occupies memory, not bytes in this file.
            offset, size = struct.unpack_from("<QQ", data, position + 24)
            require(offset + size <= len(data), "AppImage ELF section exceeds file")
            end = max(end, offset + size)
    candidates = []
    for offset in range(end, min(end + 4096, len(data) - 95)):
        if data[offset:offset + 4] != b"hsqs":
            continue
        major = struct.unpack_from("<H", data, offset + 28)[0]
        size = struct.unpack_from("<Q", data, offset + 40)[0]
        block = struct.unpack_from("<I", data, offset + 12)[0]
        if major == 4 and 96 <= size <= len(data) - offset and 4096 <= block <= 1024 * 1024 and not block & (block - 1):
            candidates.append((offset, size))
    require(len(candidates) == 1, "AppImage filesystem offset could not be proved")
    offset, size = candidates[0]
    return data[offset:offset + size]


def deb_members(data: bytes) -> dict[str, bytes]:
    require(data.startswith(b"!<arch>\n"), "Expected Debian ar container")
    offset, members = 8, {}
    while offset < len(data):
        require(offset + 60 <= len(data), "Truncated ar header")
        header = data[offset:offset + 60]
        require(header[58:] == b"`\n", "Invalid ar header")
        try:
            name = header[:16].decode("ascii").strip().removesuffix("/")
            size = int(header[48:58])
        except (UnicodeError, ValueError) as error:
            raise InspectionError("Invalid ar member") from error
        require(name not in members and 0 <= size <= MAX_ARCHIVE and offset + 60 + size <= len(data), "Invalid ar member size or duplicate")
        members[name] = data[offset + 60:offset + 60 + size]
        offset += 60 + size + size % 2
    require(members.get("debian-binary") == b"2.0\n", "Unsupported Debian format")
    require(len(members) == 3, "Unexpected Debian archive member")
    return members


def inspect_deb(path: Path, temporary: Path, version: str, *, source_root: Path = ROOT) -> tuple[str, dict]:
    members = deb_members(read(path, MAX_ARCHIVE))
    controls = [name for name in members if re.fullmatch(r"control\.tar(?:\.(?:gz|xz|bz2))?", name)]
    payloads = [name for name in members if re.fullmatch(r"data\.tar(?:\.(?:gz|xz|bz2))?", name)]
    require(len(controls) == len(payloads) == 1, "Unsupported Debian compression or layout")
    control, payload = temporary / "control", temporary / "payload"
    extract_tar(members[controls[0]], control)
    extract_tar(members[payloads[0]], payload)
    require({p.relative_to(control).as_posix() for p in control.rglob("*")} <= {"control", "md5sums"},
            "Debian maintainer hooks or unknown control files are forbidden")
    text = read(control / "control", 64 * 1024).decode("utf-8")
    fields = {}
    for line in text.splitlines():
        if not line or line.startswith((" ", "\t")):
            continue
        key, separator, value = line.partition(":")
        require(separator and key not in fields, "Malformed Debian control metadata")
        fields[key] = value.strip()
    require(fields.get("Package") == "cloud-burrito" and fields.get("Version") == version
            and fields.get("Architecture") == "amd64", "Debian identity/version/architecture mismatch")
    require(fields.get("Maintainer") == "Cloud Burrito contributors", "Unexpected Debian maintainer metadata")
    dependencies = fields.get("Depends", "")
    require("libwebkit2gtk-4.1-0" in dependencies and "libgtk-3-0" in dependencies, "Debian runtime dependency declarations missing")
    for forbidden in ("etc", "root", "home", "usr/lib/systemd", "usr/share/polkit-1"):
        require(not (payload / forbidden).exists(), "Privileged or user-data Debian payload is forbidden")
    binary = payload / "usr/bin/cloud-burrito"
    require(binary.stat().st_mode & 0o111 != 0, "Debian application is not executable")
    executable = elf(read(binary))
    desktop = list((payload / "usr/share/applications").glob("*.desktop"))
    require(len(desktop) == 1, "Expected one desktop entry")
    entry = read(desktop[0], 64 * 1024).decode("utf-8")
    require(re.search(r"(?m)^Type=Application$", entry) is not None and
            re.search(r'(?m)^Exec="?cloud-burrito"?(?: %[uUfF])?$', entry) is not None,
            "Unexpected desktop launcher")
    payload_hash, files = inventory(payload)
    _, control_files = inventory(control)
    privacy(files + control_files, source_root=source_root)
    return payload_hash, {"control": fields, "executable": executable, "executable_sha256": digest(binary), "maintainer_hooks": False}


def extract_external(path: Path, destination: Path, expected_type: str) -> None:
    tool = "7zz" if shutil.which("7zz") else "7z"
    _, listing = run([tool, "l", "-slt", "--", str(path)])
    text = listing.decode("utf-8", errors="strict")
    blocks = text.split("----------", 1)
    require(len(blocks) == 2, "Archive member listing unavailable")
    types = [line.partition(" = ")[2] for line in blocks[0].splitlines() if line.startswith("Type = ")]
    require(types == [expected_type], "Archive tool did not recognize the required format unambiguously")
    names, total = set(), 0
    links = set()
    for block in re.split(r"\r?\n\r?\n", blocks[1].strip()):
        fields = dict(line.split(" = ", 1) for line in block.splitlines() if " = " in line)
        require("Path" in fields, "Malformed archive listing")
        name = safe_name(fields["Path"].replace("\\", "/"))
        require(name.casefold() not in names, "Duplicate archive member")
        names.add(name.casefold())
        require(len(names) <= MAX_FILES, "Archive has too many members")
        require(not fields.get("Hard Link"), "External archive hardlinks are unsupported")
        if fields.get("Symbolic Link"):
            link_target(name, fields["Symbolic Link"])
            links.add(name)
        elif fields.get("Attributes", "").startswith("l"):
            raise InspectionError("Archive symlink target could not be inspected")
        size = int(fields.get("Size", "0"))
        total += size
        require(0 <= size <= MAX_FILE and total <= MAX_TOTAL, "Archive expands beyond budget")
    for name in names:
        require(not any(str(parent) in {link.casefold() for link in links}
                        for parent in PurePosixPath(name).parents), "External archive member traverses a link")
    destination.mkdir()
    run([tool, "x", "-y", "-bd", f"-o{destination}", "--", str(path)])
    inventory(destination)


def inspect_windows(path: Path, temporary: Path, version: str, build_dir: Path | None, *, source_root: Path = ROOT) -> tuple[str, dict]:
    require(build_dir is not None, "Windows inspection requires the generated NSIS script")
    script = build_dir / "nsis/x64/installer.nsi"
    text = read(script, MAX_TOOL_OUTPUT).decode("utf-8-sig")
    require(re.search(r'(?m)^!define INSTALLMODE "currentUser"\s*$', text) is not None,
            "Generated installer is not currentUser mode")
    require(re.search(r'(?m)^!define VERSION "' + re.escape(version) + r'"\s*$', text) is not None,
            "Generated installer version mismatch")
    require(not re.search(r"NSIS_HOOK_|\.cloud_burrito|(?i:\.aws(?:[\\/\"\s]|$))", text), "Installer hook or user-data migration is forbidden")
    installer = pe(read(path), amd64=False, version=version, installer=True)
    payload = temporary / "payload"
    extract_external(path, payload, "Nsis")
    binaries = [p for p in payload.rglob("*") if p.is_file() and p.name.lower() == "cloud-burrito.exe"]
    require(len(binaries) == 1, "Expected exactly one embedded application executable")
    executable = pe(read(binaries[0]), amd64=True, version=version)
    payload_hash, files = inventory(payload)
    privacy([path, *files], source_root=source_root)
    return payload_hash, {"installer": installer, "executable": executable, "generated_script_sha256": digest(script),
                          "script_install_mode": "currentUser", "custom_hooks": False}


def inspect_appimage(path: Path, temporary: Path, *, source_root: Path = ROOT) -> tuple[str, dict]:
    data = read(path, MAX_ARCHIVE)
    outer = elf(data)
    filesystem = temporary / "filesystem.squashfs"
    filesystem.write_bytes(appimage_filesystem(data))
    payload = temporary / "payload"
    # Archive extraction runs a separately installed utility, never the image's
    # own --appimage-extract/--appimage-offset entry point.
    extract_external(filesystem, payload, "SquashFS")
    binaries = list(payload.glob("usr/bin/cloud-burrito"))
    require(len(binaries) == 1, "AppImage application payload missing")
    require((payload / "AppRun").exists(), "AppImage launcher missing")
    executable = elf(read(binaries[0]))
    payload_hash, files = inventory(payload)
    for file in files:
        with file.open("rb") as stream:
            magic = stream.read(4)
        if magic == b"\x7fELF":
            elf(read(file))
    privacy([path, *files], source_root=source_root)
    return payload_hash, {"runtime": outer, "executable": executable, "executable_sha256": digest(binaries[0]), "extraction": "external archive tool"}


def no_publisher(path: Path) -> None:
    status, metadata = run(["codesign", "--display", "--verbose=4", str(path)], allow_failure=True)
    text = metadata.decode("utf-8", errors="replace")
    require("Authority=" not in text, "Publisher authority is forbidden")
    teams = re.findall(r"(?m)^TeamIdentifier=(.*)$", text)
    require(all(team == "not set" for team in teams), "Publisher team is forbidden")
    require(status == 0 or "not signed at all" in text, "Code-sign metadata inspection was inconclusive")


def inspect_app(app: Path, row: dict, version: str, *, source_root: Path = ROOT) -> tuple[str, dict]:
    require(app.is_dir() and not app.is_symlink(), "Application bundle missing")
    try:
        info = plistlib.loads(read(app / "Contents/Info.plist", 1024 * 1024))
    except (ValueError, plistlib.InvalidFileException) as error:
        raise InspectionError("Invalid application plist") from error
    require(info.get("CFBundleIdentifier") == "app.cloudburrito.desktop" and
            info.get("CFBundleShortVersionString") == version and
            info.get("CFBundleExecutable") == "cloud-burrito", "Application bundle identity mismatch")
    require(info.get("LSMinimumSystemVersion") == "13.0", "Application minimum OS mismatch")
    binary = app / "Contents/MacOS/cloud-burrito"
    require(binary.stat().st_mode & 0o111 != 0, "Application binary is not executable")
    _, architecture = run(["lipo", "-archs", str(binary)])
    require(architecture.decode().strip() == row["binary_arch"], "Application architecture mismatch")
    no_publisher(app)
    no_publisher(binary)
    payload_hash, files = inventory(app)
    privacy(files, source_root=source_root)
    return payload_hash, {"architecture": row["binary_arch"], "identifier": info["CFBundleIdentifier"],
                          "minimum_os": "13.0", "publisher_identity": False}


def inspect_macos(paths: dict[str, Path], temporary: Path, row: dict, version: str, *, source_root: Path = ROOT) -> dict[str, tuple[str, dict]]:
    require(platform.system() == "Darwin", "DMG inspection requires macOS tools")
    extracted = temporary / "zip"
    extract_zip(paths["app.zip"], extracted)
    require({p.name for p in extracted.iterdir()} <= {"Cloud Burrito.app", "__MACOSX"}, "Unexpected app ZIP top-level payload")
    zipped = inspect_app(extracted / "Cloud Burrito.app", row, version, source_root=source_root)
    _, zip_files = inventory(extracted)
    for metadata in zip_files:
        relative = metadata.relative_to(extracted)
        if relative.parts[0] == "__MACOSX":
            require(metadata.name.startswith("._") and read(metadata, MAX_FILE)[:4] == b"\x00\x05\x16\x07",
                    "Unexpected ZIP resource-fork payload")
    privacy(zip_files, source_root=source_root)
    run(["hdiutil", "verify", str(paths["dmg"])])
    no_publisher(paths["dmg"])
    mount = temporary / "mount"
    mount.mkdir()
    mounted = False
    try:
        run(["hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", str(mount), str(paths["dmg"])])
        mounted = True
        allowed = {"Cloud Burrito.app", "Applications", ".background", ".VolumeIcon.icns", ".DS_Store"}
        require({p.name for p in mount.iterdir()} <= allowed, "Unexpected DMG payload")
        applications = mount / "Applications"
        if applications.is_symlink():
            require(os.readlink(applications) == "/Applications", "Unexpected DMG Applications link")
        else:
            require(not applications.exists(), "Unexpected DMG Applications payload")
        for extra in mount.iterdir():
            if extra.name in {"Cloud Burrito.app", "Applications"}:
                continue
            require(not extra.is_symlink(), "Unexpected DMG metadata link")
            if extra.is_dir():
                _, extra_files = inventory(extra)
                privacy(extra_files, source_root=source_root)
            else:
                privacy([extra], source_root=source_root)
        disk = inspect_app(mount / "Cloud Burrito.app", row, version, source_root=source_root)
        require(zipped[0] == disk[0], "DMG and ZIP application payloads differ")
    finally:
        if mounted or os.path.ismount(mount):
            run(["hdiutil", "detach", str(mount)])
    return {"app.zip": zipped, "dmg": (disk[0], {**disk[1], "container_verified": True, "matches_app_zip": True})}


def check_config(row: dict, source_root: Path) -> None:
    base = json.loads(read(source_root / "src-tauri/tauri.conf.json", 1024 * 1024))
    overlay = json.loads(read(source_root / row["bundle_config"], 1024 * 1024))
    for config in (base, overlay):
        bundle = config.get("bundle", {})
        require(not bundle.get("externalBin") and not bundle.get("createUpdaterArtifacts"), "Unexpected executable or updater bundle")
        require(bundle.get("publisher") in (None, "Cloud Burrito contributors", "cloudburrito"), "Unexpected publisher metadata")
        macos = bundle.get("macOS", {})
        require(not macos.get("signingIdentity") and not macos.get("providerShortName"), "Apple publisher configuration is forbidden")
        windows = bundle.get("windows", {})
        nsis = windows.get("nsis", {})
        require(not nsis.get("installerHooks") and not nsis.get("template"), "Custom installer hooks/templates are forbidden")
        require(not windows.get("certificateThumbprint") and not windows.get("signCommand"), "Publisher signing configuration is forbidden")
        for family in ("deb", "rpm"):
            settings = bundle.get("linux", {}).get(family, {})
            require(not any(value for key, value in settings.items() if "script" in key.lower() or "install" in key.lower()), "Package lifecycle hooks are forbidden")


def inspect_target(row: dict, artifacts_dir: Path, version: str, *, source_root: Path = ROOT,
                   build_dir: Path | None = None) -> dict:
    """Fail closed unless every required artifact passes real format inspection."""
    try:
        require(re.fullmatch(r"\d+\.\d+\.\d+", version) is not None, "Unsupported candidate version")
        matrix = json.loads(read(source_root / "packaging/targets.json", 1024 * 1024))
        known = [candidate for candidate in matrix["targets"] if candidate["id"] == row.get("id")]
        require(len(known) == 1 and row == known[0], "Inspection target differs from the source contract")
        check_config(row, source_root)
        expected = {item["filename"].format(version=version): item["format"] for item in row["artifacts"]}
        require(artifacts_dir.is_dir() and not artifacts_dir.is_symlink(), "Artifact directory missing")
        require({p.name for p in artifacts_dir.iterdir()} == set(expected), "Missing, duplicate or extra target artifacts")
        paths = {}
        original = {}
        for name, kind in expected.items():
            require(safe_name(name) == name and "/" not in name, "Unsafe artifact filename")
            path = artifacts_dir / name
            require(path.is_file() and not path.is_symlink() and 0 < path.stat().st_size <= MAX_ARCHIVE,
                    "Artifact is missing, linked, empty or oversized")
            paths[kind] = path
            original[name] = (path.stat().st_size, digest(path))
        with tempfile.TemporaryDirectory(prefix="burrito-inspect-") as directory:
            temporary = Path(directory)
            if row["platform"] == "macos":
                inspected = inspect_macos(paths, temporary, row, version, source_root=source_root)
            elif row["platform"] == "windows":
                inspected = {"nsis": inspect_windows(paths["nsis"], temporary, version, build_dir, source_root=source_root)}
            elif row["platform"] == "linux":
                (temporary / "deb").mkdir()
                (temporary / "appimage").mkdir()
                inspected = {"deb": inspect_deb(paths["deb"], temporary / "deb", version, source_root=source_root),
                             "appimage": inspect_appimage(paths["appimage"], temporary / "appimage", source_root=source_root)}
                require(inspected['deb'][1]['executable_sha256'] == inspected['appimage'][1]['executable_sha256'],
                        "Debian and AppImage application executables differ")
            else:
                raise InspectionError("Unsupported target platform")
        artifacts = []
        for name, kind in expected.items():
            path = artifacts_dir / name
            require(original[name] == (path.stat().st_size, digest(path)), "Artifact changed during inspection")
            payload_hash, assertions = inspected[kind]
            artifacts.append({"filename": name, "format": kind, "size_bytes": original[name][0],
                              "sha256": original[name][1], "payload_sha256": payload_hash, "inspection": assertions})
        return {"schema_version": 1, "target": row["target"], "version": version, "status": "passed", "artifacts": artifacts,
                "limitations": ["Unsigned descriptive evidence is not publisher authentication.",
                                 "Install, launch, upgrade, uninstall and real-device compatibility remain P5 work.",
                                 "Static metadata and reviewed build configuration are not universal installer control-flow proof."]}
    except InspectionError:
        raise
    except (OSError, ValueError, TypeError, IndexError, KeyError, OverflowError, struct.error, zipfile.BadZipFile, tarfile.TarError) as error:
        raise InspectionError("Artifact inspection could not establish the required format proof") from error
