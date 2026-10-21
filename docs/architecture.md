# Architecture

CrabyTube is a Cargo workspace with three executables and one small library:

| Crate | Path | Role |
| --- | --- | --- |
| `crabytube` | `src/` | The desktop application: GUI, catalog client, playback, local state. |
| `botguard-helper` | `botguard-helper/` | Solves YouTube's BotGuard challenge and mints proof-of-origin (PO) tokens. |
| `js-challenge-helper` | `js-challenge-helper/` | Runs the YouTube player's signature and `n` parameter transforms. |
| `helper-guard` | `helper-guard/` | Library both helpers use: the limits a helper puts on itself, so one its app has abandoned does not keep running. |

Two vendored crates are patched in through `[patch.crates-io]`: `iced_tiny_skia` (renderer fixes,
see `vendor/iced_tiny_skia/PATCHES.md`) and `rustypipe-botguard` (see
`botguard-helper/vendor/rustypipe-botguard/PATCHES.md`).

## Application modules

| Module | Responsibility |
| --- | --- |
| `main.rs` | Process entry: allocator tuning on Linux/glibc, logging (`logging.rs`; `crabytube.log` on Windows and macOS), renderer selection, the single-instance check, start of the GUI and, once it ends, the Windows installer of a downloaded update. |
| `ui.rs`, `ui/` | Iced application (`App`): state, `update`, `view`, subscriptions. |
| `music.rs`, `music/browse.rs` | `MusicService` trait and its YouTube Music implementation: Home feed, search and suggestions, radio, album/artist/playlist pages and continuations. |
| `playback/` | `resolver.rs` (stream resolution), `pot.rs` (PO tokens via the BotGuard helper), `engine.rs` (playback actor, HTTP streaming, decoding, output), `output.rs` (audio device enumeration and opening). |
| `network.rs` | Shared HTTP client and the switches for network troubleshooting. |
| `runtime_cache.rs` | Web client values (`ytcfg`) persisted for six hours to avoid fetching the Music page on every launch. |
| `home_cache.rs` | Size-capped snapshot of the public Home feed, painted on the next launch while it refreshes. |
| `artwork.rs` | Artwork downloads, size variants, in-memory LRU and disk cache. |
| `lyrics.rs` | Lyrics lookup, parsing (LRC, TTML, QRC) and cache. |
| `preferences.rs` | Versioned local state (`state.json`): preferences, queue, favorites, playlists, history; autostart entry. |
| `storage.rs`, `temp_cleanup.rs` | Cache usage reporting and selective clearing; removal of leftover temporary files. |
| `memory.rs` | Shrinks the app's memory use while a BotGuard helper process is running. |
| `i18n/` | Interface translations: English, Spanish, Brazilian Portuguese, French, German, Italian, Russian, Japanese and Simplified Chinese. English is the key; each language has a file of exact entries and templates for text with numbers or names. |
| `mpris.rs`, `tray.rs`, `notifications.rs` | Linux MPRIS, StatusNotifier tray and desktop notifications over D-Bus (`zbus`, `ksni`). |
| `tray_native.rs` | The tray icon of Windows (notification area) and macOS (menu bar), through `tray-icon`, made on the main thread once the first window opens and removed before the app exits. On macOS it also answers the Dock's and Spotlight's reopen request and takes the app out of the Dock while the window is hidden. Other platforms use `tray_stub.rs`. |
| `mpris_macos.rs`, `media_session.rs` | The macOS media keys and Now Playing through the MediaPlayer framework (`objc2-media-player`). `media_session.rs` holds what the macOS and Windows backends share: the player state, the commands, and the rules for what to tell the system and when. |
| `mpris_windows.rs` | The Windows media keys and media panel through the System Media Transport Controls (`windows`). Other platforms use `mpris_stub.rs`. |
| `discord.rs` | Optional Discord Rich Presence over Discord's local socket (Unix) or named pipe (Windows), through a small client of its own: every place a Discord client listens is tried, sockets only, and Discord's answer to each activity is read, so a refusal shows in Settings and the log. Texts are fitted to Discord's limits. |
| `updater.rs`, `ui/updates.rs` | Daily check of the latest release's `update.json`, used only with a valid Ed25519 signature; downloads the package and checks its size and SHA-256. On Windows the `.msi` installs after the app has exited; on macOS the `.app` bundle is swapped; the Flatpak has the Flatpak portal's update monitor install the new version from CrabyTube's Flatpak repository and then starts it through the portal's `Spawn`, while the new process waits for the old one to quit (`--after-update`); other copies open the release page. Linux builds outside the Flatpak do not check. |
| `single_instance.rs` | One running copy: a second launch asks the first to show its window and exits. |
| `app_dirs.rs`, `flatpak.rs`, `background_portal.rs` | Data folders, Flatpak detection, and launch on startup through the Background portal inside the Flatpak. |
| `playback/live.rs` | Live broadcasts, which have no length and cannot be sought. |

Catalog and playback code does not depend on Iced; the UI talks to them through `MusicService`,
`StreamResolver` and `PlaybackHandle`.

## Catalog access

`YouTubeMusicService` uses YouTube Music's web client (`WEB_REMIX`) anonymously. It loads the
`music.youtube.com` page once, extracts the live `ytcfg` (client version, API key, visitor data,
context) and then posts Innertube requests (`browse`, `search`, `next`, suggestions). Responses are
parsed into small domain types (`Track`, `HomeShelf`, `HomeItem`, `Collection`, ...). Response sizes,
item counts, text lengths and continuation tokens are bounded, and raw responses are never stored.

## Playback pipeline

```text
Track (video ID)
  -> YouTubeStreamResolver
       live ytcfg + player script (signature timestamp)
       PO tokens               <- botguard-helper (cached, see below)
       /player request         (runs in parallel with token minting)
       AAC format selection
       signature + n transform <- js-challenge-helper
       one-byte range preflight against googlevideo
  -> ValidatedPlaybackResolution  (URL accepted by the preflight, with its expiry)
  -> playback actor (own thread and Tokio runtime)
       stream-download: HTTP range reader, buffered to a temporary file
       decoder thread: Symphonia (MP4/AAC) -> f32 sample chunks
       bounded channel -> Rodio source -> CPAL output device
```

- **Resolution.** `StreamResolver::resolve` returns a `ValidatedPlaybackResolution`, which can only
  be created after googlevideo accepted a small range request. It records the URL's `expire` time so
  stale URLs are rejected before decoding starts. Media URLs are never persisted. Idempotent
  configuration requests (Music page, watch page, player script) are retried a bounded number of
  times on transient failures. Player and media requests are not retried on their own; a
  resolution whose stream googlevideo refuses is attempted again a bounded number of times.
- **Tokens.** Session and content PO tokens are cached in memory and in `pot.json` (owner-only
  permissions) until shortly before they expire. When the helper runs, it also mints content
  tokens for the next tracks in the queue, so later tracks usually need no helper run.
- **Engine.** The playback actor owns the stream, decoder and output. It publishes a
  `PlaybackStatus` (state, position, duration, volume, underruns, device, format) on a watch
  channel; the UI subscribes to it and does not poll. Each track gets a new generation number, so
  events from a superseded or cancelled track are ignored. The next track in the queue is resolved,
  opened and decoded ahead of time. Seeking reopens the range reader at the new position. A 403
  from googlevideo during playback (an expired URL) triggers a fresh resolution. After a few
  seconds of pause the output device is released and reopened on resume. Choosing another output in
  Settings moves the playing track there at once. When the chosen device goes away, playback
  continues on the system default and returns to the device when it is back; with no device at
  all, playback pauses.
- **Errors** are typed (`StreamError`: network, unavailable or login required, PO token, cipher,
  media rejected, unsupported format, decoder, device, cancelled). The UI shows a short message;
  logs keep the details with URLs, tokens and identifiers replaced by short fingerprints.

## Helper processes

Both helpers are separate executables that read one JSON request on stdin, write one JSON response
on stdout and exit. The application looks for them only next to its own executable (never in
`PATH`), and kills them on cancellation or timeout.

They are separate processes because of their cost and their inputs:

- **`botguard-helper`** embeds V8 through `deno_core` and a minimal browser environment (JSDOM) to
  run Google's BotGuard program. This adds tens of megabytes of code and memory that would
  otherwise stay resident for the lifetime of the player. As a short-lived process its memory is
  returned to the system as soon as the tokens are minted. It writes a V8 snapshot of the
  initialised environment (`botguard-*.bin` in the cache directory, at most eight files / 16 MiB) so
  later runs start faster.
- **`js-challenge-helper`** runs YouTube's player JavaScript, which is untrusted and changes
  frequently. It parses the player script with OXC, extracts the needed functions, prepends small
  polyfills and evaluates the result in an embedded QuickJS runtime (`rquickjs`, via `ytdlp-ejs`).
  The preprocessed script is written to the cache (`player-js/`), keyed by the player URL and a
  SHA-256 fingerprint of its source, so later runs skip the parse. Requests and outputs are size
  bounded.

On Linux/glibc the application passes conservative allocator settings to the JS helper unless the
caller already set them. While a BotGuard helper is running, the application reduces its own
in-memory artwork budget (`memory.rs`).

## User interface

The UI is an Iced 0.14 daemon (`App` in `ui.rs`) using the message/update/view model and a custom
Tokio executor (two worker threads by default, `TOKIO_WORKER_THREADS` overrides it). It opens its
window with `window::open` and keeps running without one: close-to-tray, on by default, hides the
window on Windows and macOS and, on Linux, closes it, because Wayland cannot hide one; the tray
opens it again. Wayland lets no client raise, focus or restore its own window either, so there
a window that is behind others or minimized is closed and opened again when the tray, a second
launch or MPRIS `Raise` asks for it. When the window is minimized or closed to the tray, cached
covers other than the playing track's are dropped and the freed heap is returned.

- `ui/shell.rs`: window chrome: title bar, tabs and the player bar. The window is undecorated on
  Linux and Windows; on macOS the system frame stays, with a transparent, untitled bar that the
  app's bar fills.
- Pages (`ui/messages.rs::Page`): Home and Search (`ui/home.rs`), the Library and the page of each
  of the user's lists (`ui/library.rs`), album/artist/playlist detail (`ui/collection.rs`), Now
  Playing with Up Next and Lyrics (`ui/now_playing.rs`) and Settings (`ui/settings.rs`).
- `ui/playlists.rs`: the user's lists. Favorite songs is always there; the user makes, edits,
  orders and deletes playlists. Every heart opens the Save panel, which puts its song in any list
  or in a new one. `ui/drag.rs` lets list rows and playlists be dragged to another place.
- `ui/playback_glue.rs`, `ui/artwork_glue.rs`: queue, radio and playback commands; artwork requests
  and fades.
- `ui/subscriptions.rs`: playback status, timers, window and keyboard events.
- `ui/theme.rs`: design tokens and widget styles (monospace font, square outlines, no shadows or
  gradients), kept cheap to rasterise on the CPU renderer.
- `ui/layout.rs`: pure layout calculations, unit tested without a window.
- `ui/diagnostics.rs`, `ui/timing.rs`: development tools, compiled only with the `diagnostics`
  feature.

Long lists (queue, playlists, favorites, lyrics) build widgets only for the visible rows. Requests carry generation
numbers or cancellation tokens so that results arriving after the user moved on are discarded.

## Rendering

Both Iced renderers are compiled in. By default `ICED_BACKEND` is set to `tiny-skia`, Iced's CPU
renderer, which avoids a GPU context and its memory. The "GPU acceleration" setting switches to
`wgpu` with tiny-skia as fallback on the next start. An explicit `ICED_BACKEND` in the environment
takes precedence.

The vendored `iced_tiny_skia` collapses many small damage rectangles into one repaint, clips cached
text to its viewport, and snaps quads and images to physical pixels; see its `PATCHES.md`.

## Local state and caches

All paths come from the `dirs` crate (`~/.config/crabytube` and `~/.cache/crabytube` on Linux).

| Data | Location | Bounds |
| --- | --- | --- |
| Preferences, queue, favorites, playlists, search history, lyric offsets | `config/crabytube/state.json` | 16 MiB file; 2,000 queued tracks; 10,000 favorites; 100 playlists of up to 2,000 songs, 10,000 in all; 500 per-track lyric offsets |
| Web client values (`ytcfg`) | `cache/crabytube/runtime.json` | Six hours; dropped on HTTP 403 |
| Home snapshot (public metadata only) | `cache/crabytube/home-v1-<lang>.json` | 2 MiB |
| Artwork | `cache/crabytube/artwork/` | 64 MiB / 400 files on disk; 16 MiB / 192 decoded images in memory |
| Lyrics | `cache/crabytube/lyrics/` | 30-day lifetime, 2 MiB in total |
| Player script (raw and preprocessed) | `cache/crabytube/player-js/` | Latest player versions only; 8 MiB per preprocessed file |
| BotGuard snapshots, PO tokens | `cache/crabytube/botguard-*.bin`, `pot.json` | 8 files / 16 MiB; tokens until expiry |
| Player configuration | `cache/crabytube/player-config.json` | Six hours |
| Streaming buffer | `cache/crabytube/stream/` | Temporary files for the current tracks |
| Update download | `cache/crabytube/updates/` | One package, removed at the next start |
| Log (Windows and macOS) | `crabytube.log` in the local data folder | Moved to `crabytube.log.1` past 1 MiB |

`state.json` is written by a single background writer that coalesces rapid changes and writes
through a temporary file. Nothing in local state contains credentials, cookies, signed media URLs or
raw service responses. Settings → Storage shows the size of each cache and can clear them
individually.

## Platform integration

- **Linux:** MPRIS (`org.mpris.MediaPlayer2.crabytube`) with metadata, the saved cover file and controls; a
  StatusNotifier tray icon; desktop notifications; launch on startup
  through `~/.config/autostart`, or the Background portal inside the Flatpak. The window's application ID is `dev.holypanda.CrabyTube`, matching
  `packaging/linux/dev.holypanda.CrabyTube.desktop`.
- **Windows:** media keys and the system media panel (title, artist, cover, position) through the
  System Media Transport Controls. The icon is embedded by `build.rs` (`winresource`).
  `packaging/windows/build-zip.ps1` builds the portable zip, and `build-msi.ps1` a per-user `.msi`
  from `crabytube.wxs`, whose Start menu entry carries the app id the installed copy gives itself.
- **macOS:** media keys, headphone buttons and Control Center through `MPRemoteCommandCenter`, and Now Playing (title, artist, album, cover, position and a progress bar that advances by itself) through `MPNowPlayingInfoCenter`. The cover is read from the file the app already saved, never from a URL the system downloads. `packaging/macos/build-app.sh` builds the universal app bundle (macOS 11 or later, ad hoc signature) and the release zip and disk image.
- **All platforms:** a tray icon (the StatusNotifier item on Linux, the notification area on Windows, the menu bar on macOS) with close-to-tray, on by default; optional Discord Rich Presence.

## Development tools

The `diagnostics` feature adds command-line test modes (`--scroll-test`, `--soak`, `--autoplay`,
`--screenshots`, `--exit-after`), environment overrides for the helper paths, a lyric fixture for
the screenshots, fault injection, and Iced's debug overlay. Release builds contain none of these,
apart from a few environment switches: for network troubleshooting `CRABYTUBE_FORCE_IPV4`,
`CRABYTUBE_HTTP1_ONLY`, `CRABYTUBE_UA_PROFILE` and `CRABYTUBE_NETWORK_DIAGNOSTICS` (which the
BotGuard helper reads); `CRABYTUBE_JS_PROFILE`, which makes the JS helper report its memory use at each stage; and
`CRABYTUBE_UPDATE_FEED`, which points the update check at a test feed, whose signature is still
required. `scripts/peak-rss.sh` and
`scripts/soak-memory-cpu.sh` measure memory and CPU on Linux.
