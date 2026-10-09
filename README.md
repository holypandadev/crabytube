<p align="center">
  <img src="assets/crabytube-logo.svg" width="128" alt="CrabyTube logo">
</p>

<h1 align="center">CrabyTube</h1>

<p align="center">
  A lightweight, native YouTube Music player written in Rust.<br>
  No browser. No account. Just music.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/BUILT_WITH-RUST-ffffff?style=flat-square&labelColor=000000&logo=rust&logoColor=white" alt="Built with Rust">
  <img src="https://img.shields.io/badge/PLATFORMS-LINUX_·_MACOS_·_WINDOWS-ffffff?style=flat-square&labelColor=000000" alt="Linux, macOS and Windows">
  <img src="https://img.shields.io/badge/LICENSE-GPL--3.0-ffffff?style=flat-square&labelColor=000000" alt="License GPL-3.0">
</p>

## Features

| `01` **LIGHTWEIGHT** | `02` **NATIVE** | `03` **NO ACCOUNT** |
| :--- | :--- | :--- |
| Under 100 MB of RAM while playing and a few dozen at rest. Idle CPU stays near zero. | Pure Rust and Iced. No browser, no Electron. Opens fast, and the next track is ready before the current one ends. | Plays YouTube Music anonymously. No sign-in, no cookies, no telemetry. Your data stays on your device. |
| `04` **SYNCED LYRICS** | `05` **BRUTALIST DESIGN** | `06` **FITS YOUR DESKTOP** |
| Line-by-line and word-by-word karaoke, with translations, from several sources at once. | Pure black, monospaced type and one-pixel outlines. The layout scales from a phone-sized window to 4K. | Media keys and MPRIS, tray icon, track notifications and Discord Rich Presence. English and Spanish. |

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/home.webp" alt="Home"></td>
    <td width="50%"><img src="docs/screenshots/search.webp" alt="Search results"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/screenshots/now-playing.webp" alt="Now Playing with synced lyrics"></td>
    <td width="50%"><img src="docs/screenshots/playlist.webp" alt="Playlist"></td>
  </tr>
</table>

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

CrabyTube is free software under the [GNU General Public License v3.0 or later](LICENSE).
Third-party code, assets and services are listed in [docs/licenses.md](docs/licenses.md).
Building from source is described in [docs/building.md](docs/building.md).
