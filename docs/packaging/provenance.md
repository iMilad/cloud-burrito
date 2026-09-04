# Candidate provenance and inspection

Build from clean committed source with the pinned native tools. Each wrapper
exports the tracked Git tree; ignored files and checkout line-ending conversion
do not enter the source snapshot. Input hashes come from Git blobs. Compiler
flags remap source/Cargo/Rust paths, Cargo stays offline, and rustup automatic
installation is disabled. Native helper acquisition is a separate reviewed
prerequisite, never an automatic retry after a missing-cache error.

The macOS build runs its compiler/bundler children inside a network-denying
sandbox. Non-macOS helper preflight verifies bytes but is not a network firewall;
a future native build environment must provide its own reviewed isolation.
Recorded source/flags/tools describe the build, not a fully hermetic environment.

Each target's new output directory contains its declared files and one JSON
build manifest. It records the commit/tree, protected input digests, tool/SDK
versions, target/flags, helper inventory digest when applicable, dependency/license
inventory, and format-aware inspection results. Personal paths and raw logs are
not added to the manifest. Local compiler logs are not approved for publication.

The inspector checks extracted files, metadata and binary strings (including
UTF-16 Windows strings), executable architecture/version, user-data exclusions
and publisher-identity policy. ZIP modes/relative links are preserved; DMG app
contents must equal the ZIP. Debian and AppImage must carry the same executable.
AppImage extraction uses an external archive utility and never executes the
AppImage. Native tools must already be installed.

Scans are bounded pattern checks, not proof of absence of every secret or malicious
behavior. Static NSIS/configuration checks do not prove arbitrary installer control
flow. The generated NSIS script stays in the local build cache because it includes
build paths; its digest is recorded, but the transferred package cannot repeat
that particular policy inspection by itself.

Only a complete four-target set can be assembled:

```sh
python3 scripts/assemble_candidate.py assemble \
  --input-dir dist/candidates/SOURCE/macos-aarch64 \
  --input-dir dist/candidates/SOURCE/macos-x86_64 \
  --input-dir dist/candidates/SOURCE/windows-x86_64 \
  --input-dir dist/candidates/SOURCE/ubuntu-x86_64 \
  --output dist/candidates/SOURCE/complete
python3 scripts/assemble_candidate.py verify dist/candidates/SOURCE/complete
```

The output has seven distributables, four original build manifests, one combined
manifest and `SHA256SUMS`. Assembly rejects mixed source/version, missing/extra
files, redirects, incomplete recorded inspections and changed bytes. The verifier
checks the complete set independently of the build jobs. It does not rerun native
inspectors or authenticate unsigned metadata. An attacker replacing both a file
and its unsigned evidence is outside a checksum's guarantees.

Repeat builds must state whether compiler caches were reused. Compare executable
and normalized payload hashes separately from ZIP/DMG hashes; timestamps and
container layout may differ. No byte-reproducibility claim is made without a
successful comparison. Device execution is still P6 work.
