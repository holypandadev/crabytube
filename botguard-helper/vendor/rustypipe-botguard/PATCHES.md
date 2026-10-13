# rustypipe-botguard 0.1.2: local changes

- Base: crates.io `rustypipe-botguard` 0.1.2 (<https://codeberg.org/ThetaDev/rustypipe-botguard>).
- License: MIT, Copyright (c) 2025 ThetaDev (see `LICENSE`).
- The root `Cargo.toml` uses this copy through `[patch.crates-io]`; it is linked only into
  `botguard-helper`.

`README.md`, `Cargo.lock`, `src/main.rs`, `src/error.rs`, `src/protobuf.rs`, `src/runtime.rs`,
`js/98_global_scope_*.js` and `js/dependencies.txt` are unchanged from the published crate. The
crate's `CHANGELOG.md` is not included.

## Changes

### `Cargo.toml`

- `reqwest` 0.12 → 0.13 with default features disabled, using `rustls` for TLS (plus `brotli`,
  `charset`, `gzip`, `http2`, `json`, `system-proxy`), so the helper shares the application's TLS
  stack and does not depend on native TLS libraries.
- `CHANGELOG.md` removed from `include`.

### `src/lib.rs`

- `BotguardBuilder::homepage_challenge(ytcfg_json, challenge_json)`: initialise from a BotGuard
  challenge and `ytcfg` taken from the same YouTube homepage response instead of fetching a new
  challenge. The config is installed as the `yt` global (on `globalThis` and `window`) before the
  interpreter runs. Both values must be given together; without them the upstream challenge request
  is used.
- The builder's user agent is also passed to the JavaScript `runBotguard` call (upstream always
  passes its default user agent there).
- The integrity token is requested from
  `https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT` instead of
  `https://www.youtube.com/api/jnn/v1/GenerateIT`.
- Diagnostic lines on stderr: the challenge source and payload sizes, and the HTTP status and JSON
  shape of the GenerateIT response (a `json_type` helper names the JSON value type). No token or
  identifier values are printed.
- V8 flags set once per process, before the first isolate whether or not it starts from a
  snapshot, to keep the single short-lived isolate small (`--initial-old-space-size=8
  --max-old-space-size=96 --max-semi-space-size=1 --optimize-for-size --single-threaded`), and
  `--jitless`, so the downloaded program runs without machine code generated at run time. The
  snapshot magic number differs from upstream's, so a snapshot taken with other flags is not read.
- The BotGuard interpreter is fetched over HTTPS only.
- The snapshot file is read with bounds (64 KiB of header, 64 MiB of data, and no more data than
  the header announces), removed when it cannot be read, and written readable by the user alone.
- Environment variables read when building the HTTP client:
  `CRABYTUBE_FORCE_IPV4` binds to an IPv4 local address, `CRABYTUBE_HTTP1_ONLY` disables HTTP/2.

### `js/99_main.js`

- Adds `globalThis.__crabytubeInstallYtcfg(rawConfig)`, which parses the homepage `ytcfg` JSON and
  installs it as `yt.config_` on `globalThis` and `window`. It is called from `src/lib.rs` when
  `homepage_challenge` is used.

### `js/bg_bundle.min.js.br`

- The Brotli-compressed bundle (JSDOM + bgutils-js) additionally contains the `BotGuardClient` and
  `WebPoMinter` classes from bgutils-js 4.0.3 (MIT, Copyright (c) 2024 LuanRT), exposed as
  `globalThis.__CRABYTUBE_BgUtils`. `runBotguard` and `newMinter` use these classes (with the
  `globalObject` option of the 4.x API) instead of the bundle's bgutils-js 3.1.3 copies.

### `src/lib.rs`

- `write_snapshot_file` writes beside the target and renames it into place, so a crash mid-write keeps
  the previous snapshot. The temporary name carries the process id, so writers never share one.
