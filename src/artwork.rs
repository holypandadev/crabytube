//! Shared, bounded image transport with disk reuse and CPU-generated ambient blur.
use futures_util::StreamExt;
use iced::widget::image;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        OnceLock, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

/// CPU pixel budget for decoded artwork. Sixteen MiB holds a full
/// 2560x1440 viewport of small variants plus visible card variants; the
/// separate current-player handle keeps its cover available while the
/// shared LRU prioritizes visible items. The budget bounds resident pixels
/// via LRU eviction — it must never permanently drop visible URLs from the
/// fetch queue.
pub const MAX_ARTWORK_BYTES: usize = 16 * 1024 * 1024;
/// Logical-size variants (longest edge). Thumb covers 48px rows at 200% DPI,
/// card covers 160px cards at 200% DPI, large is reserved for hero/player.
pub const THUMB_MAX: u32 = 128;
/// A medium card variant used between 1x and 1.2x display scale.
pub const CARD_MID_MAX: u32 = 192;
pub const CARD_MAX: u32 = 320;
pub const LARGE_MAX: u32 = 640;
/// Maximum entries as a secondary cap; bytes are the primary budget.
/// 192 covers a 2560x1440 viewport (up to ~150 visible cards + overscan).
pub const MAX_ARTWORK_ENTRIES: usize = 192;
/// Persistent variant-art cache is bounded independently from decoded pixels.
/// Each (URL, variant) has its own file so small thumbs survive eviction
/// without forcing a large download for every row.
pub const MAX_DISK_ARTWORK_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DISK_ARTWORK_FILES: usize = 400;
/// Decoded-pixel budget while a BotGuard helper runs. Evicted images are reloaded from disk.
pub const HELPER_WINDOW_BUDGET_BYTES: usize = 2 * 1024 * 1024;

pub fn handle_bytes(handle: &image::Handle) -> usize {
    match handle {
        image::Handle::Rgba { width, height, .. } => (*width as usize)
            .saturating_mul(*height as usize)
            .saturating_mul(4),
        // Disk/path/encoded variants do not hold decoded pixels here.
        _ => 0,
    }
}

/// Upper bound on how many requested variants fit in the decoded-image budget.
/// The square estimate is conservative for landscape and portrait covers.
pub fn max_cached_variant_count(max_dim: u32) -> usize {
    let max_dim = max_dim.clamp(64, LARGE_MAX) as usize;
    let bytes_per_image = max_dim.saturating_mul(max_dim).saturating_mul(4).max(1);
    (MAX_ARTWORK_BYTES / bytes_per_image).clamp(1, MAX_ARTWORK_ENTRIES)
}

/// One shared app-wide LRU. Handles are keyed by source and requested pixel
/// variant, so a row thumbnail and a player cover do not overwrite each other.
/// The sizes of one source are kept together, so a lookup reads only that source.
#[derive(Default)]
pub struct ArtworkCache {
    entries: HashMap<String, BTreeMap<u32, Cached>>,
    /// Variants in the order they were last used: the least recently used comes first.
    by_use: BTreeMap<u64, (String, u32)>,
    /// The last use number handed out.
    uses: u64,
    /// Variants held.
    count: usize,
    bytes: usize,
    /// Temporary budget that replaces `MAX_ARTWORK_BYTES` while set.
    budget_override: Option<usize>,
}

/// One decoded variant of a source.
struct Cached {
    handle: image::Handle,
    /// The variant's place in `by_use`.
    used: u64,
    /// When the variant arrived, so covers can fade in instead of popping.
    loaded_at: std::time::Instant,
}

impl ArtworkCache {
    pub fn get(&self, url: &str) -> Option<&image::Handle> {
        self.get_for(url, LARGE_MAX)
    }

    pub fn get_exact(&self, url: &str, max_dim: u32) -> Option<&image::Handle> {
        self.entries
            .get(url)?
            .get(&max_dim.clamp(64, LARGE_MAX))
            .map(|cached| &cached.handle)
    }

    /// The closest variant that is at least as large as `max_dim`, or else the largest one.
    fn variant(&self, url: &str, max_dim: u32) -> Option<&Cached> {
        let sizes = self.entries.get(url)?;
        let wanted = max_dim.clamp(64, LARGE_MAX);
        sizes
            .range(wanted..)
            .next()
            .or_else(|| sizes.iter().next_back())
            .map(|(_, cached)| cached)
    }

    /// Like `get_for`, plus how long ago that variant arrived.
    pub fn get_for_with_age(
        &self,
        url: &str,
        max_dim: u32,
    ) -> Option<(&image::Handle, std::time::Duration)> {
        self.variant(url, max_dim)
            .map(|cached| (&cached.handle, cached.loaded_at.elapsed()))
    }

    /// Prefer the closest available variant that is at least as large as the
    /// requested maximum; use the largest smaller variant only as fallback.
    pub fn get_for(&self, url: &str, max_dim: u32) -> Option<&image::Handle> {
        self.variant(url, max_dim).map(|cached| &cached.handle)
    }

    pub fn contains_key(&self, url: &str) -> bool {
        self.entries.get(url).is_some_and(|sizes| !sizes.is_empty())
    }

    pub fn contains_variant(&self, url: &str, max_dim: u32) -> bool {
        self.entries
            .get(url)
            .is_some_and(|sizes| sizes.contains_key(&max_dim.clamp(64, LARGE_MAX)))
    }

    /// Marks a variant as used now, so that it is kept longest.
    pub fn touch(&mut self, url: &str, max_dim: u32) {
        let size = max_dim.clamp(64, LARGE_MAX);
        let Some(cached) = self
            .entries
            .get_mut(url)
            .and_then(|sizes| sizes.get_mut(&size))
        else {
            return;
        };
        self.uses += 1;
        self.by_use.remove(&cached.used);
        cached.used = self.uses;
        self.by_use.insert(self.uses, (url.to_owned(), size));
    }

    pub fn insert(&mut self, url: String, max_dim: u32, handle: image::Handle) {
        let size = max_dim.clamp(64, LARGE_MAX);
        // A replaced variant keeps its arrival time, so its cover does not fade in again.
        let loaded_at = match self.detach(&url, size) {
            Some(previous) => {
                self.by_use.remove(&previous.used);
                previous.loaded_at
            }
            None => std::time::Instant::now(),
        };
        self.uses += 1;
        self.bytes = self.bytes.saturating_add(handle_bytes(&handle));
        self.count += 1;
        self.by_use.insert(self.uses, (url.clone(), size));
        self.entries.entry(url).or_default().insert(
            size,
            Cached {
                handle,
                used: self.uses,
                loaded_at,
            },
        );
        self.evict_to_budget();
    }

    /// Drops every cached variant whose source `keep` rejects. Used to free memory
    /// around a helper run without blanking the covers that are on screen.
    pub fn retain_urls(&mut self, keep: impl Fn(&str) -> bool) {
        let dropped: Vec<(String, u32)> = self
            .entries
            .iter()
            .filter(|(url, _)| !keep(url))
            .flat_map(|(url, sizes)| sizes.keys().map(move |size| (url.clone(), *size)))
            .collect();
        for (url, size) in dropped {
            self.remove_variant(&url, size);
        }
    }

    /// Drops one cached variant and nothing else, for a cover that is no longer shown.
    pub fn remove_variant(&mut self, url: &str, max_dim: u32) {
        if let Some(cached) = self.detach(url, max_dim.clamp(64, LARGE_MAX)) {
            self.by_use.remove(&cached.used);
        }
    }

    /// Sets a temporary byte budget (`None` restores the normal one) and evicts to fit it.
    pub fn set_budget(&mut self, budget: Option<usize>) {
        self.budget_override = budget;
        self.evict_to_budget();
    }

    /// Takes one variant out of the map and out of the byte and count totals. Its place in
    /// `by_use` is left to the caller.
    fn detach(&mut self, url: &str, size: u32) -> Option<Cached> {
        let sizes = self.entries.get_mut(url)?;
        let cached = sizes.remove(&size)?;
        let empty = sizes.is_empty();
        if empty {
            self.entries.remove(url);
        }
        self.bytes = self.bytes.saturating_sub(handle_bytes(&cached.handle));
        self.count = self.count.saturating_sub(1);
        Some(cached)
    }

    fn evict_to_budget(&mut self) {
        let budget = self.budget_override.unwrap_or(MAX_ARTWORK_BYTES);
        while self.bytes > budget || self.count > MAX_ARTWORK_ENTRIES {
            let Some((_, (url, size))) = self.by_use.pop_first() else {
                break;
            };
            self.detach(&url, size);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_use.clear();
        self.count = 0;
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Choose enough source pixels for a 160 logical-pixel Home card at the
/// current display scale without always decoding a 320px image on 1x screens.
/// Smallest cached variant that covers `pixels` physical pixels without visible upscaling.
pub fn variant_for_pixels(pixels: f32) -> u32 {
    [THUMB_MAX, CARD_MID_MAX, CARD_MAX]
        .into_iter()
        .find(|variant| *variant as f32 >= pixels * 0.85)
        .unwrap_or(LARGE_MAX)
}

pub fn card_variant_for_scale(scale_factor: f32) -> u32 {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor.clamp(0.75, 4.0)
    } else {
        1.0
    };
    let needed = 160.0 * scale;
    if scale <= 1.0 {
        // At 1x, a 128px image is modestly upscaled but lets the bounded cache
        // retain up to 64 covers instead of evicting nearly every visible card.
        THUMB_MAX
    } else if needed <= CARD_MID_MAX as f32 {
        CARD_MID_MAX
    } else if needed <= CARD_MAX as f32 {
        CARD_MAX
    } else {
        LARGE_MAX
    }
}
use tokio_util::sync::CancellationToken;

fn client() -> Option<&'static reqwest::Client> {
    crate::network::shared().ok()
}

/// Persistent cache keyed by (URL, variant, version) so 128/192/320px
/// thumbs survive RAM eviction without forcing a 640px download per row. Each
/// file holds the bytes that were downloaded for its variant, so nothing is
/// re-encoded when it is saved. Legacy v4 master files (single file per URL)
/// are still readable as a fallback and are downsampled to the requested variant.
fn variant_cache_path(url: &str, max_dim: u32) -> std::path::PathBuf {
    let dim = max_dim.clamp(64, LARGE_MAX);
    artwork_cache_root().join(format!(
        "{:x}.img",
        Sha256::digest(format!("v6:{url}:{dim}").as_bytes())
    ))
}

/// Directory of the persistent cover cache. Tests redirect it to a scratch
/// directory so they never write into the user's cache.
fn artwork_cache_root() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(root) = TEST_CACHE_ROOT.get() {
        return root.clone();
    }
    crate::app_dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("crabytube/artwork")
}

#[cfg(test)]
static TEST_CACHE_ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();

/// The cover saved on disk for this URL at this size, read without any network request. A restored
/// session uses it at startup: a missing or unreadable file gives `None`, and nothing is fetched.
pub fn saved_sized(url: &str, max_dim: u32) -> Option<image::Handle> {
    let max_dim = max_dim.clamp(64, LARGE_MAX);
    // The large cover is what a player request saves, so it is scaled down when that is all there is.
    [
        variant_cache_path(url, max_dim),
        variant_cache_path(url, LARGE_MAX),
        legacy_cache_path(url),
    ]
    .into_iter()
    .filter(|candidate| candidate.is_file())
    .find_map(|candidate| cached_or_remove_sized(candidate, max_dim))
}

/// The large cover already saved on disk for this URL, or the single file earlier versions kept
/// for it. Nothing is downloaded to find it.
pub fn saved_cover_path(url: &str) -> Option<std::path::PathBuf> {
    [variant_cache_path(url, LARGE_MAX), legacy_cache_path(url)]
        .into_iter()
        .find(|path| path.is_file())
}

fn legacy_cache_path(url: &str) -> std::path::PathBuf {
    artwork_cache_root().join(format!(
        "{:x}.png",
        Sha256::digest(format!("v4:{url}").as_bytes())
    ))
}

/// How many times the cover folder has been cleared. A download that began before a clear does not
/// save what it got, so the folder does not fill again with covers the user just cleared.
static CLEARS: AtomicU64 = AtomicU64::new(0);

/// Saves hold it shared and a clear holds it alone, so no file lands in the folder while it is being
/// removed.
static FOLDER: RwLock<()> = RwLock::new(());

/// Runs `clear` alone on the cover folder, and turns away the saves of downloads already under way.
pub fn clearing_folder<T>(clear: impl FnOnce() -> T) -> T {
    let _alone = FOLDER.write().unwrap_or_else(PoisonError::into_inner);
    CLEARS.fetch_add(1, Ordering::AcqRel);
    clear()
}

/// Saves a downloaded cover unless the folder was cleared since the download began.
fn save_unless_cleared(path: &std::path::Path, bytes: &[u8], clears_at_start: u64) -> bool {
    let _shared = FOLDER.read().unwrap_or_else(PoisonError::into_inner);
    CLEARS.load(Ordering::Acquire) == clears_at_start && atomic_save_bytes(path, bytes)
}

/// A temp file name no other save of this process uses. Covers are saved in parallel, and the clock
/// alone does not tell two saves apart: on macOS it counts whole microseconds, and two saves that shared
/// a file left one cover's picture under the other's name.
fn temporary_name() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Atomic write of a cache file: the bytes go to a unique temp file in the same directory, which is
/// then renamed, so readers never see a partial file. The folder is trimmed once the file is in place.
fn atomic_save_bytes(path: &std::path::Path, bytes: &[u8]) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if crate::storage::create_private_dir_all(parent).is_err() {
        return false;
    }
    let tmp = parent.join(temporary_name());
    if std::fs::write(&tmp, bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, path).is_ok() {
        trim_cache(parent, Some(path));
        true
    } else {
        let _ = std::fs::remove_file(&tmp);
        false
    }
}

fn artwork_request_url_for(url: &str, width: u32) -> String {
    if let Some(prefix) = url.strip_suffix("/default.jpg") {
        return format!("{prefix}/hqdefault.jpg");
    }
    if let Some(prefix) = url.strip_suffix("/mqdefault.jpg") {
        return format!("{prefix}/hqdefault.jpg");
    }
    if let Some(prefix) = url.strip_suffix("/sddefault.jpg") {
        return format!("{prefix}/hqdefault.jpg");
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return url.to_owned();
    };
    if !parsed
        .host_str()
        .is_some_and(|host| host.ends_with("googleusercontent.com"))
    {
        return url.to_owned();
    }
    let Some((prefix, size)) = url.rsplit_once("=w") else {
        return url.to_owned();
    };
    let width_len = size.bytes().take_while(u8::is_ascii_digit).count();
    let Some(height) = size[width_len..].strip_prefix("-h") else {
        return url.to_owned();
    };
    let height_len = height.bytes().take_while(u8::is_ascii_digit).count();
    let (Ok(current_width), Ok(height_value)) = (
        size[..width_len].parse::<u32>(),
        height[..height_len].parse::<u32>(),
    ) else {
        return url.to_owned();
    };
    let wanted = width.clamp(128, 800);
    // The size the variant needs, in the shape of the original: a larger original is not
    // downloaded only to be scaled down, and a smaller one is asked for at the size needed.
    let scaled_height = if current_width == 0 {
        wanted
    } else {
        (u64::from(height_value) * u64::from(wanted)).div_ceil(u64::from(current_width)) as u32
    };
    format!(
        "{prefix}=w{wanted}-h{}{}",
        scaled_height.max(1),
        &height[height_len..]
    )
}
/// The URLs to try for a cover, most preferred first.
///
/// The player asks for the largest still YouTube has for a video and steps
/// down when a video lacks it. Other covers use the sized request, with the
/// original URL as the fallback when the sized one fails.
fn cover_candidates(url: &str, lane: Lane, max_dim: u32) -> Vec<String> {
    if lane == Lane::Player
        && let Some(stills) = video_still_candidates(url)
    {
        return stills;
    }
    let sized = artwork_request_url_for(url, max_dim.clamp(128, 800));
    if sized == url {
        vec![sized]
    } else {
        vec![sized, url.to_owned()]
    }
}

/// Video stills for `https://i.ytimg.com/vi/<id>/<name>.jpg`, largest first.
fn video_still_candidates(url: &str) -> Option<Vec<String>> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.host_str()? != "i.ytimg.com" {
        return None;
    }
    let segments: Vec<&str> = parsed.path_segments()?.collect();
    let ["vi", video_id, _] = segments.as_slice() else {
        return None;
    };
    if video_id.is_empty() {
        return None;
    }
    Some(
        ["maxresdefault", "sddefault", "hqdefault"]
            .into_iter()
            .map(|name| format!("https://i.ytimg.com/vi/{video_id}/{name}.jpg"))
            .collect(),
    )
}

/// Whether a cover address is one YouTube serves covers from, over HTTPS. Addresses come from the
/// catalog, but also from an imported backup, which must not make the app contact any other host.
fn is_cover_host(url: &str) -> bool {
    // The tests serve covers from a local server.
    #[cfg(test)]
    if reqwest::Url::parse(url).is_ok_and(|url| url.host_str() == Some("127.0.0.1")) {
        return true;
    }
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                ["ytimg.com", "googleusercontent.com", "ggpht.com"]
                    .iter()
                    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
            })
    })
}

/// The response of the first candidate that answers with success.
async fn first_available(
    http: &reqwest::Client,
    token: &CancellationToken,
    candidates: &[String],
) -> Option<reqwest::Response> {
    for candidate in candidates.iter().filter(|url| is_cover_host(url)) {
        let response = tokio::select! {
            _ = token.cancelled() => return None,
            result = http.get(candidate).timeout(Duration::from_secs(12)).send() => result,
        };
        if let Ok(response) = response
            && response.status().is_success()
        {
            return Some(response);
        }
    }
    None
}

pub async fn fetch(url: String, token: CancellationToken) -> Option<image::Handle> {
    fetch_sized(url, token, LARGE_MAX).await
}

/// Parallel downloads for list covers. A screenful shares the HTTP/2
/// connection pool, so it arrives in a few rounds instead of one by one.
const LIST_DOWNLOAD_SLOTS: usize = 8;
/// Downloads kept free for the cover of the track that plays, so a long list
/// never delays it.
const PLAYER_DOWNLOAD_SLOTS: usize = 2;
static LIST_SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static PLAYER_SLOTS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();

/// Which download slots a cover fetch uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lane {
    List,
    Player,
}

pub async fn fetch_sized(
    url: String,
    token: CancellationToken,
    max_dim: u32,
) -> Option<image::Handle> {
    fetch_in_lane(url, token, max_dim, Lane::List).await
}

/// The cover of the track that plays, fetched in its own lane.
pub async fn fetch_player(
    url: String,
    token: CancellationToken,
    max_dim: u32,
) -> Option<image::Handle> {
    fetch_in_lane(url, token, max_dim, Lane::Player).await
}

async fn fetch_in_lane(
    url: String,
    token: CancellationToken,
    max_dim: u32,
    lane: Lane,
) -> Option<image::Handle> {
    let max_dim = max_dim.clamp(64, LARGE_MAX);
    let clears_at_start = CLEARS.load(Ordering::Acquire);
    // Variant-specific file first, then legacy v4 master (downsampled).
    // Either hit avoids the network, so a second visit with an empty RAM
    // cache but warm disk reuses thumbs without re-downloading. Disk hits run
    // before the download slots are taken, so they never queue behind them.
    let variant_path = variant_cache_path(&url, max_dim);
    let legacy_path = legacy_cache_path(&url);
    for candidate in [variant_path.clone(), legacy_path.clone()] {
        if candidate.is_file()
            && let Some(handle) = tokio::task::spawn_blocking({
                let cached_path = candidate.clone();
                move || cached_or_remove_sized(cached_path, max_dim)
            })
            .await
            .ok()
            .flatten()
        {
            return Some(handle);
        }
        // A corrupt entry is removed by cached_or_remove_sized; continue to
        // the next candidate / network instead of leaving the card blank.
    }
    let slots = match lane {
        Lane::List => LIST_SLOTS.get_or_init(|| tokio::sync::Semaphore::new(LIST_DOWNLOAD_SLOTS)),
        Lane::Player => {
            PLAYER_SLOTS.get_or_init(|| tokio::sync::Semaphore::new(PLAYER_DOWNLOAD_SLOTS))
        }
    };
    let _permit = tokio::select! {_ = token.cancelled()=>return None,p=slots.acquire()=>p.ok()?};
    let host = reqwest::Url::parse(&url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "invalid-url".into());
    let http = client()?;
    let candidates = cover_candidates(&url, lane, max_dim);
    let response = first_available(http, &token, &candidates).await?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|n| n > 8 * 1024 * 1024)
    {
        tracing::debug!(host, status = %response.status(), "artwork response was rejected");
        return None;
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::select! {_ = token.cancelled()=>return None,c=stream.next()=>c};
        let Some(chunk) = chunk else { break };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                tracing::debug!(host, %error, "artwork download stream failed");
                return None;
            }
        };
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    if token.is_cancelled() {
        return None;
    }
    tokio::task::spawn_blocking(move || {
        // The download is decoded first, so a response that is not an image is never kept. The bytes
        // are then saved as they arrived. Each variant has its own file, so small thumbs are reused
        // after RAM eviction without forcing a 640px download for each row.
        let image = thumbnail_sized(&bytes, max_dim)?;
        let path = variant_cache_path(&url, max_dim);
        save_unless_cleared(&path, &bytes, clears_at_start);
        Some(rgba_handle(image))
    })
    .await
    .ok()
    .flatten()
}
fn cached_handle_sized(path: std::path::PathBuf, max_dim: u32) -> Option<image::Handle> {
    use std::io::Read;
    // Write access lets the modification time be updated below; Windows refuses it on a
    // read-only handle. A file that can only be read is still used.
    let mut file = std::fs::File::options()
        .read(true)
        .write(true)
        .open(&path)
        .or_else(|_| std::fs::File::open(&path))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    // Trimming removes the least recently modified files first, so a file that was just read counts as used now.
    let _ = file.set_modified(std::time::SystemTime::now());
    let image = thumbnail_sized(&bytes, max_dim)?;
    Some(rgba_handle(image))
}
fn cached_or_remove_sized(path: std::path::PathBuf, max_dim: u32) -> Option<image::Handle> {
    let handle = cached_handle_sized(path.clone(), max_dim);
    if handle.is_none() {
        let _ = std::fs::remove_file(path);
    }
    handle
}
fn rgba_handle(image: ::image::RgbaImage) -> image::Handle {
    image::Handle::from_rgba(image.width(), image.height(), image.into_raw())
}
fn read_image(bytes: &[u8]) -> Option<::image::DynamicImage> {
    let mut reader = ::image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = ::image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().ok()
}
fn thumbnail_sized(bytes: &[u8], max_dim: u32) -> Option<::image::RgbaImage> {
    // Keep disk, CPU and GPU artwork bounded even when a provider sends large originals.
    let max_dim = max_dim.clamp(64, LARGE_MAX);
    let decoded = read_image(bytes)?;
    let (width, height) = (decoded.width(), decoded.height());
    // Never hand the renderer a tiny source that would be scaled by 10x or
    // more, which hits a truncation hazard in tiny-skia. Reject pathological
    // thumbs so the UI shows a fixed-size container instead.
    if width < 8 || height < 8 {
        return None;
    }
    let decoded = square_crop(decoded);
    let decoded = if decoded.width() > max_dim || decoded.height() > max_dim {
        decoded.thumbnail(max_dim, max_dim)
    } else {
        decoded
    };
    Some(decoded.to_rgba8())
}

/// Centre-crops artwork to a square, the only shape the UI draws. Video
/// thumbnails often arrive as 4:3 frames with black bars around 16:9 content;
/// the bars are dropped before cropping so covers are not mostly black.
fn square_crop(image: ::image::DynamicImage) -> ::image::DynamicImage {
    let (width, height) = (image.width(), image.height());
    let (mut top, mut content_height) = (0, height);
    let four_by_three = width * 3 == height * 4;
    if four_by_three
        && is_dark_band(&image, 0, height / 10)
        && is_dark_band(&image, height - height / 10, height / 10)
    {
        content_height = width * 9 / 16;
        top = (height - content_height) / 2;
    }
    let side = width.min(content_height);
    if side == width && side == height {
        return image;
    }
    let left = (width - side) / 2;
    let top = top + (content_height - side) / 2;
    image.crop_imm(left, top, side, side)
}

/// Whether a horizontal band of rows is (nearly) black.
fn is_dark_band(image: &::image::DynamicImage, top: u32, rows: u32) -> bool {
    use ::image::GenericImageView;
    if rows == 0 {
        return false;
    }
    let step = (image.width() / 32).max(1);
    let mut total = 0_u64;
    let mut samples = 0_u64;
    for y in top..(top + rows).min(image.height()) {
        for x in (0..image.width()).step_by(step as usize) {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            total += u64::from(r) + u64::from(g) + u64::from(b);
            samples += 3;
        }
    }
    samples > 0 && total / samples < 18
}
fn trim_cache(path: &std::path::Path, keep: Option<&std::path::Path>) {
    trim_cache_to(path, keep, MAX_DISK_ARTWORK_BYTES, MAX_DISK_ARTWORK_FILES);
}

fn trim_cache_to(
    path: &std::path::Path,
    keep: Option<&std::path::Path>,
    max_bytes: u64,
    max_files: usize,
) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    let mut entries: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            if !entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "png" || ext == "img")
            {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then_some((entry, metadata.len(), metadata.modified().ok()))
        })
        .collect();
    let mut total_bytes = entries
        .iter()
        .fold(0u64, |total, (_, bytes, _)| total.saturating_add(*bytes));
    if entries.len() <= max_files && total_bytes <= max_bytes {
        return;
    }
    entries.sort_by_key(|(_, _, modified)| *modified);
    let mut file_count = entries.len();
    for (entry, bytes, _) in entries {
        if file_count <= max_files && total_bytes <= max_bytes {
            break;
        }
        if keep.is_some_and(|keep| keep == entry.path()) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            file_count = file_count.saturating_sub(1);
            total_bytes = total_bytes.saturating_sub(bytes);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // The sizes the tests check: the request for a default-width cover, and the cache and
    // thumbnail readers at the largest size.
    fn artwork_request_url(url: &str) -> String {
        artwork_request_url_for(url, 800)
    }

    fn cached_handle(path: std::path::PathBuf) -> Option<image::Handle> {
        cached_handle_sized(path, LARGE_MAX)
    }

    fn cached_or_remove(path: std::path::PathBuf) -> Option<image::Handle> {
        cached_or_remove_sized(path, LARGE_MAX)
    }

    fn thumbnail(bytes: &[u8]) -> Option<::image::RgbaImage> {
        thumbnail_sized(bytes, LARGE_MAX)
    }

    #[test]
    fn a_download_that_began_before_a_clear_is_not_saved() {
        use std::sync::atomic::Ordering;
        let directory = std::env::temp_dir().join(format!(
            "crabytube-artwork-clear-race-{}",
            std::process::id()
        ));
        let path = directory.join("cover.jpg");
        let before = super::CLEARS.load(Ordering::Acquire);
        super::clearing_folder(|| ());
        assert!(!super::save_unless_cleared(&path, b"cover", before));
        assert!(!path.exists());
        // One that began after the clear is saved. Another test may clear meanwhile, so a few tries.
        let saved = (0..3).any(|_| {
            super::save_unless_cleared(&path, b"cover", super::CLEARS.load(Ordering::Acquire))
        });
        assert!(saved && path.is_file());
        let _ = std::fs::remove_dir_all(directory);
    }
    #[test]
    fn video_frames_are_cropped_to_square_without_letterbox_bars() {
        let mut frame = ::image::RgbaImage::from_pixel(480, 360, ::image::Rgba([0, 0, 0, 255]));
        for y in 45..315 {
            for x in 0..480 {
                frame.put_pixel(x, y, ::image::Rgba([200, 120, 40, 255]));
            }
        }
        let cropped = square_crop(::image::DynamicImage::ImageRgba8(frame));
        assert_eq!((cropped.width(), cropped.height()), (270, 270));
        let rgba = cropped.to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0).0, [200, 120, 40, 255]);
        assert_eq!(rgba.get_pixel(269, 269).0, [200, 120, 40, 255]);

        let wide = ::image::DynamicImage::ImageRgba8(::image::RgbaImage::new(640, 360));
        let cropped = square_crop(wide);
        assert_eq!((cropped.width(), cropped.height()), (360, 360));
    }

    #[test]
    fn retain_urls_keeps_visible_covers_and_their_byte_count() {
        let mut cache = ArtworkCache::default();
        let handle = || image::Handle::from_rgba(8, 8, vec![0; 256]);
        cache.insert("visible".into(), THUMB_MAX, handle());
        cache.insert("visible".into(), CARD_MAX, handle());
        cache.insert("offscreen".into(), THUMB_MAX, handle());
        cache.retain_urls(|url| url == "visible");
        assert!(cache.contains_key("visible"));
        assert!(!cache.contains_key("offscreen"));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), 2 * 256);
    }

    #[test]
    fn a_saved_cover_is_read_from_disk_and_a_missing_one_is_not_fetched() {
        TEST_CACHE_ROOT.get_or_init(|| {
            std::env::temp_dir().join(format!(
                "crabytube-artwork-test-root-{}",
                std::process::id()
            ))
        });
        let url = "https://example.invalid/restored-cover.jpg";
        let thumb = variant_cache_path(url, THUMB_MAX);
        let large = variant_cache_path(url, LARGE_MAX);
        let _ = std::fs::remove_file(&thumb);
        let _ = std::fs::remove_file(&large);
        assert!(
            saved_sized(url, THUMB_MAX).is_none(),
            "nothing is saved, so nothing is read or fetched"
        );
        std::fs::create_dir_all(large.parent().unwrap()).unwrap();
        let mut encoded = std::io::Cursor::new(Vec::new());
        ::image::RgbaImage::from_pixel(8, 8, ::image::Rgba([5, 5, 5, 255]))
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();
        // Only the large cover is saved, as a player request leaves it: it is scaled down.
        std::fs::write(&large, encoded.get_ref()).unwrap();
        assert!(saved_sized(url, THUMB_MAX).is_some());
        let _ = std::fs::remove_file(&large);
    }

    #[test]
    fn a_cover_read_from_disk_is_trimmed_after_the_covers_nobody_read() {
        let dir = std::env::temp_dir().join(format!(
            "crabytube-artwork-access-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut encoded = std::io::Cursor::new(Vec::new());
        ::image::RgbaImage::from_pixel(8, 8, ::image::Rgba([5, 5, 5, 255]))
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();
        let now = std::time::SystemTime::now();
        let hours_ago = |hours: u64| now - std::time::Duration::from_secs(hours * 3600);
        for (name, age) in [("a.img", 3), ("b.img", 2), ("c.img", 1)] {
            let path = dir.join(name);
            std::fs::write(&path, encoded.get_ref()).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(hours_ago(age))
                .unwrap();
        }
        // The oldest file is read, so it becomes the most recently used one.
        assert!(cached_handle_sized(dir.join("a.img"), THUMB_MAX).is_some());

        trim_cache_to(&dir, None, u64::MAX, 2);

        assert!(dir.join("a.img").is_file());
        assert!(!dir.join("b.img").exists());
        assert!(dir.join("c.img").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_least_recently_used_variant_is_the_one_evicted_when_the_budget_is_full() {
        let mut cache = ArtworkCache::default();
        let handle = || image::Handle::from_rgba(8, 8, vec![0; 256]);
        cache.set_budget(Some(2 * 256));
        cache.insert("first".into(), THUMB_MAX, handle());
        cache.insert("second".into(), THUMB_MAX, handle());
        cache.touch("first", THUMB_MAX);
        cache.insert("third".into(), THUMB_MAX, handle());
        assert!(cache.contains_key("first"));
        assert!(!cache.contains_key("second"));
        assert!(cache.contains_key("third"));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), 2 * 256);
    }

    #[test]
    fn remove_variant_drops_one_size_and_keeps_the_other_sizes_of_the_same_cover() {
        let mut cache = ArtworkCache::default();
        let handle = || image::Handle::from_rgba(8, 8, vec![0; 256]);
        cache.insert("cover".into(), THUMB_MAX, handle());
        cache.insert("cover".into(), LARGE_MAX, handle());
        cache.insert("other".into(), LARGE_MAX, handle());
        cache.remove_variant("cover", LARGE_MAX);
        assert!(cache.get_exact("cover", THUMB_MAX).is_some());
        assert!(cache.get_exact("cover", LARGE_MAX).is_none());
        assert!(cache.get_exact("other", LARGE_MAX).is_some());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), 2 * 256);
        cache.remove_variant("missing", LARGE_MAX);
        assert_eq!(cache.bytes(), 2 * 256);
    }

    #[test]
    fn artwork_is_downsampled_before_it_is_cached() {
        let pixels = ::image::RgbaImage::from_pixel(1024, 512, ::image::Rgba([4, 8, 16, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();

        let thumbnail = thumbnail(encoded.get_ref()).unwrap();
        // Cropped to a square first, then capped at the large variant.
        assert_eq!((thumbnail.width(), thumbnail.height()), (512, 512));
    }

    #[test]
    fn artwork_variants_match_their_logical_sizes() {
        let pixels = ::image::RgbaImage::from_pixel(1024, 512, ::image::Rgba([4, 8, 16, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.get_ref();

        let thumb = thumbnail_sized(bytes, THUMB_MAX).unwrap();
        assert!(thumb.width() <= THUMB_MAX && thumb.height() <= THUMB_MAX);
        assert_eq!((thumb.width(), thumb.height()), (128, 128));

        let card = thumbnail_sized(bytes, CARD_MAX).unwrap();
        assert!(card.width() <= CARD_MAX && card.height() <= CARD_MAX);
        assert_eq!((card.width(), card.height()), (320, 320));

        let compact_card = thumbnail_sized(bytes, CARD_MID_MAX).unwrap();
        assert!(compact_card.width() <= CARD_MID_MAX && compact_card.height() <= CARD_MID_MAX);
        assert_eq!((compact_card.width(), compact_card.height()), (192, 192));

        let large = thumbnail_sized(bytes, LARGE_MAX).unwrap();
        assert!(large.width() <= LARGE_MAX && large.height() <= LARGE_MAX);
        assert_eq!((large.width(), large.height()), (512, 512));
    }

    #[test]
    fn card_artwork_variant_tracks_display_scale() {
        assert_eq!(card_variant_for_scale(1.0), THUMB_MAX);
        assert_eq!(card_variant_for_scale(1.2), CARD_MID_MAX);
        assert_eq!(card_variant_for_scale(1.25), CARD_MAX);
        assert_eq!(card_variant_for_scale(2.0), CARD_MAX);
        assert_eq!(card_variant_for_scale(2.1), LARGE_MAX);
        assert_eq!(card_variant_for_scale(f32::NAN), THUMB_MAX);
    }

    #[test]
    fn batch_variant_count_respects_the_decoded_byte_budget() {
        // 16 MiB budget: 128px -> 256 capped to 192 entries, 192px -> 113
        // capped to 192, 320px -> 41, 640px -> 10.
        assert_eq!(
            max_cached_variant_count(THUMB_MAX),
            MAX_ARTWORK_ENTRIES.min(MAX_ARTWORK_BYTES / (128 * 128 * 4))
        );
        assert_eq!(
            max_cached_variant_count(CARD_MID_MAX),
            (MAX_ARTWORK_BYTES / (192 * 192 * 4)).min(MAX_ARTWORK_ENTRIES)
        );
        assert_eq!(
            max_cached_variant_count(CARD_MAX),
            MAX_ARTWORK_BYTES / (320 * 320 * 4)
        );
        assert_eq!(
            max_cached_variant_count(LARGE_MAX),
            MAX_ARTWORK_BYTES / (640 * 640 * 4)
        );
        // Large viewport coverage: 150 small thumbs must fit.
        assert!(max_cached_variant_count(THUMB_MAX) >= 150);
    }

    #[test]
    fn handle_bytes_match_decoded_pixels_and_budget_holds_many_large_covers() {
        let handle = image::Handle::from_rgba(640, 640, vec![0u8; 640 * 640 * 4]);
        let one = handle_bytes(&handle);
        assert_eq!(one, 640 * 640 * 4);
        // The 16 MiB budget holds ten large covers, but not eleven.
        assert!(one.saturating_mul(10) <= MAX_ARTWORK_BYTES);
        assert!(one.saturating_mul(11) > MAX_ARTWORK_BYTES);
    }

    #[test]
    fn cache_selects_nearest_variant_large_enough_for_requested_pixels() {
        let mut cache = ArtworkCache::default();
        cache.insert(
            "cover".into(),
            CARD_MID_MAX,
            image::Handle::from_rgba(192, 192, vec![0; 192 * 192 * 4]),
        );
        cache.insert(
            "cover".into(),
            CARD_MAX,
            image::Handle::from_rgba(320, 320, vec![0; 320 * 320 * 4]),
        );

        assert_eq!(
            handle_bytes(cache.get_for("cover", 160).unwrap()),
            192 * 192 * 4
        );
        assert_eq!(
            handle_bytes(cache.get_for("cover", 193).unwrap()),
            320 * 320 * 4
        );
        assert_eq!(
            handle_bytes(cache.get_for("cover", 640).unwrap()),
            320 * 320 * 4
        );
    }

    #[test]
    fn covers_saved_at_the_same_moment_each_keep_their_own_picture() {
        let names: std::collections::HashSet<String> =
            (0..1000).map(|_| super::temporary_name()).collect();
        assert_eq!(names.len(), 1000);
        let directory = std::env::temp_dir().join(format!(
            "crabytube-artwork-parallel-save-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let start = std::sync::Arc::new(std::sync::Barrier::new(16));
        let saves: Vec<_> = (0..16u8)
            .map(|index| {
                let start = start.clone();
                let path = directory.join(format!("cover-{index}.img"));
                std::thread::spawn(move || {
                    start.wait();
                    assert!(super::atomic_save_bytes(&path, &[index; 4096]));
                })
            })
            .collect();
        for save in saves {
            save.join().unwrap();
        }
        for index in 0..16u8 {
            let saved = std::fs::read(directory.join(format!("cover-{index}.img"))).unwrap();
            assert_eq!(saved, vec![index; 4096]);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persistent_artwork_cache_obeys_byte_and_file_caps() {
        let directory = std::env::temp_dir().join(format!(
            "crabytube-artwork-trim-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let keep = directory.join("current.png");
        for index in 0..6 {
            let path = directory.join(format!("cover-{index}.png"));
            std::fs::write(&path, [index as u8; 4]).unwrap();
        }
        std::fs::write(&keep, b"1234").unwrap();
        std::fs::write(directory.join("unrelated.tmp"), b"keep").unwrap();

        trim_cache_to(&directory, Some(&keep), 10, 3);

        let pngs = std::fs::read_dir(&directory)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
            .collect::<Vec<_>>();
        let total = pngs
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>();
        assert!(pngs.len() <= 3);
        assert!(total <= 10);
        assert!(keep.is_file());
        assert!(directory.join("unrelated.tmp").is_file());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sized_source_requests_match_variant_while_large_keeps_800px() {
        assert_eq!(
            artwork_request_url_for(
                "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj",
                128
            ),
            "https://lh3.googleusercontent.com/abc=w128-h128-l90-rj"
        );
        assert_eq!(
            artwork_request_url("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w800-h800-l90-rj"
        );
        // The catalog hands over its largest cover: a thumbnail is not downloaded at that size.
        assert_eq!(
            artwork_request_url_for(
                "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj",
                128
            ),
            "https://lh3.googleusercontent.com/abc=w128-h128-l90-rj"
        );
        // A wide original keeps its shape.
        assert_eq!(
            artwork_request_url_for("https://lh3.googleusercontent.com/abc=w1280-h720", 320),
            "https://lh3.googleusercontent.com/abc=w320-h180"
        );
    }

    #[test]
    fn only_youtube_cover_hosts_over_https_are_asked() {
        assert!(is_cover_host("https://i.ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(is_cover_host(
            "https://lh3.googleusercontent.com/abc=w544-h544"
        ));
        assert!(is_cover_host("https://yt3.ggpht.com/abc"));
        assert!(!is_cover_host("http://i.ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(!is_cover_host("https://tracker.example/x.jpg"));
        assert!(!is_cover_host("https://ytimg.com.example/x.jpg"));
        assert!(!is_cover_host("not a url"));
    }

    #[test]
    fn video_thumbnails_request_hqdefault_instead_of_tiny_crops() {
        assert_eq!(
            artwork_request_url("https://i.ytimg.com/vi/abc123/default.jpg"),
            "https://i.ytimg.com/vi/abc123/hqdefault.jpg"
        );
        assert_eq!(
            artwork_request_url("https://i.ytimg.com/vi/abc123/mqdefault.jpg"),
            "https://i.ytimg.com/vi/abc123/hqdefault.jpg"
        );
    }

    #[test]
    fn google_artwork_requests_larger_source_when_available() {
        assert_eq!(
            artwork_request_url("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w800-h800-l90-rj"
        );
        assert_eq!(
            artwork_request_url("https://example.com/abc=w120-h120"),
            "https://example.com/abc=w120-h120"
        );
    }

    #[test]
    fn cached_artwork_returns_predecoded_rgba() {
        let path =
            std::env::temp_dir().join(format!("crabytube-artwork-test-{}.png", std::process::id()));
        let fixture = ::image::RgbaImage::from_pixel(12, 8, ::image::Rgba([4, 8, 16, 255]));
        fixture.save(&path).unwrap();

        let handle = cached_handle(path.clone()).unwrap();
        match handle {
            image::Handle::Rgba {
                width,
                height,
                pixels,
                ..
            } => {
                assert_eq!((width, height), (8, 8));
                assert_eq!(pixels.len(), (width * height * 4) as usize);
            }
            _ => panic!("cached image was not decoded to RGBA"),
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn corrupt_cache_entry_is_detectable_for_network_fallback() {
        let path = std::env::temp_dir().join(format!(
            "crabytube-artwork-corrupt-test-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, b"not an image").unwrap();
        assert!(cached_or_remove(path.clone()).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn variant_disk_paths_are_distinct_and_legacy_differs() {
        let small = variant_cache_path("https://example.com/cover", THUMB_MAX);
        let card = variant_cache_path("https://example.com/cover", CARD_MAX);
        let large = variant_cache_path("https://example.com/cover", LARGE_MAX);
        assert_ne!(small, card);
        assert_ne!(card, large);
        assert_ne!(small, large);
        // Legacy v4 path differs from all v5 variant paths.
        let legacy = legacy_cache_path("https://example.com/cover");
        assert_ne!(legacy, small);
        assert_ne!(legacy, large);
    }

    use std::sync::Arc;

    /// Serves one small PNG per connection after a short delay, and records the
    /// highest number of requests it has open at once.
    fn slow_cover_server(delay: Duration) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut png = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(
            8,
            8,
            ::image::Rgba([1, 2, 3, 255]),
        ))
        .write_to(&mut png, ::image::ImageFormat::Png)
        .unwrap();
        let png = Arc::new(png.into_inner());
        let open = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let peak_seen = Arc::clone(&peak);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (open, peak, png) =
                    (Arc::clone(&open), Arc::clone(&peak_seen), Arc::clone(&png));
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut request = [0u8; 4096];
                    let read = stream.read(&mut request).unwrap_or(0);
                    let missing = String::from_utf8_lossy(&request[..read]).contains("missing");
                    let now = open.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(delay);
                    open.fetch_sub(1, Ordering::SeqCst);
                    let head = if missing {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            png.len()
                        )
                    };
                    let _ = stream.write_all(head.as_bytes());
                    if !missing {
                        let _ = stream.write_all(&png);
                    }
                });
            }
        });
        (address, peak)
    }

    #[test]
    fn list_covers_download_in_parallel_up_to_the_slot_limit() {
        TEST_CACHE_ROOT.get_or_init(|| {
            std::env::temp_dir().join(format!(
                "crabytube-artwork-test-root-{}",
                std::process::id()
            ))
        });
        let (address, peak) = slow_cover_server(Duration::from_millis(150));
        let nonce = std::process::id();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        let covers = runtime.block_on(async {
            futures_util::future::join_all((0..24).map(|index| {
                let url = format!("http://{address}/list-{nonce}-{index}.png");
                fetch_sized(url, CancellationToken::new(), THUMB_MAX)
            }))
            .await
        });
        assert!(
            covers.iter().all(Option::is_some),
            "every list cover arrives"
        );
        let peak = peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            peak >= 6,
            "only {peak} covers were downloading at once; a visible page needs more lanes"
        );
    }

    #[test]
    fn player_cover_asks_for_the_largest_video_still_first() {
        let url = "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg";
        assert_eq!(
            cover_candidates(url, Lane::Player, LARGE_MAX),
            [
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg",
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/sddefault.jpg",
                "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
            ]
        );
        // Lists keep the small still; only the player pays for the large one.
        assert_eq!(cover_candidates(url, Lane::List, THUMB_MAX), [url]);
        // Other hosts and paths are requested as they are.
        assert_eq!(
            cover_candidates(
                "https://i.ytimg.com/vi_webp/abc/x.webp",
                Lane::Player,
                LARGE_MAX
            ),
            ["https://i.ytimg.com/vi_webp/abc/x.webp"]
        );
        assert_eq!(
            cover_candidates(
                "https://example.com/vi/abc/hqdefault.jpg",
                Lane::Player,
                LARGE_MAX
            ),
            ["https://example.com/vi/abc/hqdefault.jpg"]
        );
    }

    #[test]
    fn a_missing_largest_still_falls_back_to_the_next_size() {
        let (address, _) = slow_cover_server(Duration::ZERO);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let response = runtime.block_on(async {
            let http = crate::network::shared().unwrap();
            let candidates = [
                format!("http://{address}/missing-maxres.jpg"),
                format!("http://{address}/sd.jpg"),
            ];
            first_available(http, &CancellationToken::new(), &candidates).await
        });
        let response = response.expect("the smaller still answers");
        assert!(response.url().path().ends_with("/sd.jpg"));
    }

    #[test]
    fn small_variant_persists_atomically_and_survives_ram_eviction() {
        // A 128px thumb saved via the variant path must decode back at
        // 128px without needing the network or a 640px master.
        let dir = std::env::temp_dir().join(format!(
            "crabytube-variant-persist-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("thumb.png");
        let fixture = ::image::RgbaImage::from_pixel(256, 256, ::image::Rgba([9, 9, 9, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        fixture
            .write_to(&mut encoded, ::image::ImageFormat::Png)
            .unwrap();
        assert!(atomic_save_bytes(&path, encoded.get_ref()));
        assert!(path.is_file());
        // The file holds the bytes as they were given: nothing is re-encoded on the way to disk.
        assert_eq!(std::fs::read(&path).unwrap(), encoded.get_ref().as_slice());
        // No stray temp files remain.
        let tmps = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .count();
        assert_eq!(tmps, 0);
        let handle = cached_or_remove_sized(path.clone(), THUMB_MAX).unwrap();
        match handle {
            image::Handle::Rgba { width, height, .. } => {
                assert!(width <= THUMB_MAX && height <= THUMB_MAX);
            }
            _ => panic!("variant was not decoded to RGBA"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
