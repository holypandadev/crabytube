//! Updates. The latest release on GitHub carries `update.json`, which names its version and, for each
//! package, the address, size and SHA-256 of the file, and `update.json.sig`, the Ed25519 signature
//! of that file by the release key. The app uses the description only when the signature is right,
//! and a download only when its size and SHA-256 are the ones the signed description names, so a
//! changed file, or a release someone else made, is never installed.
//!
//! What an update does depends on how the app was installed: the Windows Installer package installs
//! the new package, a Mac app replaces its own bundle, and a copy the app cannot replace (the
//! Windows zip, an app outside a writable folder) points to the release page. The Flatpak asks the
//! Flatpak portal to update it from CrabyTube's Flatpak repository, which checks the repository's
//! own signature, and starts the new version again. Other Linux builds are updated by their package
//! manager, so nothing is checked there.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The description of the latest release. GitHub sends `latest/download/` to the newest published
/// release, which a draft or a pre-release never is.
pub const FEED_URL: &str =
    "https://github.com/holypandadev/crabytube/releases/latest/download/update.json";

/// A feed to use instead of the release's, for testing an update before a release exists. The
/// signature is checked all the same, so it changes only where the files come from.
const FEED_OVERRIDE: &str = "CRABYTUBE_UPDATE_FEED";

/// The public half of the release key. The private half signs `update.json` at release time
/// (`scripts/make-update-manifest.py`) and never leaves the release machine.
const RELEASE_KEY: [u8; 32] = [
    0x85, 0xca, 0x3e, 0x0a, 0x6a, 0xc5, 0x62, 0x9e, 0x81, 0x25, 0x31, 0x62, 0x57, 0x96, 0xda, 0x3c,
    0xbb, 0x72, 0x58, 0x7d, 0x2a, 0x6b, 0x86, 0x2f, 0xa9, 0xe2, 0xda, 0x72, 0x54, 0x91, 0x64, 0xcf,
];

/// The description and its signature are small; a larger answer is not one.
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const SIGNATURE_BYTES: usize = 64;
/// No package of the app comes near this.
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;

/// How the running copy of the app was installed, which decides what an update does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// Installed with the Windows Installer package: the update installs the new package.
    WindowsMsi,
    /// The Mac app at this path, in a folder the user can write to: the update replaces it.
    MacBundle(PathBuf),
    /// A copy the app cannot replace itself: a newer version is offered as its release page.
    Manual,
    /// The Flatpak. The Flatpak portal installs the update from the repository the app came from
    /// (see `flatpak_update`); nothing is downloaded by the app.
    Flatpak,
    /// A Linux build outside the Flatpak, which its package manager or its builder updates:
    /// nothing is checked.
    Managed,
}

impl Install {
    /// Whether the app looks for updates at all.
    pub fn checks(&self) -> bool {
        *self != Install::Managed
    }

    /// Whether the app installs an update itself: it downloads the package, or for the Flatpak has
    /// the portal install it.
    pub fn installs(&self) -> bool {
        matches!(
            self,
            Install::WindowsMsi | Install::MacBundle(_) | Install::Flatpak
        )
    }

    /// The package of a release this copy installs, by its name in `update.json`.
    fn package_kind(&self) -> Option<&'static str> {
        match self {
            Install::WindowsMsi => Some("windows-msi"),
            Install::MacBundle(_) => Some("macos-zip"),
            Install::Manual | Install::Managed | Install::Flatpak => None,
        }
    }
}

/// How the running copy was installed.
pub fn current_install() -> Install {
    #[cfg(windows)]
    {
        let installed = std::env::var_os("LOCALAPPDATA")
            .map(|local| Path::new(&local).join("Programs").join("CrabyTube"));
        let running = std::env::current_exe().ok();
        match (installed, running.as_deref().and_then(Path::parent)) {
            (Some(installed), Some(running)) if same_folder(&installed, running) => {
                Install::WindowsMsi
            }
            _ => Install::Manual,
        }
    }
    #[cfg(target_os = "macos")]
    {
        match std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(mac_bundle_of)
        {
            Some(bundle) if mac_bundle_replaceable(&bundle) => Install::MacBundle(bundle),
            _ => Install::Manual,
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if crate::flatpak::in_flatpak() {
            Install::Flatpak
        } else {
            Install::Managed
        }
    }
}

/// On Windows, gives the process the app id that the Start menu shortcut of the installed copy
/// carries, so the media panel shows the app's name and icon. A portable copy keeps none, so the
/// button of a copy pinned to the taskbar still matches its window.
pub fn identify_process() {
    #[cfg(windows)]
    if current_install() == Install::WindowsMsi {
        let id: Vec<u16> = crate::APP_ID
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: a nul-terminated wide string that lives past the call.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(id.as_ptr())
        };
        if result < 0 {
            tracing::warn!(result, "the app id could not be set");
        }
    }
}

/// Whether two folders are the same, as Windows compares paths: without regard to case.
#[cfg(any(windows, test))]
fn same_folder(a: &Path, b: &Path) -> bool {
    let normal = |path: &Path| {
        path.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    normal(a) == normal(b)
}

/// The `.app` bundle an executable inside `Contents/MacOS` belongs to.
#[cfg(any(target_os = "macos", test))]
fn mac_bundle_of(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

/// Whether the app can put a new bundle in the place of this one: its folder must be writable, and
/// it must not run from the read-only copy macOS makes of an app opened from where it was unpacked.
#[cfg(target_os = "macos")]
fn mac_bundle_replaceable(bundle: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return false;
    }
    let Some(folder) = bundle.parent() else {
        return false;
    };
    let Ok(folder) = std::ffi::CString::new(folder.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a plain system call on a path owned for the length of the call.
    unsafe { libc::access(folder.as_ptr(), libc::W_OK) == 0 }
}

/// What the latest release says about itself.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    /// The release page, with its notes and every package.
    pub notes_url: String,
    pub packages: Vec<Package>,
}

/// One file of a release.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Package {
    /// What the file is: `windows-msi`, `windows-zip`, `macos-zip`. Kinds this version does not
    /// know are left alone.
    pub kind: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hexadecimal.
    pub sha256: String,
}

/// A newer version than the one running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub notes_url: String,
    /// The file this copy installs, when it installs updates itself.
    pub package: Option<Package>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("the update information could not be read: {0}")]
    Network(String),
    #[error("the update information is not signed by the release key")]
    Signature,
    #[error("the update information is not valid: {0}")]
    Manifest(String),
    #[error("the download does not match the release")]
    Mismatch,
    #[error("the update could not be saved: {0}")]
    Storage(String),
    #[error("the update could not be installed: {0}")]
    Install(String),
}

/// Where the description is read from, and whether its packages may come over plain HTTP, which
/// only a test feed allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub url: String,
    pub allow_http: bool,
}

impl Feed {
    pub fn current() -> Feed {
        match std::env::var(FEED_OVERRIDE) {
            Ok(url) if !url.trim().is_empty() => {
                tracing::info!(url = %url.trim(), "update feed overridden for testing");
                Feed {
                    url: url.trim().to_owned(),
                    allow_http: true,
                }
            }
            _ => Feed {
                url: FEED_URL.to_owned(),
                allow_http: false,
            },
        }
    }
}

/// The client for the update requests: it says who is asking, and follows GitHub's redirects to
/// the files.
fn client() -> Result<reqwest::Client, UpdateError> {
    crate::network::configure_http(reqwest::Client::builder())
        .user_agent(concat!("CrabyTube/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|error| UpdateError::Network(error.to_string()))
}

/// Reads a small file whole, refusing one larger than `limit`.
async fn fetch_small(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
) -> Result<Vec<u8>, UpdateError> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::Network(format!("HTTP {}", response.status())));
    }
    read_at_most(response, limit).await
}

/// The body of a response, refused as soon as it passes `limit`, so a server that sends more than
/// a description is never read whole.
async fn read_at_most(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, UpdateError> {
    use futures_util::StreamExt;
    let too_large = || UpdateError::Manifest("too large".into());
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| UpdateError::Network(error.to_string()))?;
        if bytes.len() + chunk.len() > limit {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Asks the latest release whether it is newer than the running version. `Ok(None)` means this
/// version is the latest.
pub async fn check(feed: &Feed, install: &Install) -> Result<Option<Available>, UpdateError> {
    let client = client()?;
    let manifest = fetch_small(&client, &feed.url, MAX_MANIFEST_BYTES).await?;
    let signature = fetch_small(&client, &format!("{}.sig", feed.url), SIGNATURE_BYTES).await?;
    let manifest = read_manifest(&manifest, &signature, &RELEASE_KEY, feed.allow_http)?;
    Ok(newer_than_running(manifest, install))
}

/// The description, once its signature by `key` is right and its addresses are ones the app
/// downloads from.
fn read_manifest(
    manifest: &[u8],
    signature: &[u8],
    key: &[u8],
    allow_http: bool,
) -> Result<Manifest, UpdateError> {
    use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
    if signature.len() != SIGNATURE_BYTES {
        return Err(UpdateError::Signature);
    }
    UnparsedPublicKey::new(&ED25519, key)
        .verify(manifest, signature)
        .map_err(|_| UpdateError::Signature)?;
    let manifest: Manifest = serde_json::from_slice(manifest)
        .map_err(|error| UpdateError::Manifest(error.to_string()))?;
    if parse_version(&manifest.version).is_none() {
        return Err(UpdateError::Manifest(format!(
            "version {:?}",
            manifest.version
        )));
    }
    for url in std::iter::once(&manifest.notes_url)
        .chain(manifest.packages.iter().map(|package| &package.url))
    {
        if !allowed_url(url, allow_http) {
            return Err(UpdateError::Manifest(format!("address {url:?}")));
        }
    }
    for package in &manifest.packages {
        let hex = package.sha256.len() == 64
            && package
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !hex || package.size == 0 || package.size > MAX_PACKAGE_BYTES {
            return Err(UpdateError::Manifest(format!("package {:?}", package.kind)));
        }
    }
    Ok(manifest)
}

/// An address the app downloads from: HTTPS, or plain HTTP from a test feed.
fn allowed_url(url: &str, allow_http: bool) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.host_str().is_some()
            && (url.scheme() == "https" || (allow_http && url.scheme() == "http"))
    })
}

/// The release as an update, when it is newer than the running version.
fn newer_than_running(manifest: Manifest, install: &Install) -> Option<Available> {
    let running = parse_version(env!("CARGO_PKG_VERSION"))?;
    newer(manifest, running, install)
}

fn newer(manifest: Manifest, running: (u64, u64, u64), install: &Install) -> Option<Available> {
    if parse_version(&manifest.version)? <= running {
        return None;
    }
    let package = install.package_kind().and_then(|kind| {
        manifest
            .packages
            .iter()
            .find(|package| package.kind == kind)
            .cloned()
    });
    Some(Available {
        version: manifest.version,
        notes_url: manifest.notes_url,
        package,
    })
}

/// `major.minor.patch`. Anything else is no version this app compares.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.');
    let mut number = || parts.next()?.parse::<u64>().ok();
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

/// The folder downloads go to before they are installed. It is emptied before each download.
fn download_folder() -> Result<PathBuf, UpdateError> {
    crate::app_dirs::cache_dir()
        .map(|cache| cache.join("crabytube").join("updates"))
        .ok_or_else(|| UpdateError::Storage("no cache folder".into()))
}

/// Removes what an earlier update downloaded. By the time the new version starts, the installer has
/// its own copy, so the download only takes room. A file still open is left for the next start.
pub fn remove_old_downloads() {
    let Ok(folder) = download_folder() else {
        return;
    };
    if folder.exists() {
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => tracing::info!("removed the download of an installed update"),
            Err(error) => tracing::debug!(%error, "the update download is still in use"),
        }
    }
}

/// Downloads the package, reporting the bytes received and the total, and returns the file once its
/// size and SHA-256 are the ones the release names. A file that does not match is deleted.
pub async fn download(
    package: Package,
    progress: impl Fn(u64, u64) + Send + 'static,
) -> Result<PathBuf, UpdateError> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let folder = download_folder()?;
    let _ = tokio::fs::remove_dir_all(&folder).await;
    crate::storage::create_private_dir_all(&folder)
        .map_err(|error| UpdateError::Storage(error.to_string()))?;
    let name = match package.kind.as_str() {
        "windows-msi" => "CrabyTube-update.msi",
        _ => "CrabyTube-update.zip",
    };
    let path = folder.join(name);
    let partial = folder.join(format!("{name}.part"));
    let response = client()?
        .get(&package.url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    if !response.status().is_success() {
        return Err(UpdateError::Network(format!("HTTP {}", response.status())));
    }
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|error| UpdateError::Storage(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| UpdateError::Network(error.to_string()))?;
        received += chunk.len() as u64;
        if received > package.size {
            drop(file);
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(UpdateError::Mismatch);
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| UpdateError::Storage(error.to_string()))?;
        progress(received, package.size);
    }
    file.flush()
        .await
        .map_err(|error| UpdateError::Storage(error.to_string()))?;
    drop(file);
    if !matches_release(received, &hex(&hasher.finalize()), &package) {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(UpdateError::Mismatch);
    }
    tokio::fs::rename(&partial, &path)
        .await
        .map_err(|error| UpdateError::Storage(error.to_string()))?;
    Ok(path)
}

fn matches_release(size: u64, sha256: &str, package: &Package) -> bool {
    size == package.size && sha256 == package.sha256
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Installs a downloaded update, or prepares it. The app must quit right after. On Windows the
/// installer starts once the app has shut down (`run_scheduled_install`), replaces its files and
/// opens the new version; on a Mac the new bundle is put in place now, and a script waits for this
/// process to end before it opens it.
pub fn install(downloaded: &Path, install: &Install) -> Result<(), UpdateError> {
    match install {
        #[cfg(windows)]
        Install::WindowsMsi => install_msi(downloaded),
        #[cfg(target_os = "macos")]
        Install::MacBundle(bundle) => install_mac_bundle(downloaded, bundle),
        _ => {
            let _ = downloaded;
            Err(UpdateError::Install(
                "this copy cannot install updates".into(),
            ))
        }
    }
}

/// The argument of a CrabyTube started by the one it replaces after a Flatpak update: it waits for
/// that copy to end instead of asking it to show its window.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

/// How far the Flatpak portal has got with an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlatpakProgress {
    /// Under way, with the share done from 0 to 100.
    Running(u32),
    /// Installed: the new version starts the next time the app does.
    Done,
    /// The repository holds no newer version than the installed one.
    NothingNew,
    /// The new version asks for permissions the installed one does not have, which the portal does
    /// not grant by itself: the software center or `flatpak update` installs it.
    NewPermissions,
    /// Updates of the app are turned off, by the answer to the system's question or in its privacy
    /// settings.
    NotAllowed,
    /// Anything else, with the portal's explanation.
    Failed(String),
}

/// What a `Progress` signal of the portal says.
#[cfg(target_os = "linux")]
fn flatpak_progress(
    info: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
) -> FlatpakProgress {
    let number = |key: &str| {
        info.get(key)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_default()
    };
    let text = |key: &str| {
        info.get(key)
            .and_then(|value| String::try_from(value.clone()).ok())
            .unwrap_or_default()
    };
    match number("status") {
        0 => {
            // Each operation (the app, and a runtime it needs) runs from 0 to 100.
            let operations = number("n_ops").max(1);
            let done = number("op").min(operations - 1);
            FlatpakProgress::Running((done * 100 + number("progress").min(100)) / operations)
        }
        1 => FlatpakProgress::NothingNew,
        2 => FlatpakProgress::Done,
        _ => {
            let (name, message) = (text("error"), text("error_message"));
            if name.ends_with("AccessDenied") {
                FlatpakProgress::NotAllowed
            } else if name.ends_with("NotSupported") && message.contains("permissions") {
                FlatpakProgress::NewPermissions
            } else {
                FlatpakProgress::Failed(if message.is_empty() { name } else { message })
            }
        }
    }
}

/// Asks the Flatpak portal to install the newest version of the app from the repository it was
/// installed from, and follows the installation. The portal checks the repository's signature,
/// asks the user the first time whether the app may update itself, and refuses a version that
/// needs new permissions. The stream ends with the outcome.
#[cfg(target_os = "linux")]
pub fn flatpak_update() -> futures_util::stream::BoxStream<'static, FlatpakProgress> {
    use futures_util::StreamExt;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    // The update starts when the stream is first read, on the app's runtime.
    let start = futures_util::stream::once(async move {
        tokio::spawn(async move {
            if let Err(error) = follow_flatpak_update(&sender).await {
                let _ = sender.send(FlatpakProgress::Failed(error.to_string()));
            }
        });
    })
    .filter_map(|()| async { None });
    start
        .chain(futures_util::stream::unfold(
            receiver,
            |mut receiver| async { receiver.recv().await.map(|progress| (progress, receiver)) },
        ))
        .boxed()
}

#[cfg(target_os = "linux")]
async fn follow_flatpak_update(
    sender: &tokio::sync::mpsc::UnboundedSender<FlatpakProgress>,
) -> zbus::Result<()> {
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
    /// A portal that falls silent this long has stopped; the update is given up.
    const SILENCE: Duration = Duration::from_secs(10 * 60);
    const PORTAL: &str = "org.freedesktop.portal.Flatpak";
    let connection = zbus::Connection::session().await?;
    let portal = zbus::Proxy::new(
        &connection,
        PORTAL,
        "/org/freedesktop/portal/Flatpak",
        PORTAL,
    )
    .await?;
    let monitor: OwnedObjectPath = portal
        .call("CreateUpdateMonitor", &(HashMap::<&str, Value<'_>>::new(),))
        .await?;
    let monitor = zbus::Proxy::new(
        &connection,
        PORTAL,
        monitor,
        "org.freedesktop.portal.Flatpak.UpdateMonitor",
    )
    .await?;
    let mut signals = monitor.receive_signal("Progress").await?;
    monitor
        .call::<_, _, ()>("Update", &("", HashMap::<&str, Value<'_>>::new()))
        .await?;
    loop {
        let Ok(Some(signal)) = tokio::time::timeout(SILENCE, signals.next()).await else {
            let _ = sender.send(FlatpakProgress::Failed(
                "the Flatpak portal stopped answering".into(),
            ));
            break;
        };
        let (info,): (HashMap<String, OwnedValue>,) = signal.body().deserialize()?;
        let progress = flatpak_progress(&info);
        let finished = !matches!(progress, FlatpakProgress::Running(_));
        if sender.send(progress).is_err() || finished {
            break;
        }
    }
    let _ = monitor.call::<_, _, ()>("Close", &()).await;
    Ok(())
}

/// Starts the newest installed version of the app through the Flatpak portal, which the app then
/// gives way to by quitting. The new process waits for this one to end (`AFTER_UPDATE_ARG`).
#[cfg(target_os = "linux")]
pub async fn start_latest_flatpak() -> Result<(), UpdateError> {
    use std::collections::HashMap;
    use zbus::zvariant::{Fd, Value};
    /// `FLATPAK_SPAWN_FLAGS_LATEST_VERSION`: the newest version installed, not the running one.
    const LATEST_VERSION: u32 = 2;
    const PORTAL: &str = "org.freedesktop.portal.Flatpak";
    let failed = |error: zbus::Error| UpdateError::Install(error.to_string());
    // The portal takes paths and arguments as C strings.
    let c_string = |text: &str| {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    let connection = zbus::Connection::session().await.map_err(failed)?;
    let portal = zbus::Proxy::new(
        &connection,
        PORTAL,
        "/org/freedesktop/portal/Flatpak",
        PORTAL,
    )
    .await
    .map_err(failed)?;
    let argv = vec![c_string("crabytube"), c_string(AFTER_UPDATE_ARG)];
    let _pid: u32 = portal
        .call(
            "Spawn",
            &(
                c_string("/"),
                argv,
                HashMap::<u32, Fd<'_>>::new(),
                HashMap::<&str, &str>::new(),
                LATEST_VERSION,
                HashMap::<&str, Value<'_>>::new(),
            ),
        )
        .await
        .map_err(failed)?;
    Ok(())
}

/// The Windows Installer package that `run_scheduled_install` runs once the app has shut down.
#[cfg(windows)]
static SCHEDULED_PACKAGE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(windows)]
fn msiexec() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("msiexec.exe")
}

/// Keeps the package for `run_scheduled_install`. The installer must not find the app's files in
/// use, or it would leave the old ones for a restart that never comes.
#[cfg(windows)]
fn install_msi(package: &Path) -> Result<(), UpdateError> {
    if !msiexec().is_file() {
        return Err(UpdateError::Install("msiexec.exe was not found".into()));
    }
    let mut scheduled = SCHEDULED_PACKAGE
        .lock()
        .map_err(|_| UpdateError::Install("the update could not be scheduled".into()))?;
    *scheduled = Some(package.to_path_buf());
    Ok(())
}

/// Runs the Windows Installer, with its progress bar only, on the package an update scheduled. `main`
/// calls it once the interface has ended and the app has let go of its files; the process ends right
/// after. The package opens the app when it has finished (see `packaging/windows/crabytube.wxs`).
pub fn run_scheduled_install() {
    #[cfg(windows)]
    {
        let package = SCHEDULED_PACKAGE
            .lock()
            .ok()
            .and_then(|mut scheduled| scheduled.take());
        if let Some(package) = package {
            match std::process::Command::new(msiexec())
                .arg("/i")
                .arg(&package)
                .args(["/passive", "/norestart"])
                .spawn()
            {
                Ok(_) => tracing::info!("the installer of the update started"),
                Err(error) => tracing::error!(%error, "the installer of the update did not start"),
            }
        }
    }
}

/// Unpacks the new app beside the running one, swaps the two, and leaves a script that opens the
/// new app once this process has ended.
#[cfg(target_os = "macos")]
fn install_mac_bundle(zip: &Path, bundle: &Path) -> Result<(), UpdateError> {
    let failed = |error: std::io::Error| UpdateError::Install(error.to_string());
    let folder = bundle
        .parent()
        .ok_or_else(|| UpdateError::Install("the app has no folder".into()))?;
    // In the app's own folder, so the swap is a rename on one volume.
    let staging = folder.join(".CrabyTube-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging).map_err(failed)?;
    let unpacked = std::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(&staging)
        .status()
        .map_err(failed)?;
    let new_bundle = staging.join("CrabyTube.app");
    if !unpacked.success() || !new_bundle.join("Contents/MacOS/CrabyTube").is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(UpdateError::Install("the package holds no app".into()));
    }
    let old_bundle = staging.join("CrabyTube-old.app");
    std::fs::rename(bundle, &old_bundle).map_err(failed)?;
    if let Err(error) = std::fs::rename(&new_bundle, bundle) {
        // The app goes back where it was. If that fails too, it stays where it is now, and nothing
        // is deleted: the staging folder holds the only copy.
        if std::fs::rename(&old_bundle, bundle).is_ok() {
            let _ = std::fs::remove_dir_all(&staging);
        } else {
            tracing::error!(
                kept = %old_bundle.display(),
                "the app could not be put back after a failed update"
            );
        }
        return Err(failed(error));
    }
    // The running app keeps its files open, so the old bundle is removed after it has quit.
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; rm -rf "$2"; open "$3""#)
        .arg("crabytube-update")
        .arg(std::process::id().to_string())
        .arg(&staging)
        .arg(bundle)
        .spawn()
        .map(drop)
        .map_err(failed)
}

/// Opens a page in the default browser.
pub fn open_page(url: &str) {
    if !allowed_url(url, false) {
        return;
    }
    #[cfg(windows)]
    let opened = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let opened = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = opened {
        tracing::warn!(%error, "the release page could not be opened");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};

    fn key_pair() -> Ed25519KeyPair {
        let rng = aws_lc_rs::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn manifest_json(version: &str, url: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","notes_url":"https://github.com/holypandadev/crabytube/releases/tag/v{version}","packages":[{{"kind":"windows-msi","url":"{url}","size":10,"sha256":"{}"}},{{"kind":"macos-zip","url":"{url}","size":20,"sha256":"{}"}},{{"kind":"a-future-kind","url":"{url}","size":30,"sha256":"{}"}}],"something_new":true}}"#,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn a_signed_description_is_read_and_a_changed_one_is_refused() {
        let keys = key_pair();
        let public = keys.public_key().as_ref().to_vec();
        let json = manifest_json(
            "0.2.0",
            "https://github.com/a/b/releases/download/v0.2.0/x.msi",
        );
        let signature = keys.sign(&json);
        let manifest = read_manifest(&json, signature.as_ref(), &public, false).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.packages.len(), 3);
        // One changed byte, a signature by another key, or one of the wrong size: refused.
        let mut changed = json.clone();
        changed[13] = b'9';
        assert_eq!(
            read_manifest(&changed, signature.as_ref(), &public, false),
            Err(UpdateError::Signature)
        );
        let other = key_pair().sign(&json);
        assert_eq!(
            read_manifest(&json, other.as_ref(), &public, false),
            Err(UpdateError::Signature)
        );
        assert_eq!(
            read_manifest(&json, &signature.as_ref()[..63], &public, false),
            Err(UpdateError::Signature)
        );
        // The release key itself does not accept this test key's signature.
        assert_eq!(
            read_manifest(&json, signature.as_ref(), &RELEASE_KEY, false),
            Err(UpdateError::Signature)
        );
    }

    #[test]
    fn a_signed_description_with_a_plain_http_package_needs_a_test_feed() {
        let keys = key_pair();
        let public = keys.public_key().as_ref().to_vec();
        let json = manifest_json("0.2.0", "http://127.0.0.1:8765/x.msi");
        let signature = keys.sign(&json);
        assert!(matches!(
            read_manifest(&json, signature.as_ref(), &public, false),
            Err(UpdateError::Manifest(_))
        ));
        assert!(read_manifest(&json, signature.as_ref(), &public, true).is_ok());
    }

    #[test]
    fn a_signed_description_with_a_bad_hash_or_version_is_refused() {
        let keys = key_pair();
        let public = keys.public_key().as_ref().to_vec();
        for json in [
            br#"{"version":"0.2","notes_url":"https://x.dev/","packages":[]}"#.to_vec(),
            br#"{"version":"0.2.0","notes_url":"https://x.dev/","packages":[{"kind":"windows-msi","url":"https://x.dev/a","size":1,"sha256":"XYZ"}]}"#.to_vec(),
            br#"{"version":"0.2.0","notes_url":"https://x.dev/","packages":[{"kind":"windows-msi","url":"https://x.dev/a","size":0,"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}"#.to_vec(),
            br#"{"version":"0.2.0","notes_url":"file:///etc/passwd","packages":[]}"#.to_vec(),
        ] {
            let signature = keys.sign(&json);
            assert!(matches!(
                read_manifest(&json, signature.as_ref(), &public, false),
                Err(UpdateError::Manifest(_))
            ));
        }
    }

    #[test]
    fn only_a_newer_version_is_offered_with_the_package_this_copy_installs() {
        let manifest = |version: &str| {
            serde_json::from_slice::<Manifest>(&manifest_json(version, "https://x.dev/p")).unwrap()
        };
        let running = (0, 1, 0);
        assert_eq!(
            newer(manifest("0.1.0"), running, &Install::WindowsMsi),
            None
        );
        assert_eq!(
            newer(manifest("0.0.9"), running, &Install::WindowsMsi),
            None
        );
        let msi = newer(manifest("0.1.1"), running, &Install::WindowsMsi).unwrap();
        assert_eq!(msi.version, "0.1.1");
        assert_eq!(msi.package.unwrap().kind, "windows-msi");
        let mac = newer(
            manifest("0.10.0"),
            running,
            &Install::MacBundle(PathBuf::from("/Applications/CrabyTube.app")),
        )
        .unwrap();
        assert_eq!(mac.package.unwrap().kind, "macos-zip");
        // A copy that cannot install updates gets the page only.
        let manual = newer(manifest("1.0.0"), running, &Install::Manual).unwrap();
        assert!(manual.package.is_none());
        assert!(manual.notes_url.ends_with("/v1.0.0"));
    }

    /// A one-shot server on the loopback address that answers with `body`, with or without a
    /// length, and the address it listens on.
    async fn serve_once(body: Vec<u8>, with_length: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let head = if with_length {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
            } else {
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_owned()
            };
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
        });
        format!("http://{address}/update.json")
    }

    #[tokio::test]
    async fn a_description_larger_than_its_limit_is_not_read_whole() {
        let client = client().unwrap();
        // Within the limit, with and without a length.
        for with_length in [true, false] {
            let url = serve_once(vec![b'a'; 100], with_length).await;
            assert_eq!(fetch_small(&client, &url, 1000).await.unwrap().len(), 100);
        }
        // Over it: refused by the length it announces, or as soon as the bytes pass it.
        for with_length in [true, false] {
            let url = serve_once(vec![b'a'; 300_000], with_length).await;
            assert_eq!(
                fetch_small(&client, &url, 1000).await,
                Err(UpdateError::Manifest("too large".into()))
            );
        }
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert_eq!(parse_version("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version(" 1.20.3 "), Some((1, 20, 3)));
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
        for invalid in ["", "1", "1.2", "1.2.3.4", "1.2.x", "v1.2.3", "1.2.3-beta"] {
            assert_eq!(parse_version(invalid), None, "{invalid}");
        }
        // The running version is one this app reads.
        assert!(parse_version(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn a_download_is_kept_only_when_its_size_and_hash_match() {
        let package = Package {
            kind: "windows-msi".into(),
            url: "https://x.dev/p".into(),
            size: 3,
            sha256: hex(&Sha256::digest(b"abc")),
        };
        assert!(matches_release(3, &hex(&Sha256::digest(b"abc")), &package));
        assert!(!matches_release(3, &hex(&Sha256::digest(b"abd")), &package));
        assert!(!matches_release(4, &hex(&Sha256::digest(b"abc")), &package));
    }

    #[test]
    fn the_installed_copy_is_recognised_by_its_folder() {
        assert!(same_folder(
            Path::new(r"C:\Users\Ana\AppData\Local\Programs\CrabyTube"),
            Path::new(r"c:\users\ana\appdata\local\programs\crabytube\")
        ));
        assert!(!same_folder(
            Path::new(r"C:\Users\Ana\AppData\Local\Programs\CrabyTube"),
            Path::new(r"C:\Users\Ana\Downloads\CrabyTube")
        ));
        assert_eq!(
            mac_bundle_of(Path::new(
                "/Applications/CrabyTube.app/Contents/MacOS/crabytube"
            )),
            Some(PathBuf::from("/Applications/CrabyTube.app"))
        );
        assert_eq!(mac_bundle_of(Path::new("/usr/local/bin/crabytube")), None);
        assert!(!Install::Managed.checks());
        assert!(Install::Manual.checks() && !Install::Manual.installs());
        assert!(Install::WindowsMsi.installs());
        assert!(Install::Flatpak.checks() && Install::Flatpak.installs());
    }

    #[test]
    fn the_flatpak_is_offered_the_version_and_downloads_nothing() {
        let json = format!(
            r#"{{"version":"9.9.9","notes_url":"https://github.com/holypandadev/crabytube/releases/tag/v9.9.9","packages":[{{"kind":"windows-msi","url":"https://example.invalid/a.msi","size":10,"sha256":"{a}"}}]}}"#,
            a = "a".repeat(64),
        );
        let manifest: Manifest = serde_json::from_str(&json).unwrap();
        let offer = newer(manifest.clone(), (0, 1, 1), &Install::Flatpak).unwrap();
        assert_eq!(offer.version, "9.9.9");
        assert!(offer.package.is_none());
        // A build outside the Flatpak asks nothing; the Windows installer keeps its own package.
        assert!(!Install::Managed.checks());
        let msi = newer(manifest, (0, 1, 1), &Install::WindowsMsi).unwrap();
        assert_eq!(msi.package.unwrap().kind, "windows-msi");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_portal_progress_reads_as_the_app_shows_it() {
        use zbus::zvariant::{OwnedValue, Value};
        let info = |pairs: &[(&str, Value<'static>)]| {
            pairs
                .iter()
                .map(|(key, value)| {
                    (
                        (*key).to_owned(),
                        OwnedValue::try_from(value.clone()).unwrap(),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>()
        };
        // The second of two operations, half done.
        let running = info(&[
            ("status", Value::U32(0)),
            ("op", Value::U32(1)),
            ("n_ops", Value::U32(2)),
            ("progress", Value::U32(50)),
        ]);
        assert_eq!(flatpak_progress(&running), FlatpakProgress::Running(75));
        // The first report, before any operation is known.
        assert_eq!(
            flatpak_progress(&info(&[("status", Value::U32(0))])),
            FlatpakProgress::Running(0)
        );
        assert_eq!(
            flatpak_progress(&info(&[("status", Value::U32(1))])),
            FlatpakProgress::NothingNew
        );
        assert_eq!(
            flatpak_progress(&info(&[("status", Value::U32(2))])),
            FlatpakProgress::Done
        );
        let error = |name: &'static str, message: &'static str| {
            flatpak_progress(&info(&[
                ("status", Value::U32(3)),
                ("error", Value::from(name)),
                ("error_message", Value::from(message)),
            ]))
        };
        assert_eq!(
            error(
                "org.freedesktop.DBus.Error.AccessDenied",
                "Application update not allowed"
            ),
            FlatpakProgress::NotAllowed
        );
        assert_eq!(
            error(
                "org.freedesktop.DBus.Error.NotSupported",
                "Self update not supported, new version requires new permissions"
            ),
            FlatpakProgress::NewPermissions
        );
        assert_eq!(
            error("org.freedesktop.DBus.Error.Failed", "No remote refs found"),
            FlatpakProgress::Failed("No remote refs found".into())
        );
    }

    #[test]
    fn the_release_key_accepts_what_the_release_script_signs() {
        // Signed with the release key the way the release script signs (`openssl pkeyutl -sign
        // -rawin`); version 0.0.0 and no packages, so it can never offer anything.
        let json = include_bytes!("../tests/fixtures/update-key-check.json");
        let signature = include_bytes!("../tests/fixtures/update-key-check.json.sig");
        let manifest = read_manifest(json, signature, &RELEASE_KEY, false).unwrap();
        assert_eq!(manifest.version, "0.0.0");
        assert!(manifest.packages.is_empty());
        assert_eq!(newer_than_running(manifest, &Install::WindowsMsi), None);
    }
}
