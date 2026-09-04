"""Local, versioned artifact contract shared by build/inspection/assembly tools."""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
MATRIX = ROOT / "packaging/targets.json"
EXPECTED = {
    "aarch64-apple-darwin": ("macos", "Darwin", ["dmg", "app.zip"]),
    "x86_64-apple-darwin": ("macos", "Darwin", ["dmg", "app.zip"]),
    "x86_64-pc-windows-msvc": ("windows", "Windows", ["nsis"]),
    "x86_64-unknown-linux-gnu": ("linux", "Linux", ["deb", "appimage"]),
}


def load_matrix(path=MATRIX):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if data.get("schema_version") != 1 or data.get("matrix_revision") != "p4-v1":
        raise ValueError("Unsupported packaging contract revision")
    if (data.get("product_name"), data.get("identifier"), data.get("binary_name")) != (
        "Cloud Burrito", "app.cloudburrito.desktop", "cloud-burrito"
    ):
        raise ValueError("Unexpected product identity")
    targets = data.get("targets", [])
    if len(targets) != len(EXPECTED) or {t.get("target") for t in targets} != set(EXPECTED):
        raise ValueError("Contract must retain every declared target exactly once")
    names, identifiers = set(), set()
    for row in targets:
        platform, host, formats = EXPECTED[row["target"]]
        if row.get("platform") != platform or row.get("host_os") != host:
            raise ValueError("Target platform/host mismatch")
        bundles = {"macos": ["app", "dmg"], "windows": ["nsis"], "linux": ["deb", "appimage"]}
        if row.get("bundle_targets") != bundles[platform]:
            raise ValueError("Unexpected native bundle selection")
        if row.get("id") in identifiers or not re.fullmatch(r"[a-z0-9_-]+", row.get("id", "")):
            raise ValueError("Duplicate or invalid target ID")
        identifiers.add(row["id"])
        if row.get("status") not in ("planned", "build-verified", "device-validated"):
            raise ValueError("Invalid validation status")
        if not isinstance(row.get("validated_os"), list) or (
            row["status"] == "device-validated" and not row["validated_os"]
        ):
            raise ValueError("Device validation requires named evidence")
        for key in ("minimum_os", "build_baseline", "runtime", "architecture", "binary_arch"):
            if not isinstance(row.get(key), str) or not row[key].strip():
                raise ValueError("Missing compatibility declaration")
        config = row.get("bundle_config", "")
        if config != f"src-tauri/tauri.{platform}.conf.json":
            raise ValueError("Unexpected platform configuration path")
        if [a.get("format") for a in row.get("artifacts", [])] != formats:
            raise ValueError("Missing or unexpected artifact formats")
        for artifact in row["artifacts"]:
            name = artifact.get("filename", "")
            if not re.fullmatch(r"cloud-burrito_\{version\}_[a-zA-Z0-9_.-]+", name) or "_unsigned" not in name or name in names:
                raise ValueError("Unsafe, duplicate or non-unsigned artifact name")
            names.add(name)
    return data


def target_for(value, matrix=None):
    rows = (matrix or load_matrix())["targets"]
    matches = [row for row in rows if value in (row["id"], row["target"])]
    if len(matches) != 1:
        raise ValueError("Target is outside the packaging contract")
    return matches[0]


def artifact_names(version, matrix=None):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Expected a plain release version")
    return {a["filename"].format(version=version): (row, a)
            for row in (matrix or load_matrix())["targets"] for a in row["artifacts"]}


if __name__ == "__main__":
    contract = load_matrix()
    print(f"Packaging contract ok: {len(contract['targets'])} targets, {len(artifact_names('0.2.9', contract))} artifacts; device evidence remains separate")
