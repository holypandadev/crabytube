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

An application bundle can be built with `cargo-bundle`:

```sh
cargo install cargo-bundle
cargo bundle --release
```

Then copy `target/release/botguard-helper` and `target/release/js-challenge-helper` into
`CrabyTube.app/Contents/MacOS/`, next to the main executable.

The MPRIS controls, tray icon, track notifications and launch-on-startup option are only available
on Linux. On macOS the media keys and Now Playing work from the application bundle.

## Local data

CrabyTube stores everything on your device:

| What | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Settings, favorites, queue, search history (`state.json`) | `~/.config/crabytube/` | `~/Library/Application Support/crabytube/` | `%APPDATA%\crabytube\` |
| Caches: artwork, lyrics, Home snapshot, player script, BotGuard snapshot and tokens, streaming buffer | `~/.cache/crabytube/` | `~/Library/Caches/crabytube/` | `%LOCALAPPDATA%\crabytube\` |

On Linux, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` are honoured, and "Launch on startup" writes
`~/.config/autostart/crabytube.desktop`. The caches are size-limited and can be inspected and cleared from
Settings → Storage.
