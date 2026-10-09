# CrabyTube

CrabyTube is a lightweight native desktop player for YouTube Music, written in Rust with
the Iced GUI toolkit. It plays music without a browser, Electron or a Google
account, and is built to keep memory and CPU use low.

## Features

- **Home** with public YouTube Music shelves. A small snapshot is kept so the next launch can show
  it immediately while it refreshes.
- **Search** with suggestions and filters for songs, albums, artists and playlists, with incremental
  loading of more results.
- **Album, artist and playlist pages**, including long playlists loaded page by page.
- **Playback** of AAC audio streams: play/pause, seek, volume and mute, repeat, shuffle, radio
  ("autoplay") and a choice of audio output device. The next track is prepared while the current one
  plays, so track changes are quick.
- **Now Playing** with the Up Next queue and synced lyrics (line- or word-timed when available, with
  translations), from LRCLIB and Better Lyrics.
- **Favorite songs** and a restorable queue, stored locally.
- **Settings** for appearance, playback, audio, lyrics, integrations and storage, with per-cache
  usage and clearing, and backup/import of local data.
- Interface in English and Spanish.
- **Linux desktop integration:** MPRIS media controls, a system tray icon with optional
  close-to-tray, optional track notifications and launch on startup.
- Optional **Discord Rich Presence** through the local Discord client.

Keyboard: `Space` play/pause, `←`/`→` seek, `Ctrl+K` search, `Esc` back.

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

### Linux desktop integration

```sh
scripts/install-linux.sh                    # uses target/release/crabytube
scripts/install-linux.sh path/to/crabytube  # or another binary
scripts/install-linux.sh --uninstall
```

The script installs a launcher entry and the application icons for the current user under
`$XDG_DATA_HOME` (default `~/.local/share`). Wayland compositors take a window's icon from the
matching `.desktop` file, so install it to get the icon in docks and task switchers.

### Windows

The application icon is embedded in the executable at build time. Keep `botguard-helper.exe` and
`js-challenge-helper.exe` next to `crabytube.exe`.

### macOS

An application bundle can be built with `cargo-bundle`:

```sh
cargo install cargo-bundle
cargo bundle --release
```

Then copy `target/release/botguard-helper` and `target/release/js-challenge-helper` into
`CrabyTube.app/Contents/MacOS/`, next to the main executable.

The MPRIS controls, tray icon, track notifications and launch-on-startup option are only available
on Linux.

## Local data

CrabyTube stores everything on your device:

| What | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Settings, favorites, queue, search history (`state.json`) | `~/.config/crabytube/` | `~/Library/Application Support/crabytube/` | `%APPDATA%\crabytube\` |
| Caches: artwork, lyrics, Home snapshot, player script, BotGuard snapshot and tokens, streaming buffer | `~/.cache/crabytube/` | `~/Library/Caches/crabytube/` | `%LOCALAPPDATA%\crabytube\` |

On Linux, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` are honoured, and "Launch on startup" writes
`~/.config/autostart/crabytube.desktop`. The caches are size-limited and can be inspected and cleared from
Settings → Storage.

## Privacy

- No account and no login. CrabyTube uses YouTube Music anonymously and stores no credentials or
  cookies.
- No telemetry, analytics or crash reporting.
- Network requests go only to the services needed for the features you use: YouTube Music,
  YouTube and Google servers (catalog, artwork, audio and the BotGuard challenge), and LRCLIB and
  Better Lyrics when the Lyrics tab is open (they receive the track's title and artist; Better
  Lyrics also receives the album, duration and YouTube video ID).
  Discord Rich Presence (off by default) talks only to the Discord client running on your computer.
- The `CRABYTUBE_NETWORK_DIAGNOSTICS` environment variable, meant for troubleshooting, makes one
  request to `api64.ipify.org` and logs only the address family and a truncated hash of the public
  address. It is off unless you set it.

## Disclaimer

CrabyTube is an unofficial client. It is not affiliated with, endorsed by, or sponsored by Google
or YouTube. YouTube and YouTube Music are trademarks of Google LLC; other trademarks belong to their
respective owners. CrabyTube is intended for personal use. It streams music for listening and has
no download feature; audio is only buffered temporarily while it plays.

## License

CrabyTube is licensed under the GNU General Public License, version 3 or (at your option) any later
version. See [LICENSE](LICENSE).

Third-party code, assets and services are listed in [docs/licenses.md](docs/licenses.md). An
overview of the code is in [docs/architecture.md](docs/architecture.md).
