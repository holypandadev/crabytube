#!/usr/bin/env python3
"""Writes the files a release carries for updates and checksums, from the packages in one folder.

    python3 scripts/make-update-manifest.py --version 0.1.1 --dir <folder with the packages>

The folder holds the packages as the release workflow builds them:

    CrabyTube.flatpak
    CrabyTube-windows-x64.zip
    CrabyTube-<version>-windows-x64.msi
    CrabyTube-<version>-macos-universal.zip
    CrabyTube-<version>-macos-universal.dmg

and gets three files to attach to the release beside them:

    update.json      the version, the release page, and the address, size and SHA-256 of the
                     packages the app installs itself (the .msi and the macOS zip) and of the
                     Windows zip
    update.json.sig  the Ed25519 signature of update.json by the release key, which the app checks
                     before it uses anything in update.json (src/updater.rs)
    SHA256SUMS       the SHA-256 of every package and of update.json, for people who check by hand

The release key is a PEM file that never goes into the repository. By default it is read from
~/.config/crabytube-release/update-signing-key.pem. Signing needs OpenSSL 3.

For a test feed, --base-url and --notes-url point the description elsewhere, for example at a
local web server, which the app reads with CRABYTUBE_UPDATE_FEED set.
"""

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

REPOSITORY = "https://github.com/holypandadev/crabytube"
DEFAULT_KEY = pathlib.Path.home() / ".config/crabytube-release/update-signing-key.pem"


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--dir", required=True, type=pathlib.Path)
    parser.add_argument("--key", type=pathlib.Path, default=DEFAULT_KEY)
    parser.add_argument("--base-url")
    parser.add_argument("--notes-url")
    args = parser.parse_args()

    version = args.version
    parts = version.split(".")
    if len(parts) != 3 or not all(part.isdigit() for part in parts):
        sys.exit(f"the version must be major.minor.patch, not {version!r}")
    folder = args.dir
    base_url = (args.base_url or f"{REPOSITORY}/releases/download/v{version}").rstrip("/")
    notes_url = args.notes_url or f"{REPOSITORY}/releases/tag/v{version}"

    # Kinds the app knows (src/updater.rs), and the file each one is.
    kinds = {
        "windows-msi": f"CrabyTube-{version}-windows-x64.msi",
        "windows-zip": "CrabyTube-windows-x64.zip",
        "macos-zip": f"CrabyTube-{version}-macos-universal.zip",
    }
    missing = [name for name in kinds.values() if not (folder / name).is_file()]
    if missing:
        sys.exit(f"missing in {folder}: {', '.join(missing)}")

    packages = []
    for kind, name in kinds.items():
        path = folder / name
        packages.append(
            {
                "kind": kind,
                "url": f"{base_url}/{name}",
                "size": path.stat().st_size,
                "sha256": sha256(path),
            }
        )
    manifest = {"version": version, "notes_url": notes_url, "packages": packages}
    manifest_path = folder / "update.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    signature_path = folder / "update.json.sig"
    subprocess.run(
        [
            "openssl", "pkeyutl", "-sign", "-rawin",
            "-inkey", str(args.key),
            "-in", str(manifest_path),
            "-out", str(signature_path),
        ],
        check=True,
    )
    # The signature is checked with the public half, as the app checks it.
    with tempfile.TemporaryDirectory() as scratch:
        public = pathlib.Path(scratch) / "public.pem"
        subprocess.run(
            ["openssl", "pkey", "-in", str(args.key), "-pubout", "-out", str(public)],
            check=True,
        )
        subprocess.run(
            [
                "openssl", "pkeyutl", "-verify", "-rawin", "-pubin",
                "-inkey", str(public),
                "-in", str(manifest_path),
                "-sigfile", str(signature_path),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
    if signature_path.stat().st_size != 64:
        sys.exit("the signature is not an Ed25519 signature: is the key an Ed25519 key?")

    sums = [
        "CrabyTube.flatpak",
        "CrabyTube-windows-x64.zip",
        f"CrabyTube-{version}-windows-x64.msi",
        f"CrabyTube-{version}-macos-universal.zip",
        f"CrabyTube-{version}-macos-universal.dmg",
        "update.json",
    ]
    lines = [f"{sha256(folder / name)}  {name}" for name in sums if (folder / name).is_file()]
    (folder / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"CrabyTube {version}: update.json, update.json.sig and SHA256SUMS in {folder}")
    for line in lines:
        print(f"  {line}")


if __name__ == "__main__":
    main()
