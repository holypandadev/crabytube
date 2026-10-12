# Third-party notices

CrabyTube is licensed under GPL-3.0-or-later (see [`LICENSE`](../LICENSE)). It contains, links or
talks to the third-party components listed below, each under its own license. License texts that
ship with vendored code are kept next to that code.

## Vendored source code

| Component | Location | Version | License | Copyright |
| --- | --- | --- | --- | --- |
| `iced_tiny_skia` | `vendor/iced_tiny_skia/` | 0.14.1, modified | MIT | Héctor Ramón, Iced contributors |
| `rustypipe-botguard` | `botguard-helper/vendor/rustypipe-botguard/` | 0.1.2, modified | MIT | ThetaDev |

The changes to the two modified crates are listed in their `PATCHES.md` files:
[`vendor/iced_tiny_skia/PATCHES.md`](../vendor/iced_tiny_skia/PATCHES.md) and
[`botguard-helper/vendor/rustypipe-botguard/PATCHES.md`](../botguard-helper/vendor/rustypipe-botguard/PATCHES.md).
Both are used through `[patch.crates-io]` in the root `Cargo.toml`.

### JavaScript inside `rustypipe-botguard`

`botguard-helper` embeds the crate's JavaScript runtime files:

- `js/98_global_scope_shared.js`, `js/98_global_scope_window.js`: from Deno, MIT, Copyright the Deno
  authors.
- `js/bg_bundle.min.js.br`: a Brotli-compressed bundle. Its packages, as listed in
  `js/dependencies.txt`:

  | Package | Version | License |
  | --- | --- | --- |
  | `@asamuzakjp/css-color` | 2.8.3 | MIT |
  | `@csstools/color-helpers` | 5.0.1 | MIT-0 |
  | `@csstools/css-calc` | 2.1.1 | MIT |
  | `@csstools/css-color-parser` | 3.0.7 | MIT |
  | `@csstools/css-parser-algorithms` | 3.0.4 | MIT |
  | `@csstools/css-tokenizer` | 3.0.3 | MIT |
  | `@thetadevcode/jsdom-minimal` | 26.0.0 | MIT |
  | `bgutils-js` | 3.1.3 | MIT |
  | `cssstyle` | 4.2.1 | MIT |
  | `decimal.js` | 10.5.0 | MIT |
  | `entities` | 4.5.0 | BSD-2-Clause |
  | `is-potential-custom-element-name` | 1.0.1 | MIT |
  | `nwsapi` | 2.2.16 | MIT |
  | `rrweb-cssom` | 0.8.0 | MIT |
  | `saxes` | 6.0.0 | ISC |
  | `symbol-tree` | 3.2.4 | MIT |
  | `w3c-xmlserializer` | 5.0.0 | MIT |
  | `webidl-conversions` | 7.0.0 | BSD-2-Clause |
  | `whatwg-mimetype` | 4.0.0 | MIT |
  | `xml-name-validator` | 5.0.0 | Apache-2.0 |
  | `xmlchars` | 2.2.0 | MIT |

  In addition, the bundle contains the `BotGuardClient` and `WebPoMinter` classes of
  `bgutils-js` 4.0.3 (MIT, Copyright (c) 2024 LuanRT), which the helper uses in place of the 3.1.3
  copies.

## Copied source files

| Files | Origin | License |
| --- | --- | --- |
| `js-challenge-helper/src/polyfill/intl.js`, `setup.js` | `ytdlp-ejs` Rust crate by ahaoboy, commit `de624cbd4d8e7cf29f0dcb9b00b410991570d011` | MIT, Copyright (c) 2026 ahaoboy (notice in each file) |

## Fonts and artwork

- JetBrains Mono (`assets/fonts/JetBrainsMono-*.ttf`): SIL Open Font License 1.1, Copyright 2020
  The JetBrains Mono Project Authors. The license is in [`assets/fonts/OFL.txt`](../assets/fonts/OFL.txt).
- The application icon, logo and interface icons in `assets/` and `packaging/` are part of
  CrabyTube.

## Rust dependencies

The exact dependency graph and versions are recorded in `Cargo.lock`; `cargo metadata` or a tool
such as `cargo about` / `cargo deny` produces a complete per-crate list. The most significant
dependencies:

| Crate | Used by | License |
| --- | --- | --- |
| `iced` 0.14 (with `iced_wgpu`, `wgpu`, `winit`, `cosmic-text`, `resvg`) | app (GUI) | MIT; `wgpu`, `cosmic-text`, `resvg`: MIT OR Apache-2.0; `winit`: Apache-2.0 |
| `tiny-skia` 0.11 | app (CPU renderer) | BSD-3-Clause |
| `tokio`, `hyper`, `h2` | app, helpers | MIT |
| `reqwest` 0.13, `rustls` 0.23 | app, BotGuard helper | MIT OR Apache-2.0; `rustls`: Apache-2.0 OR ISC OR MIT |
| `aws-lc-rs` / `aws-lc-sys`, `ring` | TLS cryptography | ISC and Apache-2.0 (with MIT and BSD-3-Clause parts in `aws-lc-sys`) |
| `webpki-root-certs` | TLS root certificates | CDLA-Permissive-2.0 |
| `rodio` 0.22, `stream-download` 0.24 | app (playback) | MIT OR Apache-2.0 |
| `cpal` 0.17 | app (audio output) | Apache-2.0 |
| `symphonia` 0.5 | app (MP4 demuxing, AAC decoding) | MPL-2.0 |
| `image` 0.25, `zune-jpeg`, `png` | app (artwork) | MIT OR Apache-2.0 (`zune-jpeg` also Zlib) |
| `zbus` 5 | app (MPRIS, notifications; Linux) | MIT |
| `ksni` 0.3 | app (system tray; Linux) | Unlicense |
| `objc2` 0.6, `block2` 0.6, `objc2-foundation` 0.3 | app (Now Playing and media keys; macOS) | MIT |
| `objc2-app-kit` 0.3, `objc2-core-foundation` 0.3, `objc2-media-player` 0.3 | app (Now Playing and media keys; macOS) | Zlib OR Apache-2.0 OR MIT |
| `libc` 0.2 | app (process memory and CPU readings; macOS) | MIT OR Apache-2.0 |
| `discord-rich-presence` 1.1 | app (Discord Rich Presence) | MIT |
| `serde`, `serde_json`, `json5`, `roxmltree`, `sha2`, `dirs`, `rand` | app, helpers | MIT OR Apache-2.0 (`json5`: MIT) |
| `deno_core` 0.331, `deno_web`, `deno_console`, `deno_url`, `deno_webidl` | BotGuard helper | MIT |
| `v8` 130 (V8 engine) | BotGuard helper | MIT (V8 itself: BSD-3-Clause) |
| `brotli` | BotGuard helper | BSD-3-Clause AND MIT |
| `ytdlp-ejs` (Git, commit `de624cb`) | JS challenge helper | MIT |
| `rquickjs` 0.14 (QuickJS-NG) | JS challenge helper | MIT |
| `oxc_*` 0.153 | JS challenge helper | MIT |
| `swc_*` (dependencies of `ytdlp-ejs`) | JS challenge helper | Apache-2.0 |
| `winresource` | build script (Windows only) | MIT |

By license, the remaining crates are almost all under MIT and/or Apache-2.0. Other licenses that
appear in the graph: BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unlicense (dual with MIT), BSL-1.0
(Windows clipboard crates), Unicode-3.0 (`icu_*` and related crates), CC0-1.0, 0BSD and
MPL-2.0 (`symphonia`, `option-ext`, `cooked-waker`).

MPL-2.0 is a file-level copyleft license and is compatible with GPL-3.0; the MPL-covered source
files are available from their upstream crates.

## External services

CrabyTube is an unofficial client and is not affiliated with or endorsed by any of these services.
It talks to them at runtime; no code from them is distributed with CrabyTube.

| Service | Used for |
| --- | --- |
| YouTube Music / YouTube (Google) | Catalog, search, artwork, audio streams, and the BotGuard and player-script challenges that streams require. Used anonymously. |
| LRCLIB (`lrclib.net`) | Lyrics, when the Lyrics tab is open. |
| Better Lyrics (`api.betterlyrics.org`) | Timed lyrics, when the Lyrics tab is open. |
| Discord | Rich Presence through the local Discord client's IPC socket, only when enabled in Settings. |

YouTube and YouTube Music are trademarks of Google LLC. Discord is a trademark of Discord Inc.
Other names belong to their respective owners.
