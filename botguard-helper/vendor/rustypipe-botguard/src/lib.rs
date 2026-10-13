#![doc = include_str!("../README.md")]
#![warn(missing_docs, clippy::todo, clippy::dbg_macro)]

use std::io::{Cursor, Read, Write};
use std::ops::DerefMut;
use std::path::PathBuf;
use std::time::Instant;
use std::{
    fs::File,
    io::BufReader,
    ops::Deref,
    path::Path,
    sync::{Once, OnceLock},
};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use deno_core::{v8, JsRuntime, JsRuntimeForSnapshot, RuntimeOptions};
use reqwest::{header, Client, Url};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::runtime::TPerm;

mod error;
mod protobuf;
mod runtime;

pub use crate::error::Error;

enum Rt {
    FromSnapshot(JsRuntime),
    Snapshotting(JsRuntimeForSnapshot),
    NoSnapshot(JsRuntime),
}

impl Deref for Rt {
    type Target = JsRuntime;

    fn deref(&self) -> &Self::Target {
        match self {
            Rt::FromSnapshot(rt) | Rt::NoSnapshot(rt) => rt,
            Rt::Snapshotting(rt) => rt,
        }
    }
}

impl DerefMut for Rt {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Rt::FromSnapshot(rt) | Rt::NoSnapshot(rt) => rt,
            Rt::Snapshotting(rt) => rt,
        }
    }
}

/// Builder to construct a new RustyPipe Botguard client
#[derive(Default)]
pub struct BotguardBuilder<'a> {
    snapshot_path: Option<&'a Path>,
    user_agent: Option<&'a str>,
    homepage_config_json: Option<String>,
    homepage_challenge_json: Option<String>,
}

/// RustyPipe Botguard client
pub struct Botguard {
    rt: Rt,
    snapshot_path: Option<PathBuf>,
    created_at: OffsetDateTime,
    lifetime: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotInfo {
    rustypipe_botguard: String,
    v8: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    lifetime: u32,
}

struct SnapshotData {
    data: Box<[u8]>,
    info: SnapshotInfo,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChallengeData {
    #[serde(flatten)]
    interpreter_js: InterpreterJs,
    program: String,
    global_name: String,
}

#[derive(Debug)]
struct ResolvedChallengeData {
    interpreter_js: String,
    program: String,
    global_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum InterpreterJs {
    InterpreterUrl {
        #[serde(rename = "privateDoNotAccessOrElseTrustedResourceUrlWrappedValue")]
        url: String,
    },
    InterpreterJavascript {
        #[serde(rename = "privateDoNotAccessOrElseSafeScriptWrappedValue")]
        script: String,
    },
}

impl SnapshotInfo {
    fn is_valid(&self) -> bool {
        self.rustypipe_botguard == VERSION
            && self.v8 == v8::VERSION_STRING
            && (self.created_at
                + time::Duration::seconds(i64::from(self.lifetime).saturating_sub(600))
                > OffsetDateTime::now_utc())
    }
}

/// Version of the RustyPipe Botguard crate
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// First bytes of a Botguard snapshot file
const SNAPSHOT_MAGIC: u32 = 0x18cba459;

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// Note: this has to be a Webkit user agent, using other user agents results in invalid tokens
const DEFAULT_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.10 Safari/605.1.1";
const GOOG_API_KEY: &str = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
const REQUEST_KEY: &str = "O43z0dpjhgX20SCx4KAo";

const CONTENT_TYPE: &str = "application/json+protobuf";
const X_USER_AGENT: &str = "grpc-web-javascript/0.1";

static SNAPSHOT_DATA: OnceLock<SnapshotData> = OnceLock::new();

impl<'a> BotguardBuilder<'a> {
    /// Create a new [`BotguardBuilder`]
    ///
    /// This is the same as [`Botguard::builder`]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the path where the Botguard snapshot is stored.
    #[must_use]
    pub fn snapshot_path(mut self, snapshot_path: &'a Path) -> Self {
        self.snapshot_path = Some(snapshot_path);
        self
    }

    /// Set/Unset the path where the Botguard snapshot is stored.
    #[must_use]
    pub fn snapshot_path_opt(mut self, snapshot_path: Option<&'a Path>) -> Self {
        self.snapshot_path = snapshot_path;
        self
    }

    /// Set the user agent used for requesting the PO token.
    ///
    /// **Default value**: `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36(KHTML, like Gecko)`
    /// (Webkit)
    #[must_use]
    pub fn user_agent(mut self, user_agent: &'a str) -> Self {
        self.user_agent = Some(user_agent);
        self
    }

    /// Set/Unset the user agent used for requesting the PO token.
    ///
    /// **Default value**: `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36(KHTML, like Gecko)`
    /// (Webkit)
    #[must_use]
    pub fn user_agent_opt(mut self, user_agent: Option<&'a str>) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Use an initial BotGuard challenge and ytcfg extracted from the same
    /// YouTube homepage response. The config is installed into both `yt`
    /// globals before the BotGuard interpreter runs.
    #[must_use]
    pub fn homepage_challenge(
        mut self,
        ytcfg_json: impl Into<String>,
        challenge_json: impl Into<String>,
    ) -> Self {
        self.homepage_config_json = Some(ytcfg_json.into());
        self.homepage_challenge_json = Some(challenge_json.into());
        self
    }

    /// Initialize a new [`Botguard`] instance
    ///
    /// If a recent snapshot is found, the Botguard instance is recreated from the snapshot.
    ///
    /// Otherwise, a new Botguard challenge is fetched and solved.
    pub async fn init(self) -> Result<Botguard, Error> {
        apply_v8_memory_flags();
        if let Some(snapshot_path) = &self.snapshot_path {
            if SNAPSHOT_DATA.get().is_none() && snapshot_path.is_file() {
                match read_snapshot_file(snapshot_path) {
                    Ok(Some(snapshot)) => {
                        log::info!("loaded snapshot data ({} bytes)", snapshot.data.len());
                        _ = SNAPSHOT_DATA.set(snapshot);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::error!("{e}");
                        // A damaged snapshot would fail the same way at every start until it expires.
                        let _ = std::fs::remove_file(snapshot_path);
                    }
                }
            }

            if let Some(snapshot) = SNAPSHOT_DATA.get() {
                return Ok(Botguard {
                    rt: Rt::FromSnapshot(JsRuntime::try_new(RuntimeOptions {
                        extensions: rt_extensions(true),
                        startup_snapshot: Some(&snapshot.data),
                        ..Default::default()
                    })?),
                    snapshot_path: self.snapshot_path.map(Path::to_owned),
                    created_at: snapshot.info.created_at,
                    lifetime: snapshot.info.lifetime,
                });
            }
        }

        let mut client_builder = Client::builder()
            .user_agent(self.user_agent.unwrap_or(DEFAULT_UA))
            .gzip(true)
            .brotli(true);
        if std::env::var_os("CRABYTUBE_FORCE_IPV4").is_some() {
            client_builder = client_builder.local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        }
        if std::env::var_os("CRABYTUBE_HTTP1_ONLY").is_some() {
            client_builder = client_builder.http1_only();
        }
        let client = client_builder.build()?;

        let mut last_err = Error::InvalidChallenge("cannot init".into());
        for _ in 0..3 {
            match self.try_init(&client).await {
                Ok(bg) => {
                    return Ok(bg);
                }
                Err(e) => {
                    log::error!("{e}");
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }

    async fn try_init(&self, client: &Client) -> Result<Botguard, Error> {
        let mut rt = if self.snapshot_path.is_some() {
            Rt::Snapshotting(JsRuntimeForSnapshot::try_new(RuntimeOptions {
                extensions: rt_extensions(false),
                ..Default::default()
            })?)
        } else {
            Rt::NoSnapshot(JsRuntime::try_new(RuntimeOptions {
                extensions: rt_extensions(false),
                ..Default::default()
            })?)
        };

        rt.load_code().await?;

        if let Some(ytcfg_json) = &self.homepage_config_json {
            rt.call_js_fn(b"__crabytubeInstallYtcfg", &[ytcfg_json]).await?;
        }

        let created_at = OffsetDateTime::now_utc();
        // globalThis.runBotguard = async (interpreterJavascript, program, globalName, userAgent)
        let (challenge_data, config_prefix) = match (
            &self.homepage_config_json,
            &self.homepage_challenge_json,
        ) {
            (Some(_), Some(challenge_json)) => {
                let challenge = serde_json::from_str::<ChallengeData>(challenge_json)
                    .map_err(|e| Error::InvalidChallenge(e.to_string().into()))?;
                let prefix = "if(globalThis.window)globalThis.window.yt=globalThis.yt;\n".to_owned();
                let challenge = resolve_challenge_data(client, challenge).await?;
                (challenge, prefix)
            }
            (None, None) => (resolve_challenge_data(client, get_challenge(client).await?).await?, String::new()),
            _ => return Err(Error::InvalidChallenge("homepage ytcfg and challenge must be provided together".into())),
        };

        let interpreter_js = format!("{config_prefix}{}", challenge_data.interpreter_js);
        eprintln!(
            "BotGuard challenge source={} ytcfg_bytes={} program_bytes={} interpreter_bytes={} event_id_present={}",
            if self.homepage_challenge_json.is_some() { "homepage" } else { "legacy" },
            self.homepage_config_json.as_ref().map_or(0, String::len),
            challenge_data.program.len(),
            interpreter_js.len(),
            self.homepage_config_json
                .as_ref()
                .is_some_and(|config| config.contains("\"EVENT_ID\"")),
        );
        let bg_response = rt
            .call_js_fn_str(
                b"runBotguard",
                &[
                    &interpreter_js,
                    &challenge_data.program,
                    &challenge_data.global_name,
                    self.user_agent.unwrap_or(DEFAULT_UA),
                ],
            )
            .await?;

        let response = client
            .post("https://jnn-pa.googleapis.com/$rpc/google.internal.waa.v1.Waa/GenerateIT")
            .header(header::CONTENT_TYPE, CONTENT_TYPE)
            .header("x-goog-api-key", GOOG_API_KEY)
            .header("x-user-agent", X_USER_AGENT)
            .json(&[REQUEST_KEY, &bg_response])
            .send()
            .await?;
        let status = response.status();
        let resp = response.error_for_status()?.json::<serde_json::Value>().await?;
        eprintln!(
            "GenerateIT response status={} shape={}",
            status.as_u16(),
            match &resp {
                serde_json::Value::Array(items) => format!(
                    "array_len_{}_first_{}",
                    items.len(),
                    items.first().map_or("missing", json_type)
                ),
                serde_json::Value::Object(items) => {
                    format!("object_keys_{}", items.keys().count())
                }
                value => json_type(value).to_owned(),
            }
        );
        let resp_array = resp
            .as_array()
            .ok_or(Error::InvalidResponse("array expected".into()))?;
        let integrity_token = resp_array[0].as_str().ok_or(Error::InvalidResponse(
            "could not get integrity token".into(),
        ))?;
        let lifetime = resp_array[1]
            .as_u64()
            .ok_or(Error::InvalidResponse("could not get lifetime".into()))?;

        rt.call_js_fn(b"newMinter", &[integrity_token]).await?;

        // Test botguard
        {
            let vdata = "Cgs4bFZSaUotYTYtQSiJnvu8BjIKCgJERRIEEgAgFw==";
            let po_token = rt.call_js_fn_str(b"mint", &[vdata]).await?;
            validate_potoken(&po_token)
                .map_err(|e| Error::InvalidPoToken(format!("check failed: {e}").into()))?;
        }

        Ok(Botguard {
            rt,
            snapshot_path: self.snapshot_path.map(Path::to_owned),
            created_at,
            lifetime: lifetime as u32,
        })
    }
}

impl Botguard {
    /// Create a new [`BotguardBuilder`]
    ///
    /// This is the same as [`BotguardBuilder::new`]
    #[must_use]
    pub fn builder<'a>() -> BotguardBuilder<'a> {
        BotguardBuilder::new()
    }

    /// Return true if the Botguard instance was recreated from a snapshot
    pub fn is_from_snapshot(&self) -> bool {
        matches!(self.rt, Rt::FromSnapshot(_))
    }

    /// Get the creation date of the Botguard instance
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    /// Return the lifetime of the Botguard instance and its tokens in seconds
    pub fn lifetime(&self) -> u32 {
        self.lifetime
    }

    /// Return the expiry date of the Botguard instance and its tokens
    pub fn valid_until(&self) -> OffsetDateTime {
        self.created_at + Duration::seconds(self.lifetime.into())
    }

    /// Generate a new PO token from an identifier
    ///
    /// For a session-bound token used for YouTube stream URLs, use the visitor data ID as an identifier
    ///
    /// For a content-bound token used for YouTube player requests (`serviceIntegrityDimensions.poToken`
    /// parameter), use the video ID as an identifier.
    pub async fn mint_token(&mut self, ident: &str) -> Result<String, Error> {
        let ident_urldec = urlencoding::decode(ident).unwrap_or(ident.into());
        let po_token = self.rt.call_js_fn_str(b"mint", &[&ident_urldec]).await?;
        validate_potoken(&po_token)?;
        Ok(po_token)
    }

    /// Save a snapshot of the Botguard runtime
    ///
    /// Returns `true` if a snapshot has been successfully written.
    ///
    /// This function does nothing if no snapshot path has been specified or the instance
    /// has been created from a snapshot.
    ///
    /// Snapshotting consumes the runtime, so this function has to be run after using Botguard.
    pub async fn write_snapshot(self) -> bool {
        if let Rt::Snapshotting(rt) = self.rt {
            let mark = Instant::now();
            let snapshot = rt.snapshot();
            log::info!(
                "Snapshot size: {}, took {:#?}",
                snapshot.len(),
                mark.elapsed(),
            );
            let info = SnapshotInfo {
                rustypipe_botguard: VERSION.to_owned(),
                v8: v8::VERSION_STRING.to_owned(),
                created_at: self.created_at,
                lifetime: self.lifetime,
            };
            match write_snapshot_file(self.snapshot_path.as_deref().unwrap(), &info, &snapshot) {
                Ok(_) => {
                    log::debug!("snapshot written to {:?}", self.snapshot_path);
                    true
                }
                Err(e) => {
                    log::error!("could not write snapshot: {e}");
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Rt {
    async fn call_js_fn(
        &mut self,
        function: &'static [u8],
        args: &[&str],
    ) -> Result<v8::Global<v8::Value>, Error> {
        let js_fn: v8::Global<v8::Function> = {
            let context = self.main_context();
            let scope = &mut self.handle_scope();
            let context_local = v8::Local::new(scope, context);
            let global_obj = context_local.global(scope);
            let name_str = v8::String::new_external_onebyte_static(scope, function).unwrap();
            let func = global_obj
                .get(scope, name_str.into())
                .and_then(|x| x.try_cast().ok())
                .ok_or_else(|| {
                    Error::Js(
                        format!("function {} not found", String::from_utf8_lossy(function)).into(),
                    )
                })?;
            v8::Global::new(scope, func)
        };

        let arg_values = {
            let scope = &mut self.handle_scope();
            args.iter()
                .map(|arg| {
                    let s = v8::String::new(scope, arg)
                        .ok_or(Error::Js("could not create arg".into()))?;
                    Ok(v8::Global::new(scope, s.cast()))
                })
                .collect::<Result<Vec<v8::Global<v8::Value>>, Error>>()
        }?;
        let result_fut = self.call_with_args(&js_fn, &arg_values);
        let res = self
            .with_event_loop_promise(result_fut, Default::default())
            .await?;
        Ok(res)
    }

    async fn call_js_fn_str(
        &mut self,
        function: &'static [u8],
        args: &[&str],
    ) -> Result<String, Error> {
        let res = self.call_js_fn(function, args).await?;
        let scope = &mut self.handle_scope();
        Ok(res.open(scope).to_rust_string_lossy(scope))
    }

    async fn load_code(&mut self) -> Result<(), Error> {
        let code = bg_bundle();
        let mid = self
            .load_main_es_module_from_code(&Url::parse("file:///bg_bundle.min.js").unwrap(), code)
            .await
            .unwrap();
        let mut receiver = self.mod_evaluate(mid);
        tokio::select! {
            // Not using biased mode leads to non-determinism for relatively simple
            // programs.
            biased;

            maybe_result = &mut receiver => {
                log::debug!("received module evaluate {:#?}", maybe_result);
                maybe_result
            }

            event_loop_result = self.run_event_loop(Default::default()) => {
                event_loop_result.unwrap();
                receiver.await
            }
        }?;
        Ok(())
    }
}

fn bg_bundle() -> String {
    let bg_bundle: &[u8] = include_bytes!("../js/bg_bundle.min.js.br");
    let mut res = Vec::new();
    brotli::BrotliDecompress(&mut Cursor::new(bg_bundle), &mut res).unwrap();
    unsafe { String::from_utf8_unchecked(res) }
}

fn rt_extensions(from_snapshot: bool) -> Vec<deno_core::Extension> {
    if from_snapshot {
        vec![
            deno_webidl::deno_webidl::init_ops(),
            deno_console::deno_console::init_ops(),
            deno_url::deno_url::init_ops(),
            deno_web::deno_web::init_ops::<TPerm>(Default::default(), None),
            crate::runtime::runtime::init_ops(),
        ]
    } else {
        vec![
            deno_webidl::deno_webidl::init_ops_and_esm(),
            deno_console::deno_console::init_ops_and_esm(),
            deno_url::deno_url::init_ops_and_esm(),
            deno_web::deno_web::init_ops_and_esm::<TPerm>(Default::default(), None),
            crate::runtime::runtime::init_ops_and_esm(),
        ]
    }
}

/// Read a Botguard snapshot from the given file
///
/// Snapshot file format:
/// ```txt
/// <4>  magic 0x18cba459
/// <4>  info length in bytes
/// <hl> snapshot info (JSON)
/// <4>  data length in bytes
/// <dl> snapshot data
/// ```
/// Sets the V8 flags once per process, before the first isolate, whether it starts from a snapshot
/// or not. This helper owns one isolate per request; V8 background workers only add RSS here.
fn apply_v8_memory_flags() {
    static BOTGUARD_V8_MEMORY_FLAGS: Once = Once::new();
    BOTGUARD_V8_MEMORY_FLAGS.call_once(|| {
        v8::V8::set_flags_from_string(
            "--initial-old-space-size=8 --max-old-space-size=96 --max-semi-space-size=1 --optimize-for-size --single-threaded",
        );
    });
}

/// The largest snapshot header and data read back. A snapshot is about 9 MB.
const MAX_SNAPSHOT_INFO_BYTES: u32 = 64 * 1024;
const MAX_SNAPSHOT_DATA_BYTES: u32 = 64 * 1024 * 1024;

fn read_snapshot_file(path: &Path) -> Result<Option<SnapshotData>, Error> {
    let mut reader = BufReader::new(File::open(path)?);
    let magic = reader.read_u32::<BigEndian>()?;
    if magic != SNAPSHOT_MAGIC {
        return Err(Error::InvalidSnapshot("incorrect magic number".into()));
    }

    let info_len = reader.read_u32::<BigEndian>()?;
    if info_len > MAX_SNAPSHOT_INFO_BYTES {
        return Err(Error::InvalidSnapshot("info header too long".into()));
    }
    let mut info_bytes = vec![0; info_len as usize];
    reader.read_exact(&mut info_bytes)?;
    let info = serde_json::from_slice::<SnapshotInfo>(&info_bytes)
        .map_err(|e| Error::InvalidSnapshot(e.to_string().into()))?;
    if !info.is_valid() {
        return Ok(None);
    }

    let data_len = reader.read_u32::<BigEndian>()?;
    if data_len > MAX_SNAPSHOT_DATA_BYTES {
        return Err(Error::InvalidSnapshot("snapshot too long".into()));
    }
    let data_len = data_len as usize;
    let mut data = Vec::with_capacity(data_len);
    // One byte more than announced shows a file longer than its header says.
    reader.take(data_len as u64 + 1).read_to_end(&mut data)?;
    if data.len() != data_len {
        return Err(Error::InvalidSnapshot("incomplete data".into()));
    }

    Ok(Some(SnapshotData {
        data: data.into_boxed_slice(),
        info,
    }))
}

fn write_snapshot_file(path: &Path, info: &SnapshotInfo, data: &[u8]) -> Result<(), Error> {
    let info =
        serde_json::to_string(info).map_err(|e| Error::InvalidSnapshot(e.to_string().into()))?;

    // Written beside the target and renamed over it, so a crash mid-write keeps the previous snapshot.
    // The name carries the process id, so two writers never share a temporary file.
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    if let Err(error) = write_snapshot_contents(&temporary, &info, data) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        Error::from(error)
    })
}

fn write_snapshot_contents(path: &Path, info: &str, data: &[u8]) -> Result<(), Error> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Readable by the user alone, like the rest of the app's cache.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut writer = options.open(path)?;
    writer.write_u32::<BigEndian>(SNAPSHOT_MAGIC)?;
    writer.write_u32::<BigEndian>(
        info.len()
            .try_into()
            .map_err(|_| Error::InvalidSnapshot("info header too long".into()))?,
    )?;
    writer.write_all(info.as_bytes())?;
    writer.write_u32::<BigEndian>(
        data.len()
            .try_into()
            .map_err(|_| Error::InvalidSnapshot("snapshot too long".into()))?,
    )?;
    writer.write_all(data)?;
    writer.sync_all()?;
    Ok(())
}

async fn get_challenge(client: &Client) -> Result<ChallengeData, Error> {
    let resp = client
        .post("https://www.youtube.com/api/jnn/v1/Create")
        .header(header::CONTENT_TYPE, CONTENT_TYPE)
        .header("x-goog-api-key", GOOG_API_KEY)
        .header("x-user-agent", X_USER_AGENT)
        .json(&[REQUEST_KEY])
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let resp_arr = resp
        .as_array()
        .ok_or(Error::InvalidChallenge("array expected".into()))?;
    if let Some(scrambled) = resp_arr.get(1).and_then(|x| x.as_str()) {
        let descrambled = descramble(scrambled)
            .map_err(|e| Error::InvalidChallenge(format!("descramble: {e}").into()))?;
        let cdata = serde_json::from_slice::<Vec<serde_json::Value>>(&descrambled)
            .map_err(|e| Error::InvalidChallenge(e.to_string().into()))?;
        parse_challenge_data(&cdata)
    } else if let Some(obj) = resp_arr.first().and_then(|x| x.as_array()) {
        parse_challenge_data(obj)
    } else {
        Err(Error::InvalidChallenge("invalid format".into()))
    }
}

async fn resolve_challenge_data(
    client: &Client,
    challenge_data: ChallengeData,
) -> Result<ResolvedChallengeData, Error> {
    let interpreter_js = match challenge_data.interpreter_js {
        InterpreterJs::InterpreterUrl { url } => {
            let url = Url::parse(&format!("https:{url}"))
                .or_else(|_| Url::parse(&url))
                .map_err(|e| Error::InvalidChallenge(format!("{e}: {url}").into()))?;
            let domain = url
                .domain()
                .ok_or(Error::InvalidChallenge("no domain".into()))?;
            let domain = domain.strip_prefix("www.").unwrap_or(domain);
            if !matches!(domain, "google.com" | "youtube.com") {
                return Err(Error::InvalidChallenge(
                    format!("invalid domain: {domain}").into(),
                ));
            }

            client
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?
        }
        InterpreterJs::InterpreterJavascript { script } => script,
    };

    Ok(ResolvedChallengeData {
        interpreter_js,
        program: challenge_data.program,
        global_name: challenge_data.global_name,
    })
}

fn parse_challenge_data(cdata: &[serde_json::Value]) -> Result<ChallengeData, Error> {
    if cdata.len() < 6 {
        return Err(Error::InvalidChallenge("array len < 6".into()));
    }

    let interpreter_js = cdata[1]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find_map(|itm| itm.as_str().filter(|s| !s.is_empty()))
        })
        .map(|s| InterpreterJs::InterpreterJavascript {
            script: s.to_owned(),
        })
        .or_else(|| {
            cdata[2]
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .find_map(|itm| itm.as_str().filter(|s| !s.is_empty()))
                })
                .map(|url| InterpreterJs::InterpreterUrl {
                    url: url.to_owned(),
                })
        })
        .ok_or(Error::InvalidChallenge("interpreterJs".into()))?;

    let program = cdata[4]
        .as_str()
        .ok_or(Error::InvalidChallenge("program".into()))?;
    let global_name = cdata[5]
        .as_str()
        .ok_or(Error::InvalidChallenge("globalName".into()))?;

    Ok(ChallengeData {
        interpreter_js,
        program: program.to_owned(),
        global_name: global_name.to_owned(),
    })
}

fn descramble(scrambled_challenge: &str) -> Result<Vec<u8>, data_encoding::DecodeError> {
    let bts = data_encoding::BASE64.decode(scrambled_challenge.as_bytes())?;
    Ok(bts.into_iter().map(|x| x.wrapping_add(97)).collect())
}

fn validate_potoken(po_token: &str) -> Result<(), Error> {
    let token_bts = data_encoding::BASE64URL
        .decode(po_token.as_bytes())
        .map_err(|e| Error::InvalidPoToken(format!("invalid b64: {e}").into()))?;

    protobuf::bytes_from_pb(token_bts, 6).ok_or_else(|| {
        // The token itself stays out of the message, which can reach a log.
        Error::InvalidPoToken(format!("could not parse protobuf ({} bytes)", po_token.len()).into())
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Botguard;

    use temp_testdir::TempDir;

    async fn _mint_token(bg: &mut Botguard) {
        bg.mint_token("CgswRkprS3VKM1dlNCjX6Iy9BjIKCgJERRIEEgAgOw%3D%3D")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_botguard() {
        let td = TempDir::default();
        let mut snapshot_path = td.to_path_buf();
        snapshot_path.push("bg_snapshot.bin");

        let mut bg = Botguard::builder()
            .snapshot_path(&snapshot_path)
            .init()
            .await
            .unwrap();
        _mint_token(&mut bg).await;
        let cdate = bg.created_at();
        let valid_until = bg.valid_until();
        assert!(!bg.is_from_snapshot());
        assert!(bg.write_snapshot().await);

        let mut bg = Botguard::builder()
            .snapshot_path(&snapshot_path)
            .init()
            .await
            .unwrap();
        assert!(bg.is_from_snapshot());
        assert_eq!(bg.created_at(), cdate);
        assert_eq!(bg.valid_until(), valid_until);
        _mint_token(&mut bg).await;
    }
}
