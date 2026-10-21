# Building CrabyTube

## Requirements

- Rust 1.97 or newer (stable) and a C compiler.
- Linux: the development packages for ALSA, Wayland, X11 and xkbcommon, plus `pkg-config`.
  On Debian/Ubuntu:

  ```sh
  sudo apt install libasound2-dev libwayland-dev libx11-dev libx11-xcb-dev libxcb-shape0-dev \
    libxcb-xfixes0-dev libxcursor-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libxrandr-dev libvulkan-dev pkg-config
  ```

- Network access during the first build: the `v8` crate (used by `botguard-helper`) downloads a
  prebuilt V8 static library.

## Build and run

```sh
cargo build --release
cargo run --release
```

The workspace builds three executables, all of them default members:

- `crabytube`, the application;
- `botguard-helper`, which solves YouTube's BotGuard challenge and returns the proof-of-origin
  tokens that audio streams require;
- `js-challenge-helper`, which runs the YouTube player's signature and `n` transforms.

The helpers are short-lived processes started by the application. They must sit in the same
directory as the `crabytube` executable (`target/release/` after a build); the application never
looks for them in `PATH`. When you copy or package CrabyTube, copy all three executables together.

## Linux desktop integration

```sh
scripts/install-linux.sh                    # uses target/release/crabytube
scripts/install-linux.sh path/to/crabytube  # or another binary
scripts/install-linux.sh --uninstall
```

The script installs a launcher entry and the application icons for the current user under
`$XDG_DATA_HOME` (default `~/.local/share`). Wayland compositors take a window's icon from the
matching `.desktop` file, so install it to get the icon in docks and task switchers.

## Windows

The application icon and the version information are embedded in the three executables at build
time. Keep `botguard-helper.exe` and `js-challenge-helper.exe` next to `crabytube.exe`.

The Windows release has a zip with the three executables, the license and a first-launch note, and
an installer made from the same files (below). Build them on an x64 Windows PC with the MSVC Rust
toolchain and the Visual Studio C++ build tools:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-zip.ps1
```

The script compiles the workspace in release mode and writes `target\windows\CrabyTube-windows-x64.zip`
(the application is named `CrabyTube.exe` inside it) and prints its SHA-256. Nothing is signed;
`packaging/windows/README-first-launch.txt`, which goes into the zip, explains the SmartScreen
warning. `-NoBuild` packs the executables already in `target\release\`.

The Windows Installer package is made from the folder the zip comes from, with the WiX Toolset 5:

```powershell
dotnet tool install --global wix --version 5.0.2
powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1
```

It writes `target\windows\CrabyTube-<version>-windows-x64.msi` (`packaging/windows/crabytube.wxs`).
The package installs for the user who runs it, without an administrator password, into
`%LOCALAPPDATA%\Programs\CrabyTube`, with a Start menu entry and an entry in Settings → Apps. That
copy updates itself (see `src/updater.rs`).

## macOS

`packaging/macos/build-app.sh` builds the signed application bundle, and the zip and the disk image
for users:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
packaging/macos/build-app.sh
```

It compiles both architectures, joins them into universal binaries with `lipo`, puts the two helper
programs in `CrabyTube.app/Contents/MacOS/` next to the executable, writes the `Info.plist`, signs
the helpers and then the bundle ad hoc (no certificate, no hardened runtime, no notarization) and
zips the result into `target/macos/CrabyTube-<version>-macos-universal.zip`. The zip also holds
`README-first-launch.txt` (how to get past the first-start warning of macOS and where the data is),
`LICENSE.txt`, `THIRD-PARTY-LICENSES.md` and `THIRD-PARTY-LICENSES.txt`. The disk image,
`target/macos/CrabyTube-<version>-macos-universal.dmg`, holds the same files and a link to
Applications. Set `TARGETS=aarch64-apple-darwin` for a quick build of one architecture.

The MPRIS controls, track notifications and launch-on-startup option are only available on Linux;
the tray icon is in the notification area on Windows and in the menu bar on macOS. On macOS the media keys and Now Playing work from the application bundle. On Windows the
media keys and the media panel of the system flyout work from the executable, with no setup.

## Release builds on GitHub

The workflow `.github/workflows/release.yml` builds the five packages on GitHub's runners: the
Flatpak bundle (`packaging/linux/flatpak`), the Windows zip and installer
(`packaging/windows/build-zip.ps1` and `build-msi.ps1`) and the universal macOS zip and disk image
(`packaging/macos/build-app.sh`). It starts only by hand (**Actions → Release builds → Run
workflow**, or `gh workflow run release.yml`) and keeps the packages as artifacts of the run. It
publishes nothing, needs no secrets and only reads the repository.

The Flatpak is also published to CrabyTube's Flatpak repository, which GitHub Pages serves from
the repository `holypandadev/crabytube-flatpak` at `https://holypandadev.github.io/crabytube-flatpak/`.
`scripts/flatpak-repo.py` takes the workflow's bundle into the repository in a checkout of that
site, signs the commit and the summary with the repository's GnuPG key, and writes the bundle the
release carries again, with the repository's address and public key: a copy installed from it gets
its updates from the repository, as one installed with `crabytube.flatpakref` does. The site keeps
only the current version, so its checkout is committed as a single commit and pushed with
`--force`, which keeps the site repository small. The site goes up before the release is published:
the app learns of a version from `update.json` and then asks the repository for it.

A release also carries `update.json`, `update.json.sig` and `SHA256SUMS`, which
`scripts/make-update-manifest.py` writes from the packages and signs with the release key on the
release machine. Neither key goes into a repository or the workflow. Every third-party action and
the Flatpak container image are pinned by commit or digest; update them deliberately and keep the
version in the comment beside each one.

## Local data

CrabyTube stores everything on your device:

| What | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Settings, favorites, playlists, queue, search history (`state.json`), and playlist covers from the computer (`covers/`) | `~/.config/crabytube/` | `~/Library/Application Support/crabytube/` | `%APPDATA%\crabytube\` |
| Caches: artwork, lyrics, Home snapshot, player script, BotGuard snapshot and tokens, streaming buffer, an update being downloaded | `~/.cache/crabytube/` | `~/Library/Caches/crabytube/` | `%LOCALAPPDATA%\crabytube\` |

On Linux, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` are honoured, and "Launch on startup" writes
`~/.config/autostart/crabytube.desktop`. The caches are size-limited and can be inspected and cleared from
Settings → Storage.
