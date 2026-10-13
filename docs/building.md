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

The Windows release is a zip with the three executables, the license and a first-launch note. Build
it on an x64 Windows PC with the MSVC Rust toolchain and the Visual Studio C++ build tools:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-zip.ps1
```

The script compiles the workspace in release mode and writes `target\windows\CrabyTube-windows-x64.zip`
(the application is named `CrabyTube.exe` inside it) and prints its SHA-256. Nothing is signed;
`packaging/windows/README-first-launch.txt`, which goes into the zip, explains the SmartScreen
warning. `-NoBuild` packs the executables already in `target\release\`.

## macOS

`packaging/macos/build-app.sh` builds the signed application bundle and the zip for users:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
packaging/macos/build-app.sh
```

It compiles both architectures, joins them into universal binaries with `lipo`, puts the two helper
programs in `CrabyTube.app/Contents/MacOS/` next to the executable, writes the `Info.plist`, signs
the helpers and then the bundle ad hoc (no certificate, no hardened runtime, no notarization) and
zips the result into `target/macos/CrabyTube-<version>-macos-universal.zip`. The zip also holds
`README-first-launch.txt` (how to get past the first-start warning of macOS and where the data is),
`LICENSE.txt`, `THIRD-PARTY-LICENSES.md` and `THIRD-PARTY-LICENSES.txt`. Set `TARGETS=aarch64-apple-darwin` for a quick build of
one architecture.

The MPRIS controls, tray icon, track notifications and launch-on-startup option are only available
on Linux. On macOS the media keys and Now Playing work from the application bundle. On Windows the
media keys and the media panel of the system flyout work from the executable, with no setup.

## Release builds on GitHub

The workflow `.github/workflows/release.yml` builds the three packages on GitHub's runners: the
Flatpak bundle (`packaging/linux/flatpak`), the Windows zip (`packaging/windows/build-zip.ps1`) and
the universal macOS zip (`packaging/macos/build-app.sh`). It starts only by hand (**Actions →
Release builds → Run workflow**, or `gh workflow run release.yml`) and keeps the packages as
artifacts of the run. It publishes nothing, needs no secrets and only reads the repository. Every
third-party action and the Flatpak container image are pinned by commit or digest; update them
deliberately and keep the version in the comment beside each one.

## Local data

CrabyTube stores everything on your device:

| What | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Settings, favorites, queue, search history (`state.json`) | `~/.config/crabytube/` | `~/Library/Application Support/crabytube/` | `%APPDATA%\crabytube\` |
| Caches: artwork, lyrics, Home snapshot, player script, BotGuard snapshot and tokens, streaming buffer | `~/.cache/crabytube/` | `~/Library/Caches/crabytube/` | `%LOCALAPPDATA%\crabytube\` |

On Linux, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` are honoured, and "Launch on startup" writes
`~/.config/autostart/crabytube.desktop`. The caches are size-limited and can be inspected and cleared from
Settings → Storage.
