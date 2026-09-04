"""Offline metadata projection for candidate manifests; never opens license files.

This is declared-license evidence, not a legal compliance or shipped-code SBOM.
The caller supplies successful locked/offline, target-filtered Cargo metadata
and the committed npm lock document. This module performs no I/O or processes.
"""
import json
import re


class InventoryError(ValueError):
    pass


MAX_PACKAGES = 10000
NAME = re.compile(r"(?:@[A-Za-z0-9._-]+/)?[A-Za-z0-9][A-Za-z0-9._-]{0,213}\Z")
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?(?:\+[A-Za-z0-9.-]+)?\Z")
# Deliberately conservative, not a complete SPDX catalogue. Unknown IDs and
# custom LicenseRef text require review instead of echoing arbitrary metadata.
LICENSE_IDS = frozenset("""
0BSD Apache-2.0 Artistic-2.0 BSD-2-Clause BSD-3-Clause BSD-3-Clause-Clear
BSL-1.0 CC0-1.0 CC-BY-3.0 CC-BY-4.0 CC-BY-SA-4.0 GPL-2.0 GPL-2.0+
GPL-2.0-only GPL-2.0-or-later GPL-3.0 GPL-3.0+ GPL-3.0-only GPL-3.0-or-later
ISC LGPL-2.1 LGPL-2.1+ LGPL-2.1-only LGPL-2.1-or-later LGPL-3.0-only
LGPL-3.0-or-later MIT MPL-2.0 OpenSSL Unicode-3.0 Unicode-DFS-2016 Unlicense
WTFPL Zlib
""".split())
EXCEPTION_IDS = frozenset("""
LLVM-exception GCC-exception-2.0 GCC-exception-3.1 Autoconf-exception-2.0
Autoconf-exception-3.0 Classpath-exception-2.0
""".split())


def license_expression(value):
    """Return a normalized recognized expression, without retaining raw text."""
    if not isinstance(value, str) or len(value) > 1024 or not value.strip():
        return None
    tokens = re.findall(r"[A-Za-z0-9.+-]+|[()]|\S", value)
    position = 0

    def atom(depth):
        nonlocal position
        if depth > 16 or position >= len(tokens):
            return False
        if tokens[position] == "(":
            position += 1
            if not expression(depth + 1) or position >= len(tokens) or tokens[position] != ")":
                return False
            position += 1
            return True
        if tokens[position] not in LICENSE_IDS:
            return False
        position += 1
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens) or tokens[position] not in EXCEPTION_IDS:
                return False
            position += 1
        return True

    def expression(depth):
        nonlocal position
        if not atom(depth):
            return False
        while position < len(tokens) and tokens[position] in ("AND", "OR"):
            position += 1
            if not atom(depth):
                return False
        return True

    if not expression(0) or position != len(tokens):
        return None
    return " ".join(tokens).replace("( ", "(").replace(" )", ")")


def license_fields(package):
    expression = license_expression(package.get("license"))
    file_declared = bool(package.get("license_file"))
    # npm may encode a license-file instruction in its license string.
    raw = package.get("license")
    file_declared |= isinstance(raw, str) and raw.upper().startswith("SEE LICENSE IN ")
    reason = None
    if not expression:
        reason = "license_file_not_inspected" if file_declared else (
            "missing_expression" if raw is None or raw == "" else "unrecognized_expression"
        )
    return {"license": expression, "license_file_declared": file_declared,
            "license_review": "pending" if reason or file_declared else "declared_expression_only",
            "license_review_reason": reason or ("license_file_not_inspected" if file_declared else None)}


def identity(name, version, *, unresolved_version=False):
    if not isinstance(name, str) or not NAME.fullmatch(name):
        raise InventoryError("Dependency has an invalid package name")
    if version is None and unresolved_version:
        return {"name": name, "version": None}
    if not isinstance(version, str) or len(version) > 128 or not VERSION.fullmatch(version):
        raise InventoryError("Dependency has an invalid package version")
    return {"name": name, "version": version}


def cargo_rows(metadata):
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        raise InventoryError("Complete Cargo metadata is required")
    resolve = metadata.get("resolve")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("nodes"), list) or not resolve["nodes"]:
        raise InventoryError("Cargo resolved graph is required; no-deps metadata is insufficient")
    packages, nodes = metadata["packages"], resolve["nodes"]
    if len(packages) > MAX_PACKAGES or len(nodes) > MAX_PACKAGES:
        raise InventoryError("Cargo dependency inventory exceeds its package limit")
    by_id = {}
    for package in packages:
        if not isinstance(package, dict) or not isinstance(package.get("id"), str) or package["id"] in by_id:
            raise InventoryError("Cargo package identities are incomplete or ambiguous")
        by_id[package["id"]] = package
    node_ids = set()
    for node in nodes:
        if not isinstance(node, dict) or not isinstance(node.get("id"), str) or node["id"] in node_ids:
            raise InventoryError("Cargo graph identities are incomplete or ambiguous")
        node_ids.add(node["id"])
    if not node_ids <= by_id.keys():
        raise InventoryError("Cargo resolved graph references missing package metadata")
    for node in nodes:
        dependencies = node.get("dependencies", [])
        if not isinstance(dependencies, list) or any(not isinstance(item, str) or item not in node_ids for item in dependencies):
            raise InventoryError("Cargo resolved dependency edges are incomplete")
        edges = node.get("deps", [])
        if not isinstance(edges, list) or any(not isinstance(item, dict) or not isinstance(item.get("pkg"), str) or item["pkg"] not in node_ids for item in edges):
            raise InventoryError("Cargo resolved dependency edges are incomplete")
    members = metadata.get("workspace_members", [])
    if not isinstance(members, list) or any(not isinstance(member, str) for member in members):
        raise InventoryError("Cargo workspace identities are invalid")
    rows = []
    for package_id in node_ids:
        package = by_id[package_id]
        source = package.get("source")
        category = "unknown"
        if source is None:
            category = "workspace" if package_id in members else "path"
        elif isinstance(source, str):
            if source.startswith(("registry+", "sparse+")):
                category = "registry"
            elif source.startswith("git+"):
                category = "git"
        rows.append({"ecosystem": "cargo", **identity(package.get("name"), package.get("version")),
                     "source_category": category, **license_fields(package)})
    return rows, len(packages) - len(node_ids)


def npm_rows(lock):
    if not isinstance(lock, dict) or lock.get("lockfileVersion") not in (2, 3) or not isinstance(lock.get("packages"), dict):
        raise InventoryError("An npm lockfile v2/v3 packages map is required")
    packages = lock["packages"]
    if len(packages) > MAX_PACKAGES:
        raise InventoryError("Node dependency inventory exceeds its package limit")
    rows = []
    for location, package in packages.items():
        if location == "":
            continue  # Root project is not an npm dependency; it may have no version.
        if not isinstance(location, str) or not isinstance(package, dict):
            raise InventoryError("Node dependency metadata is invalid")
        name = package.get("name")
        if name is None and "node_modules/" in location:
            name = location.rsplit("node_modules/", 1)[1]
        linked = package.get("link") is True
        resolved = package.get("resolved")
        if linked or isinstance(resolved, str) and resolved.startswith("file:"):
            category = "path"
        elif isinstance(resolved, str) and resolved.startswith(("git+", "git:", "github:", "git@")):
            category = "git"
        elif isinstance(resolved, str) and resolved.startswith("https://registry.npmjs.org/"):
            category = "registry"
        elif isinstance(resolved, str) and resolved.startswith(("https://", "http://")):
            category = "remote"
        else:
            category = "unknown"
        for flag in ("dev", "devOptional", "optional", "link"):
            if flag in package and not isinstance(package[flag], bool):
                raise InventoryError("Node dependency scope flag is invalid")
        rows.append({"ecosystem": "npm", **identity(name, package.get("version"), unresolved_version=linked),
                     "source_category": category, "dev_only": package.get("dev") is True,
                     "dev_optional": package.get("devOptional") is True,
                     "optional": package.get("optional") is True, **license_fields(package)})
    return rows


def from_metadata(cargo_metadata_json, package_lock_json):
    """Project already-parsed JSON objects into a path/contact/URL-free report."""
    cargo, excluded = cargo_rows(cargo_metadata_json)
    node = npm_rows(package_lock_json)
    rows = sorted(cargo + node, key=lambda row: (row["ecosystem"], row["name"], row["version"] or "", json.dumps(row, sort_keys=True)))
    return {
        "schema_version": 1,
        "review_status": "review_pending",
        "scope": {
            "cargo": "Package nodes in the caller-supplied locked/offline target-filtered resolve graph, including workspace/build/dev dependencies when present; not a shipped-binary inventory.",
            "cargo_metadata_packages_outside_graph": excluded,
            "npm": "Lockfile package entries excluding the root project; may include optional packages for other platforms. dev_only is the lockfile flag, not inferred runtime inclusion.",
            "limits": "Declared expressions only; license files, notices, native libraries, bundler helpers and transitive system dependencies were not inspected. No legal compliance certification.",
        },
        "counts": {"cargo": len(cargo), "npm": len(node),
                   "npm_dev_only": sum(row["dev_only"] for row in node),
                   "license_review_pending": sum(row["license_review"] == "pending" for row in rows)},
        "packages": rows,
    }
