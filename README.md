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
| Line-by-line and word-by-word karaoke, with translations, from several sources at once. | Pure black, monospaced type and one-pixel outlines. The layout scales from a phone-sized window to 4K. | Media keys and the system media controls on all three systems; on Linux also a tray icon and track notifications. Discord Rich Presence. Nine languages: English, Spanish, Portuguese, French, German, Italian, Russian, Japanese and Chinese. |

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/home.webp" alt="Home"></td>
    <td width="50%"><img src="docs/screenshots/search.webp" alt="Search results with live broadcasts"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/screenshots/now-playing.webp" alt="Now Playing with the Up Next queue"></td>
    <td width="50%"><img src="docs/screenshots/playlist.webp" alt="Playlist"></td>
  </tr>
</table>

## Install

Each release on the GitHub Releases page has one file per platform. Nothing is signed with a paid
certificate, so your system asks you to confirm the first start once.

### Linux

Download the `.flatpak` file and install it for your user:

```bash
flatpak install --user CrabyTube.flatpak
```

If Flatpak asks for a runtime, allow it; it comes from Flathub. Then start CrabyTube from your
application menu.

### Windows

1. Download the `.zip` file and extract it to a folder of your choice. Keep all the files together.
2. Run `CrabyTube.exe`. Windows SmartScreen shows "Windows protected your PC" because the app is not
   signed. Click **More info**, then **Run anyway**. You only need to do this once.

Nothing else needs to be installed: the Visual C++ runtime is built into the executables.

### macOS

1. Download the `.zip` file, extract it and move `CrabyTube.app` to `/Applications`. The app is
   universal and runs natively on Apple Silicon and Intel Macs.
2. The app is signed ad hoc and not notarized, so macOS blocks the first start. Allow it once:
   - On macOS 15 and later, double-click `CrabyTube.app` and click **Done** in the message that
     says it was not opened. Then open **System Settings → Privacy & Security**, scroll to
     **Security**, click **Open Anyway** next to the message about CrabyTube and confirm with your
     password or Touch ID. Control-click → **Open** no longer skips the warning there.
   - On macOS 14 and earlier, Control-click `CrabyTube.app`, choose **Open**, then **Open** again.
   - Or run this once in Terminal, then open the app normally:

     ```bash
     xattr -dr com.apple.quarantine /Applications/CrabyTube.app
     ```

   The zip also contains a short first-launch note, the license and the third-party notices.

## Privacy

- No account and no login. CrabyTube uses YouTube Music anonymously and stores no credentials or
  cookies.
- No telemetry, analytics or crash reporting.
- Network requests go only to the services needed for the features you use: YouTube Music,
  YouTube and Google servers (catalog, artwork, audio and the BotGuard challenge), and the lyrics
  services LRCLIB and Better Lyrics (see below).
  Discord Rich Presence (off by default) talks only to the Discord client running on your computer.
- Lyrics come from two services. LRCLIB (lrclib.net) receives the track's title and artist. Better
  Lyrics (api.betterlyrics.org) receives the title, the artist, the album and the duration when they
  are known, and the YouTube video ID; its QQ Music and Kugou lookups go through the same service.
  Lyrics are requested when the Lyrics tab is open, and also during the last 30 seconds of a track
  for the next track in the queue, so they are ready when that track starts. Fetched lyrics are
  cached on this computer for 30 days.
- The track that plays and the next one are downloaded to a temporary folder in the cache directory
  while they play. Files left behind by a crash are removed when CrabyTube starts, once they are old
  enough not to be in use.
- The `CRABYTUBE_NETWORK_DIAGNOSTICS` environment variable, meant for troubleshooting, makes one
  request to `api64.ipify.org` and logs only the address family and a truncated hash of the public
  address. It is off unless you set it.

## Translations

The interface follows the system language on the first start and can be changed in Settings. If a
translation reads wrong in your language, an issue with the correction is welcome.

## Disclaimer

CrabyTube is an unofficial client. It is not affiliated with, endorsed by, or sponsored by Google
or YouTube. YouTube and YouTube Music are trademarks of Google LLC; other trademarks belong to their
respective owners. CrabyTube is intended for personal use. It streams music for listening and has
no download feature; audio is only kept temporarily in the cache while it plays.

## License

CrabyTube is free software under the [GNU General Public License v3.0 or later](LICENSE).
Third-party code, assets and services are listed in [docs/licenses.md](docs/licenses.md).
Building from source is described in [docs/building.md](docs/building.md).
