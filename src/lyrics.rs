//! Lyrics timings are supplied by the source, never synthesized from plain text.
use crate::music::Track;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::info;

/// Parser version: bump invalidates old disk entries.
pub const PARSER_VERSION: u32 = 4;
/// Disk budget for lyrics JSON (small text, not pixels).
pub const MAX_LYRICS_BYTES: u64 = 2 * 1024 * 1024;
/// Time-to-live for cached lyrics.
pub const LYRICS_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// Global deadline for the whole provider chain; the providers run concurrently.
pub const LYRICS_DEADLINE: Duration = Duration::from_secs(10);
/// Time-to-live for "no lyrics found", so a missing track is not searched again every time.
const NEGATIVE_TTL: Duration = Duration::from_secs(24 * 3600);
/// After a line-timed result, word-timed sources get this long to answer.
const LINE_GRACE: Duration = Duration::from_millis(1500);
const NOT_FOUND: &str = "No matching lyrics available from the enabled sources. Try again later.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Line {
    pub start: f64,
    pub end: Option<f64>,
    pub text: String,
    pub words: Vec<Word>,
    pub translation: Option<String>,
    /// Romanized text (TTML `x-roman`), shown under the line like a translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub romanization: Option<String>,
    /// Background vocals (TTML `x-bg`), kept apart from the main line and its words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lyrics {
    pub source: String,
    pub lines: Vec<Line>,
    pub plain: Option<String>,
}
impl Lyrics {
    pub fn active(&self, seconds: f64) -> Option<usize> {
        self.lines
            .iter()
            .rposition(|line| line.start <= seconds && line.end.is_none_or(|end| seconds < end))
    }

    pub fn is_instrumental_text(text: &str) -> bool {
        let lower = text.trim().to_lowercase();
        lower == "instrumental"
            || lower.starts_with("instrumental ")
            || lower.contains("[instrumental]")
            || lower.contains("(instrumental)")
    }
}

/// Normalize for cache keys and matching: lowercase, trim, collapse spaces,
/// round duration to whole seconds to avoid Debug-format duplication.
pub fn normalized_identity(track: &Track, duration: Option<Duration>) -> String {
    fn norm(s: &str) -> String {
        s.trim()
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    let secs = duration.map(|d| d.as_secs()).unwrap_or(u64::MAX);
    format!(
        "v{PARSER_VERSION}:{}|{}|{}|{}|{secs}",
        track.video_id.trim(),
        norm(&track.title),
        norm(&track.artist),
        norm(track.album.as_deref().unwrap_or_default()),
    )
}

pub fn lyrics_cache_path(track: &Track, duration: Option<Duration>) -> std::path::PathBuf {
    let key = normalized_identity(track, duration);
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("crabytube/lyrics")
        .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}

fn cache_is_fresh(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|modified| modified.elapsed().unwrap_or(Duration::MAX) <= LYRICS_TTL)
        .unwrap_or(false)
}

fn trim_lyrics_cache(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((e.path(), meta.len(), meta.modified().ok()?))
        })
        .collect();
    let total: u64 = files.iter().map(|(_, len, _)| *len).sum();
    if total <= MAX_LYRICS_BYTES {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut freed = 0u64;
    let need = total - MAX_LYRICS_BYTES;
    for (path, len, _) in files {
        if freed >= need {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            freed += len;
        }
    }
}

/// Version markers that must not be confused across covers/remixes/live. Whole words only:
/// "live" must not match "alive".
fn version_markers(s: &str) -> Vec<String> {
    let lower = s.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut found: Vec<String> = [
        "live",
        "remix",
        "cover",
        "acoustic",
        "instrumental",
        "karaoke",
    ]
    .into_iter()
    .filter(|marker| words.contains(marker))
    .map(str::to_owned)
    .collect();
    if words.windows(2).any(|pair| pair == ["sped", "up"]) {
        found.push("sped up".into());
    }
    if words.contains(&"slowed") {
        found.push("slowed".into());
    }
    found
}

fn base_title(s: &str) -> String {
    // Strip parenthetical/bracket suffixes for base comparison only; version
    // markers are compared separately so live/remix/cover never mismatch.
    let mut out = s.to_owned();
    // ASCII lowercasing keeps byte offsets valid for slicing the original.
    let lower = out.to_ascii_lowercase();
    for marker in [" - remaster", " - from "] {
        if let Some(index) = lower.find(marker) {
            out.truncate(index);
        }
    }
    while let Some(start) = out.rfind(['(', '[']) {
        let end = if out[start..].contains(')') {
            out[start..].find(')').map(|i| start + i + 1)
        } else if out[start..].contains(']') {
            out[start..].find(']').map(|i| start + i + 1)
        } else {
            None
        };
        let Some(end) = end else { break };
        out.replace_range(start..end, "");
    }
    out.trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits a credit such as "A, B & C feat. D" into lowercase names.
fn artist_names(credit: &str) -> Vec<String> {
    let mut text = credit.to_lowercase();
    for separator in [" feat. ", " ft. ", " featuring ", " & ", " x ", ","] {
        text = text.replace(separator, "\u{1}");
    }
    text.split('\u{1}')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Two credits match when they share at least one artist.
pub fn artists_match(query: &str, candidate: &str) -> bool {
    let wanted = artist_names(query);
    artist_names(candidate)
        .iter()
        .any(|name| wanted.contains(name))
}

pub fn titles_match(query: &str, candidate: &str) -> bool {
    if query.eq_ignore_ascii_case(candidate) {
        return true;
    }
    if base_title(query) != base_title(candidate) {
        return false;
    }
    // Same base: version markers must agree, otherwise live/remix/cover confusion.
    version_markers(query) == version_markers(candidate)
}
fn timestamp(raw: &str) -> Option<f64> {
    if let Some(ms) = raw.strip_suffix("ms") {
        return ms.parse::<f64>().ok().map(|v| v / 1000.0);
    }
    if let Some(s) = raw.strip_suffix('s') {
        return s.parse().ok();
    }
    let mut total = 0.0;
    for part in raw.split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    total.is_finite().then_some(total)
}
pub fn lrc(input: &str, source: &str) -> Lyrics {
    let mut lines = Vec::new();
    let mut offset = 0.0;
    for raw in input.lines() {
        if let Some(v) = raw
            .strip_prefix("[offset:")
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.parse::<f64>().ok())
        {
            offset = v / 1000.0;
            continue;
        }
        let mut rest = raw;
        let mut times = Vec::new();
        while let Some(tail) = rest.strip_prefix('[') {
            let Some((time, text)) = tail.split_once(']') else {
                break;
            };
            if !time.contains(':') {
                break;
            }
            let Some(time) = timestamp(time) else { break };
            times.push(time);
            rest = text;
        }
        for start in times {
            lines.push(Line {
                start,
                end: None,
                text: rest.trim().into(),
                words: vec![],
                translation: None,
                romanization: None,
                background: None,
            });
        }
    }
    for line in &mut lines {
        line.start = (line.start + offset).max(0.0);
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    for i in 0..lines.len().saturating_sub(1) {
        lines[i].end = Some(lines[i + 1].start);
    }
    Lyrics {
        source: source.into(),
        plain: lines.is_empty().then(|| input.to_owned()),
        lines,
    }
}
/// The text when it has any visible characters.
fn non_blank(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub fn ttml(input: &str) -> Option<Lyrics> {
    let doc = roxmltree::Document::parse(input).ok()?;
    let mut lines = Vec::new();
    for p in doc
        .descendants()
        .filter(|node| node.tag_name().name() == "p")
    {
        let Some(start) = p.attribute("begin").and_then(timestamp) else {
            continue;
        };
        let mut words = Vec::new();
        let mut text = String::new();
        let mut translation = String::new();
        let mut romanization = String::new();
        let mut background = String::new();
        for n in p.descendants().filter(|n| n.is_text()) {
            // The role of the nearest annotated ancestor decides where the text belongs.
            let has_role = |needle: &str| {
                n.ancestors().any(|a| {
                    a.attributes()
                        .any(|attr| attr.name() == "role" && attr.value().contains(needle))
                })
            };
            let value = n.text().unwrap_or_default();
            if has_role("translation") {
                translation.push_str(value);
                continue;
            }
            if has_role("roman") {
                romanization.push_str(value);
                continue;
            }
            if has_role("x-bg") {
                background.push_str(value);
                continue;
            }
            text.push_str(value);
            if let Some(parent) = n
                .ancestors()
                .find(|node| node.tag_name().name() == "span" && node.attribute("begin").is_some())
                && let (Some(begin), Some(end)) = (
                    parent.attribute("begin").and_then(timestamp),
                    parent.attribute("end").and_then(timestamp),
                )
            {
                words.push(Word {
                    start: begin,
                    end,
                    text: value.into(),
                });
            } else if !words.is_empty() && value.trim().is_empty() {
                words.last_mut()?.text.push_str(value);
            }
        }
        lines.push(Line {
            start,
            end: p.attribute("end").and_then(timestamp),
            text: text.trim().into(),
            words,
            translation: (!translation.is_empty()).then_some(translation),
            romanization: non_blank(romanization),
            background: non_blank(background),
        });
    }
    lines.sort_by(|a, b| a.start.total_cmp(&b.start));
    (!lines.is_empty()).then(|| Lyrics {
        source: "Better Lyrics / TTML".into(),
        lines,
        plain: None,
    })
}
pub fn qrc(input: &str) -> Option<Lyrics> {
    let doc = roxmltree::Document::parse(input).ok()?;
    let mut lines = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("Lyric_1")) {
        let Some(content) = node.attribute("LyricContent") else {
            continue;
        };
        let mut remaining = content;
        while let Some(open) = remaining.find('[') {
            let after_open = &remaining[open + 1..];
            let Some(close) = after_open.find(']') else {
                break;
            };
            let header = &after_open[..close];
            let after_header = &after_open[close + 1..];
            let Some((start_ms, duration_ms)) = header.split_once(',') else {
                remaining = after_header;
                continue;
            };
            let (Ok(start_ms), Ok(duration_ms)) =
                (start_ms.parse::<f64>(), duration_ms.parse::<f64>())
            else {
                // Metadata such as [ti:], [ar:] and [offset:] is not a lyric line.
                remaining = after_header;
                continue;
            };
            // XML normalizes literal line breaks inside attributes to spaces.
            // The next numeric [start,duration] token still safely delimits a line.
            let body_end = after_header.find('[').unwrap_or(after_header.len());
            let body = &after_header[..body_end];
            remaining = &after_header[body_end..];
            let start = start_ms / 1000.0;
            let duration = duration_ms / 1000.0;
            let mut words = Vec::new();
            let mut text = String::new();
            let mut rest = body;
            while let Some((before, timing_tail)) = rest.split_once('(') {
                let Some((timing, after)) = timing_tail.split_once(')') else {
                    text.push_str(rest);
                    rest = "";
                    break;
                };
                let mut parts = timing.split(',');
                let Some(begin) = parts.next().and_then(|s| s.parse::<f64>().ok()) else {
                    text.push_str(before);
                    text.push('(');
                    rest = timing_tail;
                    continue;
                };
                let Some(length) = parts.next().and_then(|s| s.parse::<f64>().ok()) else {
                    text.push_str(before);
                    text.push('(');
                    rest = timing_tail;
                    continue;
                };
                text.push_str(before);
                if !before.is_empty() {
                    words.push(Word {
                        start: begin / 1000.0,
                        end: (begin + length) / 1000.0,
                        text: before.into(),
                    });
                }
                rest = after;
            }
            text.push_str(rest);
            let words_end = words.iter().map(|word| word.end).fold(start, f64::max);
            let text = text.trim().to_owned();
            if !text.is_empty() {
                lines.push(Line {
                    start,
                    end: Some((start + duration).max(words_end)),
                    text,
                    words,
                    translation: None,
                    romanization: None,
                    background: None,
                });
            }
        }
    }
    (!lines.is_empty()).then(|| Lyrics {
        source: "QQ Music via Better Lyrics".into(),
        lines,
        plain: None,
    })
}

async fn json(client: &reqwest::Client, url: &str, params: &[(&str, String)]) -> Option<Value> {
    let mut url = reqwest::Url::parse(url).ok()?;
    url.query_pairs_mut()
        .extend_pairs(params.iter().map(|(key, value)| (*key, value.as_str())));
    let r = client
        .get(url)
        .timeout(Duration::from_secs(6))
        .header(reqwest::header::USER_AGENT, "CrabyTube/0.1")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if r.content_length().is_some_and(|n| n > 2 * 1024 * 1024) {
        return None;
    }
    use futures_util::StreamExt;
    let mut stream = r.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}
fn shared_client() -> Result<reqwest::Client, String> {
    crate::network::shared()
        .cloned()
        .map_err(|_| "Lyrics network setup failed".to_owned())
}

pub async fn load(track: Track, duration: Option<Duration>) -> Result<Lyrics, String> {
    load_with_progress(track, duration, |_| {}).await
}

/// On-disk marker for a track that no source had lyrics for.
#[derive(Serialize, Deserialize)]
struct NotFound {
    not_found: bool,
}

fn negative_is_fresh(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|modified| modified.elapsed().unwrap_or(Duration::MAX) <= NEGATIVE_TTL)
        .unwrap_or(false)
}

pub async fn load_with_progress<F>(
    track: Track,
    duration: Option<Duration>,
    mut on_progress: F,
) -> Result<Lyrics, String>
where
    F: FnMut(Lyrics) + Send + 'static,
{
    let started = std::time::Instant::now();
    // Sources add credits around the song; the labels and the title and artist are this track's.
    let (title, artist) = (track.title.clone(), track.artist.clone());
    let path = lyrics_cache_path(&track, duration);
    if path.is_file()
        && cache_is_fresh(&path)
        && let Ok(bytes) = tokio::fs::read(&path).await
    {
        if let Ok(lyrics) = serde_json::from_slice::<Lyrics>(&bytes) {
            return Ok(lyrics);
        }
        if serde_json::from_slice::<NotFound>(&bytes).is_ok_and(|entry| entry.not_found)
            && negative_is_fresh(&path)
        {
            return Err(NOT_FOUND.into());
        }
    }
    // Keep the newest plain text, so a deadline can still return it.
    let latest_plain = std::sync::Arc::new(std::sync::Mutex::new(None::<Lyrics>));
    let stash = std::sync::Arc::clone(&latest_plain);
    let (progress_title, progress_artist) = (title.clone(), artist.clone());
    let progress = move |lyrics: Lyrics| {
        let lyrics = drop_credit_lines(lyrics, &progress_title, &progress_artist);
        if lyrics.lines.is_empty() && lyrics.plain.is_none() {
            return;
        }
        if let Ok(mut slot) = stash.lock() {
            *slot = Some(lyrics.clone());
        }
        on_progress(lyrics);
    };
    // Global deadline for the whole chain; providers share one client.
    let result = match tokio::time::timeout(LYRICS_DEADLINE, load_inner(track, duration, progress))
        .await
    {
        Ok(result) => result,
        Err(_) => match latest_plain.lock().ok().and_then(|slot| slot.clone()) {
            Some(plain) => Ok(plain),
            None => Err("Lyrics request timed out. Check your connection and try again.".into()),
        },
    };
    // A source that holds only credits has no lyrics to show.
    let result = result.and_then(|lyrics| {
        let lyrics = drop_credit_lines(lyrics, &title, &artist);
        if lyrics.lines.is_empty() && lyrics.plain.is_none() {
            Err(NOT_FOUND.into())
        } else {
            Ok(lyrics)
        }
    });
    match &result {
        Ok(lyrics) => {
            info!(elapsed_ms = started.elapsed().as_millis(), source = %lyrics.source, timed = !lyrics.lines.is_empty(), "lyrics loaded")
        }
        Err(error) => {
            info!(elapsed_ms = started.elapsed().as_millis(), %error, "lyrics not loaded")
        }
    }
    let to_store = match &result {
        Ok(lyrics) => serde_json::to_vec(lyrics).ok(),
        Err(error) if error == NOT_FOUND => serde_json::to_vec(&NotFound { not_found: true }).ok(),
        Err(_) => None,
    };
    if let Some(data) = to_store
        && let Some(parent) = path.parent()
    {
        let _ = tokio::fs::create_dir_all(parent).await;
        if tokio::fs::write(&path, data).await.is_ok() {
            let parent = parent.to_path_buf();
            drop(tokio::task::spawn_blocking(move || {
                trim_lyrics_cache(&parent)
            }));
        }
    }
    result
}

/// What one provider found, ranked by timing quality.
enum Found {
    /// Word-level timing (TTML, QRC).
    Word(Lyrics),
    /// Line-level timing (LRC).
    Line(Lyrics),
    Plain(Lyrics),
    Instrumental,
    Missing,
    Unreachable,
}

/// Labels that open a credit line in lyric sources, in English and Chinese. A credit has one of
/// these labels, then a colon (":" or the full-width "：").
const CREDIT_LABELS: [&str; 14] = [
    "lyrics by",
    "lyric by",
    "composed by",
    "written by",
    "produced by",
    "arranged by",
    "lyricist",
    "composer",
    "作词",
    "作曲",
    "编曲",
    "制作人",
    "词",
    "曲",
];

/// Whether a line credits the song rather than sings it: a labelled credit, or "title - artist"
/// for this track.
fn is_credit_line(text: &str, title: &str, artist: &str) -> bool {
    let text = text.trim();
    let lower = text.to_lowercase();
    let labelled = CREDIT_LABELS.iter().any(|label| {
        lower.strip_prefix(label).is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.starts_with(':') || rest.starts_with('：')
        })
    });
    labelled || is_title_and_artist(text, title, artist)
}

/// "Title - Artist" for this track. The title may carry a suffix such as "(Explicit)". Without a
/// known artist nothing is recognised, so a real line is never removed by guesswork.
fn is_title_and_artist(text: &str, title: &str, artist: &str) -> bool {
    if artist.trim().is_empty() {
        return false;
    }
    let Some((left, right)) = text.rsplit_once(" - ").or_else(|| text.rsplit_once(" – ")) else {
        return false;
    };
    titles_match(title, left.trim()) && artists_match(artist, right.trim())
}

/// Removes the credits sources put before the first sung line and after the last one. Credits in
/// the middle stay. The lines that remain keep their own timing, so the next line still starts
/// when it did.
pub fn drop_credit_lines(mut lyrics: Lyrics, title: &str, artist: &str) -> Lyrics {
    while lyrics
        .lines
        .first()
        .is_some_and(|line| is_credit_line(&line.text, title, artist))
    {
        lyrics.lines.remove(0);
    }
    while lyrics
        .lines
        .last()
        .is_some_and(|line| is_credit_line(&line.text, title, artist))
    {
        lyrics.lines.pop();
    }
    if let Some(plain) = lyrics.plain.take() {
        let rows: Vec<&str> = plain.lines().collect();
        let mut first = 0;
        let mut last = rows.len();
        while first < last && is_credit_line(rows[first], title, artist) {
            first += 1;
        }
        while last > first && is_credit_line(rows[last - 1], title, artist) {
            last -= 1;
        }
        let kept = rows[first..last].join("\n");
        lyrics.plain = (!kept.trim().is_empty()).then_some(kept);
    }
    lyrics
}

fn plain_found(source: &str, text: &str) -> Found {
    if Lyrics::is_instrumental_text(text) {
        return Found::Instrumental;
    }
    Found::Plain(Lyrics {
        source: source.into(),
        lines: vec![],
        plain: Some(text.into()),
    })
}

async fn lrclib_provider(
    client: &reqwest::Client,
    track: &Track,
    duration: Option<Duration>,
) -> Found {
    let params = vec![
        ("track_name", track.title.clone()),
        ("artist_name", track.artist.clone()),
    ];
    let Some(value) = json(client, "https://lrclib.net/api/search", &params).await else {
        return Found::Unreachable;
    };
    let Value::Array(entries) = value else {
        return Found::Missing;
    };
    let best = entries
        .iter()
        .filter(|v| {
            v.get("trackName")
                .and_then(Value::as_str)
                .is_some_and(|s| titles_match(&track.title, s))
                && v.get("artistName")
                    .and_then(Value::as_str)
                    .is_some_and(|s| artists_match(&track.artist, s))
        })
        .filter(|v| {
            duration.is_none_or(|d| {
                v.get("duration")
                    .and_then(Value::as_f64)
                    .is_some_and(|n| (n - d.as_secs_f64()).abs() < 3.0)
            })
        })
        .min_by_key(|v| v.get("syncedLyrics").and_then(Value::as_str).is_none());
    let Some(entry) = best else {
        return Found::Missing;
    };
    if let Some(s) = entry.get("syncedLyrics").and_then(Value::as_str) {
        return Found::Line(lrc(s, "LRCLIB"));
    }
    match entry.get("plainLyrics").and_then(Value::as_str) {
        Some(s) => plain_found("LRCLIB", s),
        None => Found::Missing,
    }
}

async fn better_lyrics_provider(client: &reqwest::Client, params: &[(&str, String)]) -> Found {
    let Some(v) = json(client, "https://api.betterlyrics.org/getLyrics", params).await else {
        return Found::Unreachable;
    };
    let confident = v
        .get("score")
        .and_then(Value::as_f64)
        .is_none_or(|score| score >= 0.7);
    match v
        .get("ttml")
        .and_then(Value::as_str)
        .filter(|_| confident)
        .and_then(ttml)
    {
        Some(lyrics) => Found::Word(lyrics),
        None => Found::Missing,
    }
}

async fn better_lyrics_qq_provider(client: &reqwest::Client, params: &[(&str, String)]) -> Found {
    let Some(v) = json(client, "https://api.betterlyrics.org/qq/getLyrics", params).await else {
        return Found::Unreachable;
    };
    match v.get("lyrics").and_then(Value::as_str).and_then(qrc) {
        Some(lyrics) => Found::Word(lyrics),
        None => Found::Missing,
    }
}

async fn better_lyrics_kugou_provider(
    client: &reqwest::Client,
    params: &[(&str, String)],
) -> Found {
    let Some(v) = json(
        client,
        "https://api.betterlyrics.org/kugou/getLyrics",
        params,
    )
    .await
    else {
        return Found::Unreachable;
    };
    let Some(s) = v.get("lyrics").and_then(Value::as_str) else {
        return Found::Missing;
    };
    let parsed = lrc(s, "Kugou via Better Lyrics");
    if !parsed.lines.is_empty() {
        Found::Line(parsed)
    } else {
        plain_found("Kugou via Better Lyrics", s)
    }
}

async fn load_inner<F>(
    track: Track,
    duration: Option<Duration>,
    mut on_progress: F,
) -> Result<Lyrics, String>
where
    F: FnMut(Lyrics),
{
    let client = shared_client()?;
    let mut params = vec![
        ("s", track.title.clone()),
        ("a", track.artist.clone()),
        // Album and duration substantially improve matching and provider cache hits.
        ("al", track.album.clone().unwrap_or_default()),
        ("videoId", track.video_id.clone()),
    ];
    if let Some(d) = duration {
        params.push(("d", d.as_secs().to_string()));
    }
    // Every provider starts at once. The Better Lyrics sources need a duration.
    let mut running = FuturesUnordered::new();
    let mut started = 1_usize;
    running.push(lrclib_provider(&client, &track, duration).boxed());
    if duration.is_some() {
        let (client, params) = (&client, &params);
        running.push(better_lyrics_provider(client, params).boxed());
        running.push(better_lyrics_qq_provider(client, params).boxed());
        running.push(better_lyrics_kugou_provider(client, params).boxed());
        started += 3;
    }
    let mut best_line: Option<Lyrics> = None;
    let mut best_plain: Option<Lyrics> = None;
    let mut instrumental = false;
    let mut unreachable = 0_usize;
    loop {
        // Once a line-timed result exists, a word-timed one may still arrive shortly.
        let next = if best_line.is_some() {
            match tokio::time::timeout(LINE_GRACE, running.next()).await {
                Ok(next) => next,
                Err(_) => break,
            }
        } else {
            running.next().await
        };
        let Some(found) = next else {
            break;
        };
        match found {
            Found::Word(lyrics) => return Ok(lyrics),
            Found::Line(lyrics) => {
                best_line.get_or_insert(lyrics);
            }
            Found::Plain(lyrics) => {
                if best_plain.is_none() {
                    // Plain text shows at once; a timed result replaces it when one arrives.
                    on_progress(lyrics.clone());
                    best_plain = Some(lyrics);
                }
            }
            Found::Instrumental => instrumental = true,
            Found::Unreachable => unreachable += 1,
            Found::Missing => {}
        }
    }
    drop(running);
    if let Some(lyrics) = best_line {
        return Ok(lyrics);
    }
    if let Some(plain) = best_plain {
        return Ok(plain);
    }
    if instrumental {
        return Err("Instrumental track — no lyrics to display.".into());
    }
    if unreachable == started {
        return Err(
            "Lyrics providers are unreachable. Check your connection and try again.".into(),
        );
    }
    Err(NOT_FOUND.into())
}

/// Estimated wrapping geometry for Iced's lyric text (the widget API does not
/// expose measured content heights). Wider scripts count closer to a full em.
pub fn estimated_line_height(
    line: &Line,
    width: f32,
    font_size: f32,
    show_translation: bool,
) -> f32 {
    fn text_height(text: &str, width: f32, font_size: f32) -> f32 {
        let units = text
            .chars()
            .map(|character| if character.is_ascii() { 0.54 } else { 1.0 })
            .sum::<f32>();
        let columns = (width.max(font_size * 4.0) / (font_size * 0.72)).max(1.0);
        let wraps = (units / columns).ceil().max(1.0);
        wraps * font_size * 1.42
    }

    let font_size = font_size.clamp(12.0, 28.0);
    let mut height = text_height(&line.text, width, font_size) + 10.0 + LYRICS_SPACING;
    if show_translation
        && let Some(translation) = line
            .translation
            .as_deref()
            .filter(|text| !text.trim().is_empty())
    {
        height += text_height(translation, width, 12.0) + 2.0;
    }
    if show_translation && let Some(romanization) = line.romanization.as_deref() {
        height += text_height(romanization, width, 12.0) + 2.0;
    }
    if let Some(background) = line.background.as_deref() {
        height += text_height(background, width, 12.0) + 2.0;
    }
    height
}

pub const LYRICS_SPACING: f32 = 2.0;

pub fn estimated_scroll_offset(heights: &[f32], index: usize) -> f32 {
    heights.iter().take(index.saturating_sub(2)).sum::<f32>()
}

/// Returns visible row indices plus the top/bottom spacer heights.
pub fn virtual_window(
    heights: &[f32],
    scroll_y: f32,
    viewport_height: f32,
    overscan: usize,
) -> (std::ops::Range<usize>, f32, f32) {
    let mut prefix = Vec::with_capacity(heights.len() + 1);
    prefix.push(0.0);
    for height in heights {
        prefix.push(prefix.last().copied().unwrap_or(0.0) + height.max(1.0));
    }
    let total_height = *prefix.last().unwrap_or(&0.0);
    let start_at = scroll_y.max(0.0);
    let end_at = start_at + viewport_height.max(0.0);
    let first_visible = prefix
        .partition_point(|bottom| *bottom <= start_at)
        .min(heights.len());
    let after_visible = prefix
        .partition_point(|bottom| *bottom < end_at)
        .min(heights.len());
    let start = first_visible.saturating_sub(overscan);
    let end = after_visible.saturating_add(overscan).min(heights.len());
    (
        start..end,
        prefix[start],
        (total_height - prefix[end]).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lrc_multiple_timestamps_and_offset() {
        let l = lrc(
            "[offset:-200]\n[00:01.50][00:03.50]Hello\n[00:05.00]World",
            "fixture",
        );
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[0].start, 1.3);
        assert_eq!(l.active(4.0), Some(1));
        assert_eq!(l.active(0.1), None);
    }
    #[test]
    fn qq_qrc_has_word_level_timing_from_provider_data() {
        let l =
            qrc(r#"<Lyric_1 LyricContent="[1000,1500]Hello(1000,400) world(1400,700)"/>"#).unwrap();
        assert_eq!(l.lines[0].text, "Hello world");
        assert_eq!(l.lines[0].words.len(), 2);
        assert_eq!(l.lines[0].words[1].start, 1.4);
        assert_eq!(l.source, "QQ Music via Better Lyrics");
    }
    #[test]
    fn qq_qrc_parses_every_line_and_skips_metadata() {
        let l = qrc(r#"<Lyric_1 LyricContent="[ti:Song]
[1000,1500]Hello(1000,400) world(1400,700)
[2500,1200]Again(2500,500) soon(3000,500)"/>"#)
        .unwrap();
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].text, "Hello world");
        assert_eq!(l.lines[1].text, "Again soon");
        assert_eq!(l.lines[1].start, 2.5);
        assert_eq!(l.lines[1].words[1].end, 3.5);
    }
    #[test]
    fn plain_has_no_invented_timings() {
        let l = lrc("Hello world", "fixture");
        assert!(l.lines.is_empty());
        assert!(l.plain.is_some());
    }

    #[test]
    fn lyric_virtual_window_accounts_for_wrapping_and_translation() {
        let long = Line {
            start: 0.0,
            end: None,
            text: "A very long lyric line that wraps when the panel is narrow. ".repeat(8),
            words: Vec::new(),
            translation: Some("Una traducción extensa para probar el ajuste de línea.".repeat(4)),
            romanization: None,
            background: None,
        };
        let short = Line {
            start: 1.0,
            end: None,
            text: "Short".into(),
            words: Vec::new(),
            translation: None,
            romanization: None,
            background: None,
        };
        let wide = estimated_line_height(&long, 520.0, 18.0, true);
        let narrow = estimated_line_height(&long, 180.0, 18.0, true);
        assert!(narrow > wide);
        let heights: Vec<_> = (0..1_000)
            .map(|index| {
                estimated_line_height(if index == 40 { &long } else { &short }, 300.0, 18.0, true)
            })
            .collect();
        let offset = estimated_scroll_offset(&heights, 500);
        let (range, top, bottom) = virtual_window(&heights, offset, 600.0, 8);
        assert!(range.contains(&500));
        assert!(range.len() < 40);
        assert!(top > 0.0 && bottom > 0.0);
    }
    #[test]
    fn ttml_preserves_word_timing_and_spaces() {
        let l=ttml(r#"<tt><body><p begin="1s" end="4s"><span begin="1s" end="2s">Hello</span> <span begin="2s" end="3s">world</span></p></body></tt>"#).unwrap();
        assert_eq!(l.lines[0].text, "Hello world");
        assert_eq!(l.lines[0].words[0].text, "Hello ");
        assert_eq!(l.lines[0].words[1].start, 2.0);
        assert_eq!(l.active(4.5), None);
    }
    #[test]
    fn ttml_accepts_namespaced_provider_documents() {
        let lyrics = ttml(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><p begin="1s" end="3s"><span begin="1s" end="2s">Hello</span> <span begin="2s" end="3s">world</span></p></body></tt>"#,
        )
        .unwrap();
        assert_eq!(lyrics.lines[0].text, "Hello world");
        assert_eq!(lyrics.lines[0].words.len(), 2);
    }

    fn fixture_track(title: &str, artist: &str) -> Track {
        Track {
            duration_secs: None,
            video_id: "vid".into(),
            title: title.into(),
            artist: artist.into(),
            album: None,
            thumbnail_url: None,
        }
    }

    #[test]
    fn normalized_identity_avoids_duration_debug_duplication() {
        let t = fixture_track("  Hello   World ", "Artist");
        let a = normalized_identity(&t, Some(Duration::from_secs(183)));
        let b = normalized_identity(&t, Some(Duration::from_millis(183_400)));
        assert_eq!(a, b);
        let c = normalized_identity(&t, Some(Duration::from_secs(184)));
        assert_ne!(a, c);
    }

    #[test]
    fn version_markers_match_whole_words_only() {
        assert!(!titles_match("Alive", "Live"), "live must not match alive");
        assert!(version_markers("Alive").is_empty());
        assert_eq!(version_markers("Song (Live)"), vec!["live".to_owned()]);
        assert_eq!(
            version_markers("Sped Up Version"),
            vec!["sped up".to_owned()]
        );
    }

    #[test]
    fn artist_credits_match_on_any_shared_artist_with_unicode_case() {
        assert!(artists_match("Daft Punk, Julian Casablancas", "Daft Punk"));
        assert!(artists_match("Artist", "Artist feat. Someone"));
        assert!(artists_match("Björk & Friends", "BJÖRK"));
        assert!(!artists_match("Abba", "Bjorn"));
    }

    #[test]
    fn remastered_and_from_suffixes_do_not_block_a_title_match() {
        assert!(titles_match("Song - Remastered 2011", "Song"));
        assert!(titles_match("Song - From \"Movie\"", "Song"));
        assert!(!titles_match("Song - Live", "Song"));
    }

    #[test]
    fn version_markers_prevent_live_remix_cover_confusion() {
        assert!(titles_match("Song", "Song"));
        assert!(titles_match("Song (Live)", "Song (Live)"));
        assert!(!titles_match("Song", "Song (Live)"));
        assert!(!titles_match("Song (Remix)", "Song"));
        assert!(!titles_match("Song (Cover)", "Song (Acoustic)"));
        assert!(titles_match("Song feat. Someone", "Song feat. Someone"));
    }

    #[test]
    fn instrumental_is_detected_without_invented_timings() {
        assert!(Lyrics::is_instrumental_text("Instrumental"));
        assert!(Lyrics::is_instrumental_text("[Instrumental]"));
        let l = lrc("Instrumental", "fixture");
        assert!(l.lines.is_empty());
    }

    #[test]
    fn unicode_long_translation_and_active_borders() {
        let l = lrc(
            "[00:01.00]こんにちは世界\n[00:05.00]مرحبا بالعالم\n[00:09.00]Hello",
            "fixture",
        );
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[0].text, "こんにちは世界");
        // End-exclusive: exactly at next start belongs to next line.
        assert_eq!(l.active(4.999), Some(0));
        assert_eq!(l.active(5.0), Some(1));
        let long = "a".repeat(500);
        let l = lrc(&format!("[00:01.00]{long}"), "fixture");
        assert_eq!(l.lines[0].text.len(), 500);
        let t = ttml(r#"<tt><body><p begin="1s" end="3s">Hello<span role="translation">Hola</span></p></body></tt>"#).unwrap();
        assert_eq!(t.lines[0].translation.as_deref(), Some("Hola"));
    }

    #[test]
    fn ttml_keeps_romanization_and_background_vocals_apart_from_the_line() {
        let input = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div><p begin="1.0" end="3.0"><span begin="1.0" end="2.0">Hola</span> <span ttm:role="x-bg"><span begin="2.0" end="2.5">(eh)</span></span><span ttm:role="x-roman">ola</span></p></div></body></tt>"#;
        let lyrics = super::ttml(input).expect("valid TTML");
        let line = &lyrics.lines[0];
        assert_eq!(line.text, "Hola");
        assert_eq!(line.background.as_deref(), Some("(eh)"));
        assert_eq!(line.romanization.as_deref(), Some("ola"));
        assert_eq!(
            line.words.len(),
            1,
            "background words are not karaoke words"
        );
    }

    fn timed_line(start: f64, text: &str) -> Line {
        Line {
            start,
            end: Some(start + 2.0),
            text: text.into(),
            words: Vec::new(),
            translation: None,
            romanization: None,
            background: None,
        }
    }

    #[test]
    fn leading_and_trailing_credits_go_and_the_sung_lines_keep_their_timing() {
        let lyrics = Lyrics {
            source: "QQ Music via Better Lyrics".into(),
            lines: vec![
                timed_line(0.0, "Paper Lanterns (Explicit) - The Quiet Harbors"),
                timed_line(1.1, "Lyrics by：Ana Ruiz/Leo Park/Mia Chen"),
                timed_line(2.2, "Composed by：Mia Chen/Ana Ruiz"),
                timed_line(19.3, "We left the porch light burning"),
                timed_line(23.0, "The river knows the way back home"),
                timed_line(230.0, "Written by: Someone Else"),
            ],
            plain: None,
        };
        let cleaned = drop_credit_lines(lyrics, "Paper Lanterns", "The Quiet Harbors");
        assert_eq!(cleaned.lines.len(), 2);
        assert_eq!(cleaned.lines[0].text, "We left the porch light burning");
        assert_eq!(cleaned.lines[0].start, 19.3);
        assert_eq!(cleaned.lines[0].end, Some(21.3));
        assert_eq!(
            cleaned.active(5.0),
            None,
            "no line is active during the credits"
        );
        assert_eq!(cleaned.active(19.5), Some(0));
    }

    #[test]
    fn credits_in_the_middle_of_a_song_stay() {
        let lyrics = Lyrics {
            source: "QQ".into(),
            lines: vec![
                timed_line(19.3, "We left the porch light burning"),
                timed_line(40.0, "Lyrics by: a bridge credit"),
                timed_line(60.0, "The river knows the way"),
            ],
            plain: None,
        };
        let cleaned = drop_credit_lines(lyrics, "Paper Lanterns", "The Quiet Harbors");
        assert_eq!(cleaned.lines.len(), 3);
    }

    #[test]
    fn credit_labels_in_chinese_and_with_either_colon_are_recognised() {
        for text in [
            "作词：张三",
            "作曲: 张三",
            "编曲：某人",
            "制作人: 某人",
            "词：某人",
            "曲 : 某人",
            "Written by: X",
        ] {
            assert!(is_credit_line(text, "Song", "Artist"), "{text}");
        }
        for text in [
            "歌词里的一句",
            "Lyrics about you",
            "the composer of my heart",
        ] {
            assert!(!is_credit_line(text, "Song", "Artist"), "{text}");
        }
    }

    #[test]
    fn title_and_artist_lines_need_this_tracks_artist() {
        assert!(is_credit_line(
            "Paper Lanterns (Explicit) - The Quiet Harbors",
            "Paper Lanterns",
            "The Quiet Harbors"
        ));
        assert!(!is_credit_line(
            "Paper Lanterns - Someone Else",
            "Paper Lanterns",
            "The Quiet Harbors"
        ));
        assert!(!is_credit_line(
            "Paper Lanterns - The Quiet Harbors",
            "Paper Lanterns",
            ""
        ));
    }

    #[test]
    fn plain_text_loses_credit_rows_at_both_ends_only() {
        let lyrics = Lyrics {
            source: "Plain".into(),
            lines: Vec::new(),
            plain: Some(
                "Lyrics by: X\nfirst line\nLyrics by: a middle credit\nsecond line\nComposed by: Y"
                    .into(),
            ),
        };
        let cleaned = drop_credit_lines(lyrics, "Song", "Artist");
        assert_eq!(
            cleaned.plain.as_deref(),
            Some("first line\nLyrics by: a middle credit\nsecond line")
        );
    }

    /// Calls the lyric providers. Run with `cargo test -- --ignored credits_are_gone_live`.
    #[tokio::test]
    #[ignore = "calls the lyric providers over the network"]
    async fn credits_are_gone_live() {
        let track = crate::music::Track {
            video_id: "dQw4w9WgXcQ".into(),
            title: "Never Gonna Give You Up".into(),
            artist: "Rick Astley".into(),
            album: None,
            thumbnail_url: None,
            duration_secs: Some(213),
        };
        let lyrics = load(track, Some(Duration::from_secs(213)))
            .await
            .expect("the providers answered");
        let first = lyrics
            .lines
            .first()
            .map(|line| line.text.clone())
            .or_else(|| lyrics.plain.clone())
            .unwrap_or_default();
        println!("first line: {first:?} from {}", lyrics.source);
        assert!(!first.contains("Lyrics by"), "{first}");
        assert!(!first.contains(" - Rick Astley"), "{first}");
    }
}
