#!/usr/bin/env python3
"""Writes the license texts of everything the release binaries contain.

The packages ship this file beside the app. It lists every crate the binaries link on Linux,
Windows and macOS (normal dependencies only: build tools and test crates are not shipped), with
the license files each crate publishes. Texts that several crates share are written once, after
the names of the crates that use them. The font and the code kept in this repository follow.

Run it from the repository root after a change to Cargo.lock:

    python3 scripts/third-party-licenses.py > packaging/THIRD-PARTY-LICENSES.txt
"""

import json
import pathlib
import subprocess
import sys

TARGETS = [
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]
LICENSE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense", "copyright")
ROOT = pathlib.Path(__file__).resolve().parent.parent

# Texts the binaries carry besides those the crates publish: the font, and V8 and its ICU data,
# which the v8 crate builds but whose package leaves their license files out (both kept in
# packaging/licenses/, from the V8 13.0.245.12 and ICU 74.2 sources).
LOCAL_TEXTS = [
    ("JetBrains Mono (font)", "assets/fonts/OFL.txt"),
    ("V8 13.0 (inside the v8 crate)", "packaging/licenses/v8.txt"),
    ("ICU data (inside the v8 and deno_core_icudata crates)", "packaging/licenses/icu.txt"),
]


def shipped_packages():
    """Package ids reached from the workspace through normal dependencies, on any release target."""
    packages = {}
    reached = set()
    workspace = set()
    for target in TARGETS:
        output = subprocess.run(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--filter-platform",
                target,
            ],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        metadata = json.loads(output)
        for package in metadata["packages"]:
            packages[package["id"]] = package
        workspace.update(metadata["workspace_members"])
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        # Each target is walked on its own: a crate seen for one target still leads to the
        # dependencies it has only on the next.
        seen = set()
        pending = list(metadata["workspace_members"])
        while pending:
            current = pending.pop()
            if current in seen:
                continue
            seen.add(current)
            reached.add(current)
            for dependency in nodes[current]["deps"]:
                kinds = {kind["kind"] for kind in dependency["dep_kinds"]}
                if None in kinds:
                    pending.append(dependency["pkg"])
    # The app's own crates are covered by its license; code vendored into the repository is not.
    return [packages[package_id] for package_id in reached - workspace]


def license_files(package):
    folder = pathlib.Path(package["manifest_path"]).parent
    found = []
    for path in sorted(folder.iterdir()):
        if path.is_file() and path.name.lower().startswith(LICENSE_PREFIXES):
            found.append(path)
    if package.get("license_file"):
        named = folder / package["license_file"]
        if named.is_file() and named not in found:
            found.append(named)
    return found


def main():
    texts = {}  # text -> names of the crates that publish it
    without_files = []
    for package in sorted(shipped_packages(), key=lambda p: (p["name"], p["version"])):
        name = f"{package['name']} {package['version']}"
        files = license_files(package)
        if not files:
            license_name = package.get("license") or "no license declared"
            authors = ", ".join(package.get("authors") or []) or "its authors"
            without_files.append(f"{name}: {license_name}, copyright {authors}")
            continue
        for path in files:
            text = path.read_text(encoding="utf-8", errors="replace").strip()
            texts.setdefault(text, []).append(f"{name} ({path.name})")
    for label, relative in LOCAL_TEXTS:
        text = (ROOT / relative).read_text(encoding="utf-8").strip()
        texts.setdefault(text, []).append(label)

    out = sys.stdout
    out.write("CrabyTube: licenses of third-party software\n")
    out.write("============================================\n\n")
    out.write("CrabyTube is licensed under GPL-3.0-or-later (see LICENSE). The binaries also contain\n")
    out.write("the software below. Each license text follows the list of the parts it covers.\n\n")
    if without_files:
        out.write("These crates publish no license file; their declared license applies, and its\n")
        out.write("text is among those below:\n\n")
        for entry in without_files:
            out.write(f"  {entry}\n")
        out.write("\n")
    for text, users in sorted(texts.items(), key=lambda item: item[1][0].lower()):
        out.write("-" * 78 + "\n")
        for user in users:
            out.write(f"{user}\n")
        out.write("-" * 78 + "\n\n")
        out.write(text + "\n\n")


if __name__ == "__main__":
    main()
