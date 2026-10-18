#!/usr/bin/env python3
"""Publishes a CrabyTube Flatpak to its Flatpak repository, and writes the bundle a release carries.

    python3 scripts/flatpak-repo.py --version 0.1.2 --bundle <CrabyTube.flatpak from the release
        workflow> --site <checkout of the site> --out <folder with the release's packages>

1. The bundle's commit goes into <site>/repo, an OSTree repository made on first use. It is signed
   with the repository key, and the summary and the AppStream data that software centers read are
   written again; objects no commit needs any more are removed.
2. <out>/CrabyTube.flatpak is written again from the repository, with the repository's address and
   public key: a copy installed from it gets later versions from the repository, in Discover,
   GNOME Software, `flatpak update` and the app's own Update now. Its runtime comes from Flathub.
3. The site gets crabytube.flatpakref (installs the app and adds the repository), crabytube.flatpakrepo
   (adds the repository), the public key, an index page and .nojekyll, so that GitHub Pages serves
   every file of the repository as it is.

The site is the checkout of the GitHub Pages repository. This script changes files only; committing
and pushing them is the next step of the release (docs/building.md).

The repository key is a GnuPG key that never goes into a repository. By default it is the only
secret key in ~/.config/crabytube-release/flatpak-gnupg. For a test, --site-url names another
address, such as a local web server, and --gnupg another key.
"""

import argparse
import base64
import html
import pathlib
import subprocess
import sys

APP_ID = "dev.holypanda.CrabyTube"
BRANCH = "master"
SITE_URL = "https://holypandadev.github.io/crabytube-flatpak"
HOMEPAGE = "https://github.com/holypandadev/crabytube"
RUNTIME_REPO = "https://dl.flathub.org/repo/flathub.flatpakrepo"
DEFAULT_GNUPG = pathlib.Path.home() / ".config/crabytube-release/flatpak-gnupg"


def run(*command, **options):
    return subprocess.run(command, check=True, **options)


def secret_key(gnupg):
    """The fingerprint of the one secret key in the GnuPG home."""
    listing = run(
        "gpg", "--homedir", str(gnupg), "--list-secret-keys", "--with-colons",
        capture_output=True, text=True,
    ).stdout
    fingerprints = []
    in_secret_key = False
    for line in listing.splitlines():
        fields = line.split(":")
        if fields[0] == "sec":
            in_secret_key = True
        elif fields[0] == "fpr" and in_secret_key:
            fingerprints.append(fields[9])
            in_secret_key = False
    if len(fingerprints) != 1:
        sys.exit(f"{gnupg} must hold exactly one secret key, not {len(fingerprints)}")
    return fingerprints[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--bundle", required=True, type=pathlib.Path)
    parser.add_argument("--site", required=True, type=pathlib.Path)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--site-url", default=SITE_URL)
    parser.add_argument("--gnupg", type=pathlib.Path, default=DEFAULT_GNUPG)
    args = parser.parse_args()

    site_url = args.site_url.rstrip("/")
    repo_url = f"{site_url}/repo/"
    repo = args.site / "repo"
    key = secret_key(args.gnupg)
    gpg = [f"--gpg-sign={key}", f"--gpg-homedir={args.gnupg}"]

    public_key = args.site / "crabytube.gpg"
    args.site.mkdir(parents=True, exist_ok=True)
    with open(public_key, "wb") as file:
        run("gpg", "--homedir", str(args.gnupg), "--export", key, stdout=file)
    key_base64 = base64.b64encode(public_key.read_bytes()).decode()

    if not (repo / "config").is_file():
        run("ostree", "init", "--mode=archive-z2", f"--repo={repo}")
    run("flatpak", "build-import-bundle", *gpg, "--update-appstream", str(repo), str(args.bundle))
    run(
        "flatpak", "build-update-repo", *gpg,
        "--title=CrabyTube",
        "--comment=CrabyTube, a lightweight YouTube Music player",
        f"--homepage={HOMEPAGE}",
        f"--default-branch={BRANCH}",
        "--prune",
        str(repo),
    )

    args.out.mkdir(parents=True, exist_ok=True)
    bundle = args.out / "CrabyTube.flatpak"
    run(
        "flatpak", "build-bundle",
        f"--repo-url={repo_url}",
        f"--runtime-repo={RUNTIME_REPO}",
        f"--gpg-keys={public_key}",
        str(repo), str(bundle), APP_ID, BRANCH,
    )

    (args.site / "crabytube.flatpakref").write_text(
        "[Flatpak Ref]\n"
        f"Name={APP_ID}\n"
        f"Branch={BRANCH}\n"
        "Title=CrabyTube\n"
        f"Url={repo_url}\n"
        "SuggestRemoteName=crabytube\n"
        f"RuntimeRepo={RUNTIME_REPO}\n"
        "IsRuntime=false\n"
        f"Homepage={HOMEPAGE}\n"
        f"GPGKey={key_base64}\n",
        encoding="utf-8",
    )
    (args.site / "crabytube.flatpakrepo").write_text(
        "[Flatpak Repo]\n"
        "Title=CrabyTube\n"
        "Comment=CrabyTube, a lightweight YouTube Music player\n"
        f"Url={repo_url}\n"
        f"Homepage={HOMEPAGE}\n"
        f"GPGKey={key_base64}\n",
        encoding="utf-8",
    )
    (args.site / ".nojekyll").write_text("", encoding="utf-8")
    ref_url = html.escape(f"{site_url}/crabytube.flatpakref")
    (args.site / "index.html").write_text(
        f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CrabyTube Flatpak repository</title>
</head>
<body>
<h1>CrabyTube Flatpak repository</h1>
<p>The Flatpak of <a href="{html.escape(HOMEPAGE)}">CrabyTube</a> {html.escape(args.version)}.
Installed from here, CrabyTube gets its updates in your software center, with
<code>flatpak update</code> and from its own Settings.</p>
<p>Install it by opening <a href="crabytube.flatpakref">crabytube.flatpakref</a>, or with:</p>
<pre>flatpak install --user {ref_url}</pre>
<p>The commits and the summary are signed with the key in
<a href="crabytube.gpg">crabytube.gpg</a>.</p>
</body>
</html>
""",
        encoding="utf-8",
    )

    print(f"CrabyTube {args.version} is in {repo}, signed with {key}")
    print(f"  bundle with the repository's address: {bundle}")
    print(f"  site files written in {args.site}")


if __name__ == "__main__":
    main()
