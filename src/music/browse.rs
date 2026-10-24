//! Catalog destinations and collection parsing. Independent of playback internals.
use serde_json::Value;
use std::time::Duration;

use super::{MusicError, Track, home_item_thumbnail, text_value};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BrowseTarget {
    pub browse_id: String,
    /// Opaque service parameters: preserve verbatim, never reconstruct from titles.
    pub params: Option<String>,
    pub page_type: Option<String>,
}

/// A podcast's page, which lists its episodes.
const PODCAST_PAGE: &str = "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE";
/// An episode's own page. The app plays the episode instead of opening it.
pub(super) const EPISODE_PAGE: &str = "MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE";

impl BrowseTarget {
    pub fn is_public_collection(&self) -> bool {
        match self.page_type.as_deref() {
            Some(
                "MUSIC_PAGE_TYPE_PLAYLIST"
                | "MUSIC_PAGE_TYPE_ALBUM"
                | "MUSIC_PAGE_TYPE_ARTIST"
                | "MUSIC_PAGE_TYPE_USER_CHANNEL"
                | PODCAST_PAGE,
            ) => true,
            Some("MUSIC_PAGE_TYPE_MIXED_CONTENT") | None => {
                self.browse_id.starts_with("VL") || self.browse_id.starts_with("RD")
            }
            _ => false,
        }
    }

    /// A podcast, whose page lists its episodes.
    pub fn is_podcast(&self) -> bool {
        self.page_type.as_deref() == Some(PODCAST_PAGE)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogReference {
    pub label: String,
    pub target: BrowseTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionTrack {
    /// The same `Track` value the playback engine receives.
    pub track: Track,
    pub duration: Option<Duration>,
    pub references: Vec<CatalogReference>,
    /// When an episode came out, as the catalog words it ("2 days ago", "Oct 16").
    pub published: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Continuation {
    pub token: String,
    /// The podcast whose further episodes the token loads: they name it as their artist.
    pub podcast: Option<Box<(String, BrowseTarget)>>,
}

const MAX_CONTINUATION_TOKEN_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug)]
pub struct BrowseMore {
    pub tracks: Vec<CollectionTrack>,
    pub continuation: Option<Continuation>,
}

impl std::fmt::Debug for Continuation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Continuation(<opaque>)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub thumbnail_url: Option<String>,
    /// Service-provided count/duration text; do not infer a complete total from one batch.
    pub summary: Option<String>,
    pub tracks: Vec<CollectionTrack>,
    pub continuation: Option<Continuation>,
    pub unavailable_tracks: usize,
    pub related: Vec<super::HomeShelf>,
    /// An artist's page shows a few of their songs; this page has all of them.
    pub all_songs: Option<Box<BrowseTarget>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowsePage {
    Collection(Box<Collection>),
    Unsupported { page_type: Option<String> },
}

pub(super) fn parse_browse_endpoint(endpoint: &Value) -> Option<BrowseTarget> {
    let browse = endpoint.get("browseEndpoint")?;
    Some(BrowseTarget {
        browse_id: browse.get("browseId")?.as_str()?.to_owned(),
        params: browse
            .get("params")
            .and_then(Value::as_str)
            .map(str::to_owned),
        page_type: browse
            .pointer(
                "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
            )
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

pub(super) fn item_destination(item: &Value) -> Option<BrowseTarget> {
    item.get("navigationEndpoint")
        .and_then(parse_browse_endpoint)
        .or_else(|| {
            item.pointer("/title/runs")?
                .as_array()?
                .iter()
                .find_map(|run| parse_browse_endpoint(run.get("navigationEndpoint")?))
        })
        .or_else(|| {
            item.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs")?
                .as_array()?
                .iter()
                .find_map(|run| parse_browse_endpoint(run.get("navigationEndpoint")?))
        })
}

fn text_runs(item: &Value) -> Vec<&Value> {
    let mut runs = Vec::new();
    for key in ["title", "subtitle"] {
        if let Some(values) = item
            .pointer(&format!("/{key}/runs"))
            .and_then(Value::as_array)
        {
            runs.extend(values);
        }
    }
    if let Some(columns) = item.get("flexColumns").and_then(Value::as_array) {
        for column in columns {
            if let Some(values) = column
                .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                .and_then(Value::as_array)
            {
                runs.extend(values);
            }
        }
    }
    runs
}

/// Item-type labels YouTube Music puts first in a byline, in the UI languages it serves.
const BYLINE_TYPE_LABELS: &[&str] = &[
    "song",
    "video",
    "album",
    "single",
    "ep",
    "playlist",
    "episode",
    "podcast",
    "canción",
    "vídeo",
    "álbum",
    "sencillo",
    "lista de reproducción",
    "episodio",
    "pódcast",
    "música",
    "episódio",
    "titre",
    "vidéo",
    "épisode",
    "titel",
    "folge",
    "brano",
    "singolo",
    "puntata",
    "композиция",
    "видео",
    "альбом",
    "сингл",
    "плейлист",
    "выпуск",
    "подкаст",
    "曲",
    "動画",
    "アルバム",
    "シングル",
    "プレイリスト",
    "エピソード",
    "ポッドキャスト",
    "歌曲",
    "视频",
    "专辑",
    "单曲",
    "播放列表",
    "分集",
    "播客",
];

/// The artist part of an unlinked byline such as `Song • Artist • 3:45`:
/// drops the item-type label, durations, years and play counts.
pub(super) fn clean_byline(text: &str) -> String {
    text.split('•')
        .map(str::trim)
        .enumerate()
        .filter(|(index, part)| {
            let lower = part.to_lowercase();
            let is_label = *index == 0 && BYLINE_TYPE_LABELS.contains(&lower.as_str());
            let is_clock =
                part.contains(':') && part.chars().all(|c| c.is_ascii_digit() || c == ':');
            let is_year = part.len() == 4 && part.chars().all(|c| c.is_ascii_digit());
            !part.is_empty() && !is_label && !is_clock && !is_year
        })
        .map(|(_, part)| part)
        .next()
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn parse_catalog_track(item: &Value) -> Option<CollectionTrack> {
    // Greyed-out/unplayable rows must never become clickable tracks.
    if item.get("isPlayable").and_then(Value::as_bool) == Some(false)
        || item
            .get("musicItemRendererDisplayPolicy")
            .and_then(Value::as_str)
            == Some("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT")
    {
        return None;
    }
    let runs = text_runs(item);
    let watch_video = |endpoint: &Value| {
        endpoint
            .pointer("/watchEndpoint/videoId")
            .or_else(|| endpoint.pointer("/videoEndpoint/videoId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    // Only the item's own playback endpoints, never an arbitrary menu/radio action.
    let video_id = item.pointer("/playlistItemData/videoId").and_then(Value::as_str).map(str::to_owned)
        .or_else(|| item.get("navigationEndpoint").and_then(watch_video))
        .or_else(|| runs.iter().find_map(|run| run.get("navigationEndpoint").and_then(watch_video)))
        .or_else(|| item.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint").and_then(watch_video))?;
    if video_id.len() > 256 {
        return None;
    }
    let title = item.get("title").and_then(text_value).or_else(|| {
        item.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")
            .and_then(text_value)
    })?;
    if video_id.is_empty() || title.trim().is_empty() {
        return None;
    }
    let title = super::bound_catalog_text(&title, 512);
    let references: Vec<_> = runs
        .iter()
        .take(16)
        .filter_map(|run| {
            Some(CatalogReference {
                label: super::bound_catalog_text(run.get("text")?.as_str()?, 256),
                target: parse_browse_endpoint(run.get("navigationEndpoint")?)?,
            })
        })
        .collect();
    let artists: Vec<_> = references
        .iter()
        .filter(|reference| is_artist_destination(reference))
        .map(|reference| reference.label.as_str())
        .collect();
    // Album is taken only from a linked album/release destination. This keeps
    // lyrics matching metadata structural and avoids guessing from display text.
    let album = references
        .iter()
        .find(|reference| is_album_destination(reference))
        .map(|reference| reference.label.clone());
    // An episode links its podcast where a song links its artist.
    let artist_page = references
        .iter()
        .find(|reference| is_artist_destination(reference))
        .or_else(|| {
            references
                .iter()
                .find(|reference| reference.target.is_podcast())
        })
        .map(|reference| reference.target.clone());
    let album_page = references
        .iter()
        .find(|reference| is_album_destination(reference))
        .map(|reference| reference.target.clone());
    let artist = if !artists.is_empty() {
        artists.join(", ")
    } else {
        // Some responses omit pageType even on the artist link.
        item.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs")
            .and_then(Value::as_array)
            .and_then(|runs| {
                runs.iter()
                    .find(|run| run.pointer("/navigationEndpoint/browseEndpoint").is_some())
            })
            .and_then(|run| run.get("text"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                item.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
                    .and_then(text_value)
                    .map(|text| clean_byline(&text))
            })
            .or_else(|| {
                item.get("subtitle")
                    .and_then(text_value)
                    .map(|text| clean_byline(&text))
            })
            .unwrap_or_default()
    };
    let artist = super::bound_catalog_text(&artist, 1_024);
    let album = album.map(|album| super::bound_catalog_text(&album, 512));
    // Search rows put the length at the end of the second column
    // (`Song • Artist • 3:45`) instead of a fixed column.
    let duration = item
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
        .and_then(text_value)
        .as_deref()
        .and_then(parse_duration)
        .or_else(|| {
            let byline = item
                .pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
                .and_then(text_value)?;
            parse_duration(byline.rsplit('•').next()?.trim())
        })
        // A podcast's page gives an episode's length in words ("2 hr 43 min").
        .or_else(|| {
            item.pointer("/playbackProgress/musicPlaybackProgressRenderer/durationText")
                .and_then(text_value)
                .as_deref()
                .and_then(parse_spoken_duration)
        });
    Some(CollectionTrack {
        track: Track {
            video_id,
            title,
            artist,
            album,
            artist_page,
            album_page,
            thumbnail_url: home_item_thumbnail(item).filter(|url| url.len() <= 2_048),
            duration_secs: duration.and_then(|length| u32::try_from(length.as_secs()).ok()),
        },
        duration,
        references,
        published: None,
    })
}

/// An episode row of a podcast's page. Its byline reads "682K views • 20h ago": the episode's
/// artist is the podcast, which links to the podcast's page, and the date shows in its place.
fn parse_episode(item: &Value, podcast: &str, target: &BrowseTarget) -> Option<CollectionTrack> {
    let mut entry = parse_catalog_track(item)?;
    entry.track.artist = super::bound_catalog_text(podcast, 1_024);
    entry.track.artist_page = Some(target.clone());
    entry.published = item
        .get("subtitle")
        .and_then(text_value)
        .and_then(|text| {
            text.rsplit('•')
                .next()
                .map(|date| super::bound_catalog_text(date.trim(), 64))
        })
        .filter(|date| !date.is_empty());
    Some(entry)
}

/// The rows of a collection's shelf that become tracks: the songs of a playlist or an album, or the
/// episodes of a podcast, which name the podcast as their artist.
fn shelf_tracks(
    rows: &[Value],
    podcast: Option<(&str, &BrowseTarget)>,
) -> (Vec<CollectionTrack>, usize) {
    let mut tracks = Vec::new();
    let mut unavailable = 0;
    for row in rows.iter().take(super::MAX_COLLECTION_PAGE_TRACKS) {
        let parsed = if let Some(item) = row.get("musicResponsiveListItemRenderer") {
            parse_catalog_track(item)
        } else if let Some(item) = row.get("musicMultiRowListItemRenderer") {
            match podcast {
                Some((title, target)) => parse_episode(item, title, target),
                None => parse_catalog_track(item),
            }
        } else {
            continue;
        };
        match parsed {
            Some(track) => tracks.push(track),
            None => unavailable += 1,
        }
    }
    (tracks, unavailable)
}

/// An artist destination: the page type says so, or, when the response omits it, a channel id.
fn is_artist_destination(reference: &CatalogReference) -> bool {
    matches!(
        reference.target.page_type.as_deref(),
        Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL")
    ) || (reference.target.page_type.is_none() && reference.target.browse_id.starts_with("UC"))
}

/// A linked album or release destination.
/// An album destination: the page type says so, or, when the response omits it, an `MP` id. Episodes
/// and podcasts have `MP` ids too, but always with their page type.
fn is_album_destination(reference: &CatalogReference) -> bool {
    match reference.target.page_type.as_deref() {
        Some(page_type) => page_type == "MUSIC_PAGE_TYPE_ALBUM",
        None => reference.target.browse_id.starts_with("MP"),
    }
}

pub(super) fn parse_duration(text: &str) -> Option<Duration> {
    let parts: Vec<_> = text.trim().split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut seconds = 0_u64;
    for (index, part) in parts.iter().enumerate() {
        let value = part.parse::<u64>().ok()?;
        if index > 0 && value >= 60 {
            return None;
        }
        seconds = seconds.checked_mul(60)?.checked_add(value)?;
    }
    Some(Duration::from_secs(seconds))
}

/// A length in words, as podcasts give it: "2 hr 43 min", "2 Std., 43 Min.", "2時間 43 分",
/// "2小时43分钟". Each number is read with the unit that follows it.
pub(super) fn parse_spoken_duration(text: &str) -> Option<Duration> {
    let mut seconds = 0_u64;
    let mut found = false;
    let mut rest = text.trim_start_matches(|c: char| !c.is_ascii_digit());
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let value: u64 = rest[..digits].parse().ok()?;
        let after = rest[digits..].trim_start();
        let unit: String = after
            .chars()
            .take_while(|c| !c.is_ascii_digit() && !c.is_whitespace() && *c != ',')
            .collect::<String>()
            .to_lowercase();
        let scale = if unit.starts_with("min") || unit.starts_with("мин") || unit.starts_with('分')
        {
            60
        } else if unit.starts_with('h')
            || unit.starts_with("std")
            || unit.starts_with('ч')
            || unit.starts_with('時')
            || unit.starts_with('小')
        {
            3_600
        } else if unit.starts_with('s') || unit.starts_with("сек") || unit.starts_with('秒') {
            1
        } else {
            return None;
        };
        seconds = seconds.checked_add(value.checked_mul(scale)?)?;
        found = true;
        rest = after.trim_start_matches(|c: char| !c.is_ascii_digit());
    }
    found.then(|| Duration::from_secs(seconds))
}

pub(super) fn find_renderer<'a>(value: &'a Value, names: &[&str]) -> Option<&'a Value> {
    match value {
        Value::Object(object) => names.iter().find_map(|name| object.get(*name)).or_else(|| {
            object
                .values()
                .find_map(|value| find_renderer(value, names))
        }),
        Value::Array(values) => values.iter().find_map(|value| find_renderer(value, names)),
        _ => None,
    }
}

fn parse_continuation(shelf: &Value) -> Option<Continuation> {
    let token = shelf
        .pointer("/continuations/0/nextContinuationData/continuation")
        .or_else(|| shelf.pointer("/continuations/0/reloadContinuationData/continuation"))
        .and_then(Value::as_str)
        .or_else(|| {
            let item = shelf
                .get("contents")?
                .as_array()?
                .iter()
                .find_map(|item| item.get("continuationItemRenderer"))?;
            item.pointer("/continuationEndpoint/continuationCommand/token")
                .and_then(Value::as_str)
        })?;
    continuation_from_token(token)
}

fn continuation_from_token(token: &str) -> Option<Continuation> {
    (!token.is_empty() && token.len() <= MAX_CONTINUATION_TOKEN_BYTES).then(|| Continuation {
        token: token.to_owned(),
        podcast: None,
    })
}

pub(super) fn parse_search_continuation(payload: &Value) -> Option<Continuation> {
    find_renderer(payload, &["musicShelfRenderer"]).and_then(parse_continuation)
}

pub(super) fn parse_search_more(
    payload: &Value,
) -> Option<(Vec<super::HomeItem>, Option<Continuation>)> {
    let shelf = find_renderer(payload, &["musicShelfContinuation", "musicShelfRenderer"]);
    let appended = find_renderer(
        payload,
        &[
            "appendContinuationItemsAction",
            "reloadContinuationItemsCommand",
        ],
    );
    let rows = shelf
        .and_then(|shelf| shelf.get("contents").and_then(Value::as_array))
        .or_else(|| {
            appended.and_then(|action| action.get("continuationItems").and_then(Value::as_array))
        })?;
    let items = rows
        .iter()
        .take(super::MAX_CATALOG_ITEMS)
        .filter_map(super::parse_home_item)
        .collect();
    let continuation = shelf.and_then(parse_continuation).or_else(|| {
        rows.iter().find_map(|row| {
            row.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
                .and_then(Value::as_str)
                .and_then(continuation_from_token)
        })
    });
    Some((items, continuation))
}

pub(super) fn parse_browse_more(
    payload: &Value,
    podcast: Option<&(String, BrowseTarget)>,
) -> Result<BrowseMore, MusicError> {
    let shelf = find_renderer(
        payload,
        &[
            "musicPlaylistShelfContinuation",
            "musicShelfContinuation",
            "musicPlaylistShelfRenderer",
            "musicShelfRenderer",
        ],
    );
    let appended = find_renderer(
        payload,
        &[
            "appendContinuationItemsAction",
            "reloadContinuationItemsCommand",
        ],
    );
    let rows = shelf
        .and_then(|shelf| shelf.get("contents").and_then(Value::as_array))
        .or_else(|| {
            appended.and_then(|action| action.get("continuationItems").and_then(Value::as_array))
        })
        .ok_or_else(|| MusicError::InvalidResponse("playlist continuation rows missing".into()))?;
    let (tracks, _) = shelf_tracks(
        rows,
        podcast.map(|(title, target)| (title.as_str(), target)),
    );
    let continuation = shelf
        .and_then(parse_continuation)
        .or_else(|| {
            rows.iter().find_map(|row| {
                row.pointer(
                    "/continuationItemRenderer/continuationEndpoint/continuationCommand/token",
                )
                .and_then(Value::as_str)
                .and_then(continuation_from_token)
            })
        })
        .map(|continuation| Continuation {
            podcast: podcast.cloned().map(Box::new),
            ..continuation
        });
    Ok(BrowseMore {
        tracks,
        continuation,
    })
}

pub(super) fn parse_browse_page(
    payload: &Value,
    target: &BrowseTarget,
) -> Result<BrowsePage, MusicError> {
    if !target.is_public_collection() {
        return Ok(BrowsePage::Unsupported {
            page_type: target.page_type.clone(),
        });
    }
    let malformed =
        || MusicError::InvalidResponse("collection header or track shelf missing".into());
    let header = payload
        .get("header")
        .and_then(|value| {
            find_renderer(
                value,
                &[
                    "musicDetailHeaderRenderer",
                    "musicResponsiveHeaderRenderer",
                    "musicImmersiveHeaderRenderer",
                    "musicVisualHeaderRenderer",
                ],
            )
        })
        .or_else(|| {
            payload.get("contents").and_then(|value| {
                find_renderer(
                    value,
                    &[
                        "musicResponsiveHeaderRenderer",
                        "musicDetailHeaderRenderer",
                        "musicImmersiveHeaderRenderer",
                        "musicVisualHeaderRenderer",
                    ],
                )
            })
        })
        .ok_or_else(malformed)?;
    let title = header
        .get("title")
        .and_then(text_value)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(malformed)?;
    let contents = payload.get("contents").ok_or_else(malformed)?;
    // Playlist shelves contain the collection's ordered tracks, not recommendation carousels.
    let names: &[&str] = if target.page_type.as_deref() == Some("MUSIC_PAGE_TYPE_ALBUM") {
        &["musicShelfRenderer"]
    } else {
        &["musicPlaylistShelfRenderer", "musicShelfRenderer"]
    };
    let shelf = find_renderer(contents, names).ok_or_else(malformed)?;
    let rows = shelf
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let podcast = target.is_podcast().then_some((title.as_str(), target));
    let (tracks, unavailable_tracks) = shelf_tracks(rows, podcast);
    let owner = header
        .pointer("/facepile/avatarStackViewModel/text/content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| header.get("straplineTextOne").and_then(text_value))
        .or_else(|| {
            header
                .pointer("/subtitle/runs")
                .and_then(Value::as_array)
                .and_then(|runs| {
                    runs.iter().find_map(|run| {
                        let target = parse_browse_endpoint(run.get("navigationEndpoint")?)?;
                        if matches!(
                            target.page_type.as_deref(),
                            Some("MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_ARTIST")
                        ) || target.browse_id.starts_with("UC")
                        {
                            run.get("text")?.as_str().map(str::to_owned)
                        } else {
                            None
                        }
                    })
                })
        });
    let artist = target.page_type.as_deref() == Some("MUSIC_PAGE_TYPE_ARTIST");
    let continuation = if tracks.len() >= super::MAX_COLLECTION_TRACKS {
        None
    } else {
        parse_continuation(shelf).map(|continuation| Continuation {
            podcast: podcast.map(|(title, target)| Box::new((title.to_owned(), target.clone()))),
            ..continuation
        })
    };
    Ok(BrowsePage::Collection(Box::new(Collection {
        title,
        subtitle: header.get("subtitle").and_then(text_value),
        description: header
            .pointer("/description/musicDescriptionShelfRenderer/description")
            .and_then(text_value)
            .or_else(|| header.get("description").and_then(text_value)),
        owner,
        thumbnail_url: home_item_thumbnail(header),
        summary: header.get("secondSubtitle").and_then(text_value),
        tracks,
        continuation,
        unavailable_tracks,
        related: if artist {
            super::parse_home_feed(payload).unwrap_or_default()
        } else {
            Vec::new()
        },
        all_songs: artist
            .then(|| {
                ["/bottomEndpoint", "/title/runs/0/navigationEndpoint"]
                    .into_iter()
                    .find_map(|pointer| parse_browse_endpoint(shelf.pointer(pointer)?))
            })
            .flatten()
            .filter(BrowseTarget::is_public_collection)
            .map(Box::new),
    })))
}

#[cfg(test)]
mod byline_tests {
    use super::clean_byline;

    #[test]
    fn unlinked_bylines_keep_only_the_artist() {
        assert_eq!(clean_byline("Canción • 6:10"), "");
        assert_eq!(clean_byline("Song • Daft Punk • 6:10"), "Daft Punk");
        // The labels YouTube Music writes in the other interface languages.
        for label in [
            "Música",
            "Titre",
            "Titel",
            "Brano",
            "Композиция",
            "曲",
            "歌曲",
        ] {
            assert_eq!(
                clean_byline(&format!("{label} • Daft Punk • 6:10")),
                "Daft Punk",
                "{label}"
            );
        }
        assert_eq!(
            clean_byline("Daft Punk • Random Access Memories • 2013"),
            "Daft Punk"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn podcast() -> BrowseTarget {
        BrowseTarget {
            browse_id: "MPSPPLfixture".into(),
            params: None,
            page_type: Some(PODCAST_PAGE.into()),
        }
    }

    /// An episode row of a podcast's page, as YouTube Music sends it.
    fn episode_row(id: &str, title: &str, date: &str, length: &str) -> Value {
        json!({"musicMultiRowListItemRenderer": {
            "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [
                {"url": format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"), "width": 400, "height": 225}
            ]}}},
            "overlay": {"musicItemThumbnailOverlayRenderer": {"content": {"musicPlayButtonRenderer": {
                "playNavigationEndpoint": {"watchEndpoint": {"videoId": id}}
            }}}},
            "onTap": {"watchEndpoint": {"videoId": id}},
            "title": {"runs": [{"text": title, "navigationEndpoint": {"browseEndpoint": {
                "browseId": format!("MPED{id}"),
                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": EPISODE_PAGE}}
            }}}]},
            "subtitle": {"runs": [{"text": "1.2K views"}, {"text": " • "}, {"text": date}]},
            "playbackProgress": {"musicPlaybackProgressRenderer": {
                "durationText": {"runs": [{"text": " • "}, {"text": length}]}
            }},
            "description": {"runs": [{"text": "What the episode is about."}]}
        }})
    }

    #[test]
    fn a_podcast_page_lists_its_episodes_with_their_dates_and_lengths() {
        let payload = json!({"contents": {"twoColumnBrowseResultsRenderer": {
            "secondaryContents": {"sectionListRenderer": {"contents": [{"musicShelfRenderer": {
                "contents": [
                    episode_row("episode-two", "Episode two", "20h ago", "2 hr 43 min"),
                    episode_row("episode-one", "Episode one", "Oct 16", "58 min")
                ],
                "continuations": [{"nextContinuationData": {"continuation": "more-episodes"}}]
            }}]}},
            "tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [{
                "musicResponsiveHeaderRenderer": {
                    "title": {"runs": [{"text": "A fixture podcast"}]},
                    "straplineTextOne": {"runs": [{"text": "Its host"}]}
                }
            }]}}}}]
        }}});
        let BrowsePage::Collection(data) = parse_browse_page(&payload, &podcast()).unwrap() else {
            panic!("a podcast's page is a collection");
        };
        assert_eq!(data.title, "A fixture podcast");
        assert_eq!(data.owner.as_deref(), Some("Its host"));
        assert_eq!(data.tracks.len(), 2);
        let first = &data.tracks[0];
        assert_eq!(first.track.video_id, "episode-two");
        assert_eq!(first.track.title, "Episode two");
        // An episode names its podcast as its artist, and links to it.
        assert_eq!(first.track.artist, "A fixture podcast");
        assert_eq!(first.track.artist_page, Some(podcast()));
        assert_eq!(first.published.as_deref(), Some("20h ago"));
        assert_eq!(
            first.duration,
            Some(Duration::from_secs(2 * 3_600 + 43 * 60))
        );
        assert_eq!(data.tracks[1].duration, Some(Duration::from_secs(58 * 60)));
        // Further episodes keep the podcast they belong to.
        let continuation = data.continuation.unwrap();
        assert_eq!(continuation.token, "more-episodes");
        assert_eq!(
            continuation.podcast.as_deref(),
            Some(&("A fixture podcast".to_owned(), podcast()))
        );
        let more = json!({"continuationContents": {"musicShelfContinuation": {
            "contents": [episode_row("episode-zero", "Episode zero", "Sep 9", "1 hr")]
        }}});
        let more = parse_browse_more(&more, continuation.podcast.as_deref()).unwrap();
        assert_eq!(more.tracks.len(), 1);
        assert_eq!(more.tracks[0].track.artist, "A fixture podcast");
        assert_eq!(more.tracks[0].published.as_deref(), Some("Sep 9"));
        assert!(more.continuation.is_none());
    }

    #[test]
    fn episode_lengths_are_read_in_every_catalog_language() {
        let two_43 = Some(Duration::from_secs(2 * 3_600 + 43 * 60));
        for text in [
            " • 2 hr 43 min",
            "2 h 43 min",
            "2 Std., 43 Min.",
            "2 ч 43 мин",
            "2 時間 43 分",
            "2小时43分钟",
        ] {
            assert_eq!(parse_spoken_duration(text), two_43, "{text}");
        }
        assert_eq!(
            parse_spoken_duration("58 min 12 sec"),
            Some(Duration::from_secs(58 * 60 + 12))
        );
        assert_eq!(parse_spoken_duration("Played"), None);
        assert_eq!(parse_spoken_duration("3 days"), None);
    }

    fn target() -> BrowseTarget {
        BrowseTarget {
            browse_id: "VLRDfixture".into(),
            params: Some("fixture-params".into()),
            page_type: Some("MUSIC_PAGE_TYPE_PLAYLIST".into()),
        }
    }
    fn responsive() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/collection_responsive.json"
        ))
        .unwrap()
    }
    fn collection(payload: &Value) -> Collection {
        match parse_browse_page(payload, &target()).unwrap() {
            BrowsePage::Collection(data) => *data,
            _ => panic!("expected collection"),
        }
    }
    #[test]
    fn parses_responsive_mix_header_tracks_duration_owner_and_continuation() {
        let data = collection(&responsive());
        assert_eq!(data.title, "Fixture collection");
        assert_eq!(data.owner.as_deref(), Some("YouTube\u{a0}Music"));
        assert!(data.description.is_some());
        assert!(data.thumbnail_url.is_some());
        assert!(data.summary.as_ref().unwrap().contains("80"));
        assert_eq!(data.tracks.len(), 2);
        assert_eq!(data.tracks[0].track.video_id, "fixture-first");
        assert_eq!(data.tracks[1].track.video_id, "fixture-second");
        assert_eq!(data.tracks[0].track.artist, "Marc Anthony");
        assert_eq!(data.tracks[0].duration, Some(Duration::from_secs(327)));
        assert_eq!(
            data.tracks[0].references[0].target.browse_id,
            "UCfixture-artist"
        );
        assert_eq!(data.continuation.unwrap().token, "fixture-continuation");
    }
    #[test]
    fn parses_legacy_playlist_and_linked_owner() {
        let payload =
            serde_json::from_str(include_str!("../../tests/fixtures/collection_legacy.json"))
                .unwrap();
        let data = collection(&payload);
        assert_eq!(data.title, "Fixture legacy playlist");
        assert_eq!(data.owner.as_deref(), Some("Fixture owner"));
        assert_eq!(data.tracks.len(), 2);
        assert!(data.continuation.is_none());
    }
    #[test]
    fn oversized_continuation_token_is_rejected() {
        assert!(continuation_from_token(&"x".repeat(MAX_CONTINUATION_TOKEN_BYTES + 1)).is_none());
        assert!(continuation_from_token("").is_none());
    }
    #[test]
    fn empty_is_distinct_from_malformed_and_unavailable_rows_are_skipped() {
        let mut payload = responsive();
        let shelf = payload.pointer_mut("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer").unwrap();
        shelf["contents"][0]["musicResponsiveListItemRenderer"]["musicItemRendererDisplayPolicy"] =
            json!("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT");
        let data = collection(&payload);
        assert_eq!(data.tracks.len(), 1);
        assert_eq!(data.unavailable_tracks, 1);
        payload.pointer_mut("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer").unwrap()["contents"] = json!([]);
        assert!(collection(&payload).tracks.is_empty());
        assert!(parse_browse_page(&json!({"contents":{}}), &target()).is_err());
    }
    #[test]
    fn watch_only_song_preserves_title_artist_and_artwork() {
        let item = json!({"title":{"runs":[{"text":"A song"}]},"subtitle":{"runs":[{"text":"An artist","navigationEndpoint":{"browseEndpoint":{"browseId":"UCartist","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_ARTIST"}}}}}]},"navigationEndpoint":{"watchEndpoint":{"videoId":"watch-only"}},"thumbnailRenderer":{"musicThumbnailRenderer":{"thumbnail":{"thumbnails":[{"url":"https://example.test/cover.jpg"}]}}}});
        let entry = parse_catalog_track(&item).unwrap();
        assert_eq!(entry.track.video_id, "watch-only");
        assert_eq!(entry.track.title, "A song");
        assert_eq!(entry.track.artist, "An artist");
        assert!(entry.track.thumbnail_url.is_some());
    }
    #[test]
    fn linked_album_destination_is_preserved_for_lyrics_matching() {
        let item = json!({
            "title": {"runs": [{"text": "A song"}]},
            "subtitle": {"runs": [
                {"text": "An artist", "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "UCartist",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}
                }}},
                {"text": " • "},
                {"text": "An album", "navigationEndpoint": {"browseEndpoint": {
                    "browseId": "MPalbum",
                    "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}
                }}}
            ]},
            "navigationEndpoint": {"watchEndpoint": {"videoId": "watch-only"}}
        });
        let entry = parse_catalog_track(&item).unwrap();
        assert_eq!(entry.track.artist, "An artist");
        assert_eq!(entry.track.album.as_deref(), Some("An album"));
        // The destinations keep their ids, so menus can open the artist and the album.
        assert_eq!(
            entry
                .track
                .artist_page
                .as_ref()
                .map(|page| page.browse_id.as_str()),
            Some("UCartist")
        );
        assert_eq!(
            entry
                .track
                .album_page
                .as_ref()
                .map(|page| page.browse_id.as_str()),
            Some("MPalbum")
        );
    }
    #[test]
    fn does_not_take_video_ids_from_menu_actions_but_supports_albums() {
        let item = json!({"title":{"simpleText":"Playlist"},"menu":{"navigationEndpoint":{"watchEndpoint":{"videoId":"wrong-video"}}}});
        assert!(parse_catalog_track(&item).is_none());
        let mut album = target();
        album.page_type = Some("MUSIC_PAGE_TYPE_ALBUM".into());
        assert!(matches!(
            parse_browse_page(&responsive(), &album),
            Err(MusicError::InvalidResponse(_))
        ));
    }
    #[test]
    fn continuation_item_is_retained_without_claiming_complete_results() {
        let shelf = json!({"contents":[{"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"fixture-next"}}}}]});
        assert_eq!(parse_continuation(&shelf).unwrap().token, "fixture-next");
        assert_eq!(
            format!("{:?}", parse_continuation(&shelf).unwrap()),
            "Continuation(<opaque>)"
        );
    }
}
