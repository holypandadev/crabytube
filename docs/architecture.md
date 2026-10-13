# Architecture

CrabyTube is a Cargo workspace with three executables:

| Crate | Path | Role |
| --- | --- | --- |
| `crabytube` | `src/` | The desktop application: GUI, catalog client, playback, local state. |
| `botguard-helper` | `botguard-helper/` | Solves YouTube's BotGuard challenge and mints proof-of-origin (PO) tokens. |
| `js-challenge-helper` | `js-challenge-helper/` | Runs the YouTube player's signature and `n` parameter transforms. |

Two vendored crates are patched in through `[patch.crates-io]`: `iced_tiny_skia` (renderer fixes,
see `vendor/iced_tiny_skia/PATCHES.md`) and `rustypipe-botguard` (see
`botguard-helper/vendor/rustypipe-botguard/PATCHES.md`).

## Application modules

| Module | Responsibility |
| --- | --- |
| `main.rs` | Process entry: allocator tuning on Linux/glibc, logging (`tracing`, `RUST_LOG`), renderer selection, start of the GUI. |
| `ui.rs`, `ui/` | Iced application (`App`): state, `update`, `view`, subscriptions. |
| `music.rs`, `music/browse.rs` | `MusicService` trait and its YouTube Music implementation: Home feed, search and suggestions, radio, album/artist/playlist pages and continuations. |
| `playback/` | `resolver.rs` (stream resolution), `pot.rs` (PO tokens via the BotGuard helper), `engine.rs` (playback actor, HTTP streaming, decoding, output), `output.rs` (audio device enumeration and opening). |
| `network.rs` | Shared HTTP client and the switches for network troubleshooting. |
| `runtime_cache.rs` | Web client values (`ytcfg`) persisted for six hours to avoid fetching the Music page on every launch. |
| `home_cache.rs` | Size-capped snapshot of the public Home feed, painted on the next launch while it refreshes. |
| `artwork.rs` | Artwork downloads, size variants, in-memory LRU and disk cache. |
| `lyrics.rs` | Lyrics lookup, parsing (LRC, TTML, QRC) and cache. |
| `preferences.rs` | Versioned local state (`state.json`): preferences, queue, favorites, history; autostart entry. |
| `storage.rs`, `temp_cleanup.rs` | Cache usage reporting and selective clearing; removal of leftover temporary files. |
| `memory.rs` | Shrinks the app's memory use while a BotGuard helper process is running. |
| `i18n/` | Interface translations: English, Spanish, Brazilian Portuguese, French, German, Italian, Russian, Japanese and Simplified Chinese. English is the key; each language has a file of exact entries and templates for text with numbers or names. |
| `mpris.rs`, `tray.rs`, `notifications.rs` | Linux MPRIS, StatusNotifier tray and desktop notifications over D-Bus (`zbus`, `ksni`). Other platforms use `tray_stub.rs`. |
| `mpris_macos.rs`, `media_session.rs` | The macOS media keys and Now Playing through the MediaPlayer framework (`objc2-media-player`). `media_session.rs` holds what is not specific to one system: the player state, the commands, and the rules for what to tell the system and when. |
| `mpris_windows.rs` | The Windows media keys and media panel through the System Media Transport Controls (`windows`). Other platforms use `mpris_stub.rs`. |
| `discord.rs` | Optional Discord Rich Presence over the local Discord IPC socket. |

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
  seconds of pause the output device is released and reopened on resume. When the device goes
  away, the output is opened again on the system default, or playback pauses if there is none.
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

The UI is an Iced 0.14 application (`App` in `ui.rs`) using the message/update/view model and a
custom Tokio executor (two worker threads by default, `TOKIO_WORKER_THREADS` overrides it).

- `ui/shell.rs`: window chrome (the window is undecorated): title bar, tabs and the player bar.
- Pages (`ui/messages.rs::Page`): Home and Search (`ui/home.rs`), Favorites (`ui/favorites.rs`),
  album/artist/playlist detail (`ui/collection.rs`), Now Playing with Up Next and Lyrics
  (`ui/now_playing.rs`) and Settings (`ui/settings.rs`).
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
| Preferences, queue, favorites, search history, lyric offsets | `config/crabytube/state.json` | 16 MiB file; 2,000 queued tracks; 10,000 favorites; 500 per-track lyric offsets |
| Web client values (`ytcfg`) | `cache/crabytube/runtime.json` | Six hours; dropped on HTTP 403 |
| Home snapshot (public metadata only) | `cache/crabytube/home-v1-<lang>.json` | 2 MiB |
| Artwork | `cache/crabytube/artwork/` | 64 MiB / 400 files on disk; 16 MiB / 192 decoded images in memory |
| Lyrics | `cache/crabytube/lyrics/` | 30-day lifetime, 2 MiB in total |
| Player script (raw and preprocessed) | `cache/crabytube/player-js/` | Latest player versions only; 8 MiB per preprocessed file |
| BotGuard snapshots, PO tokens | `cache/crabytube/botguard-*.bin`, `pot.json` | 8 files / 16 MiB; tokens until expiry |
| Streaming buffer | `cache/crabytube/stream/` | Temporary files for the current tracks |

`state.json` is written by a single background writer that coalesces rapid changes and writes
through a temporary file. Nothing in local state contains credentials, cookies, signed media URLs or
raw service responses. Settings → Storage shows the size of each cache and can clear them
individually.

## Platform integration

- **Linux:** MPRIS (`org.mpris.MediaPlayer2.crabytube`) with metadata, the saved cover file and controls; a
  StatusNotifier tray icon with optional close-to-tray; desktop notifications; launch on startup
  through `~/.config/autostart`. The window's application ID is `dev.holypanda.CrabyTube`, matching
  `packaging/linux/dev.holypanda.CrabyTube.desktop`.
- **Windows:** media keys and the system media panel (title, artist, cover, position) through the
  System Media Transport Controls. The icon is embedded by `build.rs` (`winresource`).
- **macOS:** media keys, headphone buttons and Control Center through `MPRemoteCommandCenter`, and Now Playing (title, artist, album, cover, position and a progress bar that advances by itself) through `MPNowPlayingInfoCenter`. The cover is read from the file the app already saved, never from a URL the system downloads. `packaging/macos/build-app.sh` builds the signed universal app bundle.
- **All platforms:** optional Discord Rich Presence.

## Development tools

The `diagnostics` feature adds command-line test modes (`--scroll-test`, `--soak`, `--autoplay`,
`--screenshots`, `--exit-after`), environment overrides for the helper paths, a lyric fixture for
the screenshots, fault injection, and Iced's debug overlay. Release builds contain none of these,
apart from a few environment switches for network troubleshooting (`CRABYTUBE_FORCE_IPV4`,
`CRABYTUBE_HTTP1_ONLY`, `CRABYTUBE_UA_PROFILE`, and `CRABYTUBE_NETWORK_DIAGNOSTICS`, which the
BotGuard helper reads). `scripts/peak-rss.sh` and
`scripts/soak-memory-cpu.sh` measure memory and CPU on Linux.
