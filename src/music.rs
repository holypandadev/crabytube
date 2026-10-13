mod browse;
pub use browse::{
    BrowseMore, BrowsePage, BrowseTarget, CatalogReference, Collection, CollectionTrack,
    Continuation,
};

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use thiserror::Error;
use tracing::debug;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    /// Album metadata is optional because many YouTube Music renderers omit it.
    #[serde(default)]
    pub album: Option<String>,
    /// Artist and album destinations, kept with their service parameters, when the catalog names
    /// them. The track menu offers "Go to artist" and "Go to album" only for these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_page: Option<browse::BrowseTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_page: Option<browse::BrowseTarget>,
    pub thumbnail_url: Option<String>,
    /// Length in whole seconds, kept so Favorites can show it after a restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum HomeItemKind {
    Song,
    /// A live broadcast: a track that plays as a stream with no end.
    Live,
    Album,
    Artist,
    Playlist,
    Mix,
    Unknown,
}

impl HomeItemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Song => "Song",
            Self::Live => "Live",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Playlist => "Playlist",
            Self::Mix => "Mix",
            Self::Unknown => "Music",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HomeItem {
    pub title: String,
    pub subtitle: String,
    pub thumbnail_url: Option<String>,
    pub kind: HomeItemKind,
    pub browse_id: Option<String>,
    pub browse_target: Option<BrowseTarget>,
    pub track: Option<Track>,
    /// Song length when the catalog row shows one.
    #[serde(default)]
    pub duration: Option<std::time::Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HomeShelf {
    pub title: String,
    pub items: Vec<HomeItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPage {
    pub items: Vec<HomeItem>,
    pub continuation: Option<Continuation>,
}

/// Remote YouTube Music search scope. The encoded values are Innertube's
/// public WEB_REMIX search filters; callers still validate returned kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CatalogFilter {
    #[default]
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl CatalogFilter {
    pub fn params(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Songs => Some("EgWKAQIIAWoKEAMQBBAJEAoQCRAF"),
            Self::Albums => Some("EgWKAQIYAWoKEAMQBBAJEAoQCRAF"),
            Self::Artists => Some("EgWKAQIgAWoKEAMQBBAJEAoQCRAF"),
            // Playlists have a separate WEB_REMIX params shape; the generic
            // EgWKAQ* selector used for videos returns songs in this client.
            Self::Playlists => Some("Eg-KAQwIABAAGAAgACgBMABqChAEEAMQCRAFEAo%3D"),
        }
    }

    pub fn matches(self, kind: HomeItemKind) -> bool {
        match self {
            Self::All => true,
            Self::Songs => kind == HomeItemKind::Song,
            Self::Albums => kind == HomeItemKind::Album,
            Self::Artists => kind == HomeItemKind::Artist,
            Self::Playlists => matches!(kind, HomeItemKind::Playlist | HomeItemKind::Mix),
        }
    }
}

#[derive(Debug, Error)]
pub enum MusicError {
    #[error("could not initialize the network client: {0}")]
    Client(String),
    #[error("network request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("YouTube Music returned HTTP {0}")]
    HttpStatus(reqwest::StatusCode),
    #[error("YouTube Music returned an invalid response: {0}")]
    InvalidResponse(String),
}

const MAX_INNERTUBE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MUSIC_PAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_FEED_SHELVES: usize = 64;
const MAX_FEED_ITEMS_PER_SHELF: usize = 120;
pub(crate) const MAX_CATALOG_ITEMS: usize = 400;
const MAX_RADIO_TRACKS: usize = 400;
pub(crate) const MAX_COLLECTION_TRACKS: usize = 5_000;
pub(crate) const MAX_COLLECTION_PAGE_TRACKS: usize = 1_000;
const MAX_CATALOG_TEXT_BYTES: usize = 4_096;

async fn response_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, MusicError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(MusicError::InvalidResponse(format!(
            "response exceeded the {} MiB limit",
            limit / 1024 / 1024
        )));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(MusicError::InvalidResponse(format!(
                "response exceeded the {} MiB limit",
                limit / 1024 / 1024
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn response_json(response: reqwest::Response, limit: usize) -> Result<Value, MusicError> {
    serde_json::from_slice(&response_bytes(response, limit).await?)
        .map_err(|error| MusicError::InvalidResponse(error.to_string()))
}

impl MusicError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Client(_) => "Network setup failed. Please restart the app and try again.".into(),
            Self::Network(_) => {
                "Could not reach YouTube Music. Check your connection and try again.".into()
            }
            Self::HttpStatus(status) if status.as_u16() == 429 => {
                "YouTube Music is receiving too many requests. Try again shortly.".into()
            }
            Self::HttpStatus(_) => {
                "YouTube Music could not complete this request. Try again later.".into()
            }
            Self::InvalidResponse(_) => {
                "YouTube Music returned data this version cannot read.".into()
            }
        }
    }
}

#[async_trait]
pub trait MusicService: Send + Sync {
    async fn search_catalog(&self, query: &str) -> Result<Vec<HomeItem>, MusicError>;
    async fn search_catalog_filtered(
        &self,
        query: &str,
        filter: CatalogFilter,
    ) -> Result<Vec<HomeItem>, MusicError> {
        Ok(self
            .search_catalog(query)
            .await?
            .into_iter()
            .filter(|item| filter.matches(item.kind))
            .collect())
    }
    async fn search_catalog_filtered_page(
        &self,
        query: &str,
        filter: CatalogFilter,
        continuation: Option<Continuation>,
    ) -> Result<CatalogPage, MusicError> {
        if continuation.is_some() {
            return Ok(CatalogPage {
                items: Vec::new(),
                continuation: None,
            });
        }
        Ok(CatalogPage {
            items: self.search_catalog_filtered(query, filter).await?,
            continuation: None,
        })
    }
    async fn radio(&self, _video_id: &str) -> Result<Vec<Track>, MusicError> {
        Ok(Vec::new())
    }

    async fn home(&self) -> Result<Vec<HomeShelf>, MusicError>;
    async fn home_more(&self) -> Result<Vec<HomeShelf>, MusicError> {
        Ok(Vec::new())
    }
    async fn browse(&self, target: &BrowseTarget) -> Result<BrowsePage, MusicError>;
    async fn browse_more(&self, _continuation: &Continuation) -> Result<BrowseMore, MusicError> {
        Ok(BrowseMore {
            tracks: Vec::new(),
            continuation: None,
        })
    }
    async fn suggestions(&self, query: &str) -> Result<Vec<String>, MusicError>;
}

/// Anonymous, read-only access to the public YouTube Music web search endpoint.
/// It does not load or persist account credentials.
pub struct YouTubeMusicService {
    session: YouTubeSession,
    home_config: tokio::sync::Mutex<Option<WebMusicConfig>>,
    suggestions: std::sync::Mutex<SuggestionCache>,
    /// Continuation token of the last Home feed, followed by `home_more`.
    home_continuation: std::sync::Mutex<Option<String>>,
}

const SUGGESTION_CACHE_ENTRIES: usize = 64;

/// Suggestions for recently typed prefixes, most recent first.
#[derive(Default)]
struct SuggestionCache(std::collections::VecDeque<(String, Vec<String>)>);

impl SuggestionCache {
    fn get(&mut self, query: &str) -> Option<Vec<String>> {
        let index = self.0.iter().position(|(key, _)| key == query)?;
        let entry = self.0.remove(index)?;
        let value = entry.1.clone();
        self.0.push_front(entry);
        Some(value)
    }

    fn put(&mut self, query: String, suggestions: Vec<String>) {
        self.0.retain(|(key, _)| key != &query);
        self.0.push_front((query, suggestions));
        self.0.truncate(SUGGESTION_CACHE_ENTRIES);
    }
}

/// Innertube `hl` for the current interface language.
pub fn content_language() -> &'static str {
    crate::i18n::current_language().content_code()
}

#[derive(Clone)]
struct WebMusicConfig {
    api_key: String,
    client_id: String,
    client_version: String,
    context: Value,
    visitor_data: Option<String>,
}

impl From<crate::runtime_cache::WebValues> for WebMusicConfig {
    fn from(values: crate::runtime_cache::WebValues) -> Self {
        Self {
            api_key: values.api_key,
            client_id: values.client_id,
            client_version: values.client_version,
            context: values.context,
            visitor_data: values.visitor_data,
        }
    }
}

impl From<WebMusicConfig> for crate::runtime_cache::WebValues {
    fn from(config: WebMusicConfig) -> Self {
        Self {
            api_key: config.api_key,
            client_id: config.client_id,
            client_version: config.client_version,
            visitor_data: config.visitor_data,
            context: config.context,
        }
    }
}

#[derive(Clone, Default)]
pub struct YouTubeSession {
    visitor_data: Arc<RwLock<Option<String>>>,
}

impl YouTubeSession {
    pub fn visitor_data(&self) -> Option<String> {
        self.visitor_data
            .read()
            .ok()
            .and_then(|value| value.clone())
    }

    fn update_visitor_data(&self, value: Option<&str>) {
        if let (Some(value), Ok(mut current)) = (value, self.visitor_data.write())
            && !value.is_empty()
        {
            *current = Some(value.to_owned());
        }
    }
}

impl YouTubeMusicService {
    pub fn new() -> Self {
        Self::with_session(YouTubeSession::default())
    }

    pub fn with_session(session: YouTubeSession) -> Self {
        Self {
            session,
            home_config: tokio::sync::Mutex::new(None),
            suggestions: std::sync::Mutex::new(SuggestionCache::default()),
            home_continuation: std::sync::Mutex::new(None),
        }
    }

    fn client(&self) -> Result<&'static Client, MusicError> {
        crate::network::shared().map_err(MusicError::Client)
    }

    async fn live_web_music_config(&self) -> Result<WebMusicConfig, MusicError> {
        let mut config = self.shared_web_music_config().await?;
        // Catalog text follows the app language, not the IP or system locale.
        if let Some(client) = config
            .context
            .get_mut("client")
            .and_then(Value::as_object_mut)
        {
            client.insert("hl".into(), Value::String(content_language().into()));
        }
        Ok(config)
    }

    async fn shared_web_music_config(&self) -> Result<WebMusicConfig, MusicError> {
        let mut slot = self.home_config.lock().await;
        if let Some(config) = slot.as_ref() {
            return Ok(config.clone());
        }
        let config = match crate::runtime_cache::web() {
            Some(saved) => saved.into(),
            None => {
                let fetched = self.fetch_web_music_config().await?;
                crate::runtime_cache::store_web(fetched.clone().into());
                fetched
            }
        };
        *slot = Some(config.clone());
        Ok(config)
    }

    /// Drops the WEB_REMIX values after Innertube refuses them, so the next call fetches them again.
    async fn reject_web_config_if_forbidden(&self, status: reqwest::StatusCode) {
        if status == reqwest::StatusCode::FORBIDDEN {
            *self.home_config.lock().await = None;
            crate::runtime_cache::forget_web();
        }
    }

    async fn fetch_web_music_config(&self) -> Result<WebMusicConfig, MusicError> {
        let response = self
            .client()?
            .get("https://music.youtube.com/")
            .header("Accept-Language", content_language())
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(MusicError::HttpStatus(response.status()));
        }
        let page = String::from_utf8(response_bytes(response, MAX_MUSIC_PAGE_BYTES).await?)
            .map_err(|error| MusicError::InvalidResponse(error.to_string()))?;
        let config = crate::playback::parse_ytcfg(&page).ok_or_else(|| {
            MusicError::InvalidResponse("music.youtube.com ytcfg was missing".into())
        })?;
        let api_key = config
            .get("INNERTUBE_API_KEY")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                MusicError::InvalidResponse("live Innertube API key was missing".into())
            })?
            .to_owned();
        let mut context = config
            .get("INNERTUBE_CONTEXT")
            .cloned()
            .unwrap_or_else(|| json!({"client": {}, "user": {}}));
        let client = context
            .get_mut("client")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                MusicError::InvalidResponse("live WEB_REMIX client context was missing".into())
            })?;
        let client_name = config
            .get("INNERTUBE_CLIENT_NAME")
            .and_then(Value::as_str)
            .or_else(|| client.get("clientName").and_then(Value::as_str))
            .unwrap_or("WEB_REMIX")
            .to_owned();
        let client_version = config
            .get("INNERTUBE_CLIENT_VERSION")
            .and_then(Value::as_str)
            .or_else(|| client.get("clientVersion").and_then(Value::as_str))
            .ok_or_else(|| {
                MusicError::InvalidResponse("live WEB_REMIX client version was missing".into())
            })?
            .to_owned();
        let client_id = config
            .get("INNERTUBE_CONTEXT_CLIENT_NAME")
            .map(|value| match value {
                Value::String(value) => value.clone(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "67".into());
        client.insert("clientName".into(), Value::String(client_name.clone()));
        client.insert(
            "clientVersion".into(),
            Value::String(client_version.clone()),
        );
        let visitor_data = config
            .get("VISITOR_DATA")
            .and_then(Value::as_str)
            .or_else(|| client.get("visitorData").and_then(Value::as_str))
            .map(str::to_owned);
        Ok(WebMusicConfig {
            api_key,
            client_id,
            client_version,
            context,
            visitor_data,
        })
    }
    async fn catalog_payload(&self, endpoint: &str, mut body: Value) -> Result<Value, MusicError> {
        let config = self.live_web_music_config().await?;
        body["context"] = config.context;
        let mut request = self
            .client()?
            .post(format!(
                "https://music.youtube.com/youtubei/v1/{endpoint}?alt=json&key={}",
                config.api_key
            ))
            .header("X-YouTube-Client-Name", config.client_id)
            .header("X-YouTube-Client-Version", config.client_version)
            .header("Origin", "https://music.youtube.com")
            .json(&body);
        if let Some(visitor) = config.visitor_data.or_else(|| self.session.visitor_data()) {
            request = request.header("X-Goog-Visitor-Id", visitor);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            self.reject_web_config_if_forbidden(response.status()).await;
            return Err(MusicError::HttpStatus(response.status()));
        }
        let payload = response_json(response, MAX_INNERTUBE_RESPONSE_BYTES).await?;
        self.session.update_visitor_data(
            payload
                .pointer("/responseContext/visitorData")
                .and_then(Value::as_str),
        );
        Ok(payload)
    }
    async fn browse_payload(&self, target: &BrowseTarget) -> Result<Value, MusicError> {
        let config = self.live_web_music_config().await?;
        let visitor_data = config
            .visitor_data
            .clone()
            .or_else(|| self.session.visitor_data());
        let mut request = self
            .client()?
            .post(format!(
                "https://music.youtube.com/youtubei/v1/browse?alt=json&key={}",
                config.api_key
            ))
            .header("X-YouTube-Client-Name", &config.client_id)
            .header("X-YouTube-Client-Version", &config.client_version)
            .header("Origin", "https://music.youtube.com")
            .json(&json!({
                "context": config.context,
                "browseId": target.browse_id,
                "params": target.params
            }));
        if let Some(visitor_data) = visitor_data.as_deref() {
            request = request.header("X-Goog-Visitor-Id", visitor_data);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            self.reject_web_config_if_forbidden(response.status()).await;
            return Err(MusicError::HttpStatus(response.status()));
        }
        let payload = response_json(response, MAX_INNERTUBE_RESPONSE_BYTES).await?;
        self.session.update_visitor_data(
            payload
                .pointer("/responseContext/visitorData")
                .and_then(Value::as_str),
        );
        Ok(payload)
    }
}

impl Default for YouTubeMusicService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicService for YouTubeMusicService {
    async fn search_catalog(&self, query: &str) -> Result<Vec<HomeItem>, MusicError> {
        self.search_catalog_filtered(query, CatalogFilter::All)
            .await
    }
    async fn search_catalog_filtered(
        &self,
        query: &str,
        filter: CatalogFilter,
    ) -> Result<Vec<HomeItem>, MusicError> {
        Ok(self
            .search_catalog_filtered_page(query, filter, None)
            .await?
            .items)
    }
    async fn search_catalog_filtered_page(
        &self,
        query: &str,
        filter: CatalogFilter,
        continuation: Option<Continuation>,
    ) -> Result<CatalogPage, MusicError> {
        let is_continuation = continuation.is_some();
        let payload = if let Some(continuation) = continuation {
            self.catalog_payload("search", json!({"continuation": continuation.token}))
                .await?
        } else {
            let mut body = json!({"query": query});
            if let Some(params) = filter.params() {
                body["params"] = Value::String(params.into());
            }
            self.catalog_payload("search", body).await?
        };
        if is_continuation && let Some(page) = browse::parse_search_more(&payload) {
            return Ok(CatalogPage {
                items: page
                    .0
                    .into_iter()
                    .filter(|item| filter.matches(item.kind))
                    .collect(),
                continuation: page.1,
            });
        }
        Ok(CatalogPage {
            items: parse_catalog_results(&payload)
                .into_iter()
                .filter(|item| filter.matches(item.kind))
                .collect(),
            continuation: browse::parse_search_continuation(&payload),
        })
    }
    async fn radio(&self, video_id: &str) -> Result<Vec<Track>, MusicError> {
        let payload = self
            .catalog_payload(
                "next",
                json!({"videoId":video_id,"isAudioOnly":true,"enablePersistentPlaylistPanel":true}),
            )
            .await?;
        let mut tracks = parse_radio(&payload);
        if let Some(endpoint) = browse::find_renderer(&payload, &["watchPlaylistEndpoint"])
            && let Some(id) = endpoint.get("playlistId").and_then(Value::as_str)
        {
            let more = self
                .catalog_payload(
                    "next",
                    json!({"playlistId":id,"videoId":video_id,"isAudioOnly":true}),
                )
                .await?;
            tracks.extend(parse_radio(&more));
        }
        let mut seen = std::collections::HashSet::new();
        tracks.retain(|t| t.video_id != video_id && seen.insert(t.video_id.clone()));
        Ok(tracks)
    }
    async fn home(&self) -> Result<Vec<HomeShelf>, MusicError> {
        let target = BrowseTarget {
            browse_id: "FEmusic_home".into(),
            params: None,
            page_type: None,
        };
        let payload = self.browse_payload(&target).await?;
        if let Ok(mut token) = self.home_continuation.lock() {
            *token = home_feed_continuation(&payload);
        }
        let shelves = parse_home_feed(&payload)
            .filter(|shelves| !shelves.is_empty())
            .ok_or_else(|| {
                MusicError::InvalidResponse("primary music feed did not contain shelves".into())
            })?;
        debug!(
            shelves = shelves.len(),
            items = shelves.iter().map(|shelf| shelf.items.len()).sum::<usize>(),
            "loaded primary YouTube Music home feed"
        );
        Ok(shelves)
    }

    /// Follows the Home feed's continuations. YouTube Music places shelves such
    /// as "All-time essentials" on a later page, so a few pages are read.
    async fn home_more(&self) -> Result<Vec<HomeShelf>, MusicError> {
        const MAX_PAGES: usize = 3;
        let mut token = self
            .home_continuation
            .lock()
            .ok()
            .and_then(|mut token| token.take());
        let mut shelves = Vec::new();
        for _ in 0..MAX_PAGES {
            let Some(current) = token.take() else {
                break;
            };
            let payload = self
                .catalog_payload("browse", json!({"continuation": current}))
                .await?;
            let (page, next) = parse_home_continuation(&payload);
            append_unique_shelves(&mut shelves, page);
            token = next;
        }
        Ok(shelves)
    }
    async fn browse(&self, target: &BrowseTarget) -> Result<BrowsePage, MusicError> {
        let payload = self.browse_payload(target).await?;
        browse::parse_browse_page(&payload, target)
    }
    async fn browse_more(&self, continuation: &Continuation) -> Result<BrowseMore, MusicError> {
        let payload = self
            .catalog_payload("browse", json!({"continuation": continuation.token}))
            .await?;
        browse::parse_browse_more(&payload)
    }

    async fn suggestions(&self, query: &str) -> Result<Vec<String>, MusicError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        if let Ok(mut cache) = self.suggestions.lock()
            && let Some(hit) = cache.get(query)
        {
            return Ok(hit);
        }
        let config = self.live_web_music_config().await?;
        let visitor_data = config
            .visitor_data
            .clone()
            .or_else(|| self.session.visitor_data());
        let mut request = self.client()?
            .post(format!(
                "https://music.youtube.com/youtubei/v1/music/get_search_suggestions?alt=json&key={}",
                config.api_key
            ))
            .header("X-YouTube-Client-Name", &config.client_id)
            .header("X-YouTube-Client-Version", &config.client_version)
            .header("Origin", "https://music.youtube.com")
            .json(&json!({
                "context": config.context,
                "input": query
            }));
        if let Some(visitor_data) = visitor_data.as_deref() {
            request = request.header("X-Goog-Visitor-Id", visitor_data);
        }
        let response = request.send().await?;
        if !response.status().is_success() {
            self.reject_web_config_if_forbidden(response.status()).await;
            return Err(MusicError::HttpStatus(response.status()));
        }
        let payload = response_json(response, MAX_INNERTUBE_RESPONSE_BYTES).await?;
        self.session.update_visitor_data(
            payload
                .pointer("/responseContext/visitorData")
                .and_then(Value::as_str),
        );
        let suggestions = parse_suggestions(&payload);
        if let Ok(mut cache) = self.suggestions.lock() {
            cache.put(query.to_owned(), suggestions.clone());
        }
        Ok(suggestions)
    }
}

/// Token for the next page of a Home feed or of one of its continuations.
fn home_feed_continuation(payload: &Value) -> Option<String> {
    [
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/continuations/0/nextContinuationData/continuation",
        "/continuationContents/sectionListContinuation/continuations/0/nextContinuationData/continuation",
    ]
    .iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(Value::as_str))
    // The same bound as the continuations of other pages.
    .filter(|token| !token.is_empty() && token.len() <= 32 * 1024)
    .map(str::to_owned)
}

/// Shelves of one Home continuation page and the token for the page after it.
fn parse_home_continuation(payload: &Value) -> (Vec<HomeShelf>, Option<String>) {
    let shelves = payload
        .pointer("/continuationContents/sectionListContinuation/contents")
        .and_then(Value::as_array)
        .map(|sections| {
            sections
                .iter()
                .take(MAX_FEED_SHELVES)
                .filter_map(parse_home_shelf)
                .filter(|shelf| !shelf.items.is_empty())
                .collect()
        })
        .unwrap_or_default();
    (shelves, home_feed_continuation(payload))
}

fn parse_home_feed(payload: &Value) -> Option<Vec<HomeShelf>> {
    let sections = payload
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .or_else(|| {
            payload.pointer(
                "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
            )
        })?
        .as_array()?;

    // FEmusic_home nests shelves inside itemSectionRenderer in some sessions.
    let flattened: Vec<&Value> = sections
        .iter()
        .take(MAX_FEED_SHELVES)
        .flat_map(|section| {
            if let Some(inner) = section
                .get("itemSectionRenderer")
                .and_then(|renderer| renderer.get("contents"))
                .and_then(Value::as_array)
            {
                inner
                    .iter()
                    .take(MAX_FEED_ITEMS_PER_SHELF)
                    .collect::<Vec<&Value>>()
            } else {
                vec![section]
            }
        })
        .collect();

    Some(
        flattened
            .iter()
            .take(MAX_FEED_SHELVES)
            .filter_map(|section| parse_home_shelf(section))
            .filter(|shelf| !shelf.items.is_empty())
            .collect(),
    )
}

fn shelf_identity(shelf: &HomeShelf) -> (String, String) {
    let title = shelf.title.trim().to_lowercase();
    let first = shelf.items.first();
    let identity = first
        .and_then(|item| item.browse_id.clone())
        .or_else(|| {
            first
                .and_then(|item| item.track.as_ref())
                .map(|track| track.video_id.clone())
        })
        .or_else(|| first.map(|item| item.title.trim().to_lowercase()))
        .unwrap_or_default();
    (title, identity)
}

pub(crate) fn append_unique_shelves(destination: &mut Vec<HomeShelf>, incoming: Vec<HomeShelf>) {
    let mut seen: HashSet<_> = destination.iter().map(shelf_identity).collect();
    let remaining = MAX_FEED_SHELVES.saturating_sub(destination.len());
    destination.extend(
        incoming
            .into_iter()
            .filter(|shelf| seen.insert(shelf_identity(shelf)))
            .take(remaining),
    );
}

fn parse_suggestions(payload: &Value) -> Vec<String> {
    fn visit(v: &Value, out: &mut Vec<String>) {
        if let Some(r) = v.get("searchSuggestionRenderer")
            && let Some(text) = r.get("suggestion").and_then(text_value).or_else(|| {
                r.pointer("/navigationEndpoint/searchEndpoint/query")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        {
            let text = text.trim().to_owned();
            if !text.is_empty() && !out.contains(&text) {
                out.push(text);
            }
            return;
        }
        match v {
            Value::Object(map) => {
                for child in map.values() {
                    visit(child, out)
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, out)
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    visit(payload, &mut out);
    out.truncate(8);
    out
}

fn parse_catalog_results(payload: &Value) -> Vec<HomeItem> {
    fn visit(v: &Value, out: &mut Vec<HomeItem>, card_artist: Option<&str>) {
        if out.len() >= MAX_CATALOG_ITEMS {
            return;
        }
        if let Some(item) = parse_home_item(v) {
            out.push(credit_card_artist(item, card_artist));
            return;
        }
        match v {
            Value::Object(map) => {
                for (key, child) in map {
                    // A card whose title opens an artist heads that artist's shelf of songs.
                    let shelf_artist = if key == "musicCardShelfRenderer" {
                        artist_card_name(child)
                    } else {
                        None
                    };
                    visit(child, out, shelf_artist.as_deref().or(card_artist));
                    if out.len() >= MAX_CATALOG_ITEMS {
                        return;
                    }
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, out, card_artist);
                    if out.len() >= MAX_CATALOG_ITEMS {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(contents) = payload.get("contents") {
        visit(contents, &mut out, None);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|i| {
        seen.insert((
            i.title.clone(),
            i.browse_id.clone(),
            i.track.as_ref().map(|t| t.video_id.clone()),
        ))
    });
    out
}
/// The artist an artist card names. The songs in its shelf are that artist's, and their rows
/// leave the artist out of the byline. The card is known by the page its title opens, since the
/// word "Artist" in its subtitle comes in the language of the interface.
fn artist_card_name(card: &Value) -> Option<String> {
    let title = card.get("title")?;
    let opens_artist = title
        .get("runs")
        .and_then(Value::as_array)
        .is_some_and(|runs| {
            runs.iter().any(|run| {
                run.pointer("/navigationEndpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
                    .and_then(Value::as_str)
                    == Some("MUSIC_PAGE_TYPE_ARTIST")
            })
        });
    if !opens_artist {
        return None;
    }
    text_value(title).filter(|name| !name.trim().is_empty())
}

/// Fills in the artist of a song row that has none, from the card it sits in.
fn credit_card_artist(mut item: HomeItem, card_artist: Option<&str>) -> HomeItem {
    let Some(artist) = card_artist else {
        return item;
    };
    if let Some(track) = item.track.as_mut()
        && track.artist.trim().is_empty()
    {
        track.artist = artist.to_owned();
        if item.subtitle.trim().is_empty() {
            item.subtitle = artist.to_owned();
        }
    }
    item
}

fn parse_radio(payload: &Value) -> Vec<Track> {
    fn visit(v: &Value, out: &mut Vec<Track>) {
        if out.len() >= MAX_RADIO_TRACKS {
            return;
        }
        if let Some(r) = v.get("playlistPanelVideoRenderer") {
            if let (Some(id), Some(title)) = (
                r.get("videoId").and_then(Value::as_str),
                r.get("title").and_then(text_value),
            ) {
                out.push(Track {
                    video_id: id.into(),
                    title,
                    // The short byline is the artist alone; the long one adds album and year.
                    artist: r
                        .get("shortBylineText")
                        .and_then(text_value)
                        .or_else(|| {
                            r.get("longBylineText")
                                .and_then(text_value)
                                .map(|text| browse::clean_byline(&text))
                        })
                        .unwrap_or_default(),
                    album: None,
                    artist_page: None,
                    album_page: None,
                    thumbnail_url: home_item_thumbnail(r),
                    duration_secs: r
                        .get("lengthText")
                        .and_then(text_value)
                        .as_deref()
                        .and_then(browse::parse_duration)
                        .and_then(|length| u32::try_from(length.as_secs()).ok()),
                });
            }
            return;
        }
        match v {
            Value::Object(map) => {
                for child in map.values() {
                    visit(child, out);
                    if out.len() >= MAX_RADIO_TRACKS {
                        return;
                    }
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, out);
                    if out.len() >= MAX_RADIO_TRACKS {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    visit(payload, &mut out);
    out
}

#[cfg(test)]
mod live_feed_probe {
    use super::*;

    /// Opt-in catalog contract check: the public browse IDs Explore uses still
    /// return real shelves.
    #[tokio::test]
    #[ignore = "requires live anonymous YouTube Music; run explicitly"]
    async fn known_public_music_browse_ids_return_shelves() {
        let service = YouTubeMusicService::new();
        for browse_id in ["FEmusic_new_releases", "FEmusic_charts"] {
            let payload = service
                .browse_payload(&BrowseTarget {
                    browse_id: browse_id.into(),
                    params: None,
                    page_type: None,
                })
                .await
                .expect("live browse payload");
            let shelves =
                parse_home_feed(&payload).expect("live browse payload has shelf renderer");
            assert!(!shelves.is_empty(), "{browse_id} returned no live shelves");
            println!(
                "{browse_id}: shelves={}, items={}",
                shelves.len(),
                shelves.iter().map(|shelf| shelf.items.len()).sum::<usize>()
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live anonymous YouTube Music; run explicitly"]
    async fn playlist_continuation_loads_more_tracks() {
        use futures_util::StreamExt;

        let service = YouTubeMusicService::new();
        let mut shelves = service.home().await.unwrap();
        shelves.extend(service.home_more().await.unwrap_or_default());
        let mut seen = HashSet::new();
        let mut targets: Vec<_> = shelves
            .iter()
            .flat_map(|shelf| &shelf.items)
            .filter_map(|item| item.browse_target.clone())
            .filter(|target| target.is_public_collection() && seen.insert(target.browse_id.clone()))
            .take(12)
            .collect();
        if let Ok(search) = service
            .search_catalog_filtered_page("Pop's Biggest Hits", CatalogFilter::Playlists, None)
            .await
        {
            for item in search.items {
                if let Some(target) = item.browse_target
                    && target.is_public_collection()
                    && seen.insert(target.browse_id.clone())
                {
                    targets.push(target);
                }
            }
        }
        let mut candidates = futures_util::stream::iter(targets)
            .map(|target| {
                let service = &service;
                async move {
                    match service.browse(&target).await.ok()? {
                        BrowsePage::Collection(page) => Some(page),
                        _ => None,
                    }
                }
            })
            .buffer_unordered(4);
        let mut page = None;
        while let Some(candidate) = candidates.next().await {
            if candidate
                .as_ref()
                .is_some_and(|page| page.continuation.is_some())
            {
                page = candidate;
                break;
            }
        }
        let page =
            page.expect("Home or playlist search should expose a paginated public collection");
        let continuation = page
            .continuation
            .expect("page was filtered for continuation");
        let more = service.browse_more(&continuation).await.unwrap();
        assert!(!more.tracks.is_empty(), "continuation returned no tracks");
        println!(
            "live collection continuation: {} tracks then {} more",
            page.tracks.len(),
            more.tracks.len()
        );
    }

    #[tokio::test]
    #[ignore = "requires live anonymous YouTube Music; run explicitly"]
    async fn playlist_filtered_search_exposes_public_collection_results() {
        let service = YouTubeMusicService::new();
        let payload = service
            .catalog_payload(
                "search",
                json!({
                    "query": "Pop's Biggest Hits",
                    "params": CatalogFilter::Playlists.params().unwrap()
                }),
            )
            .await
            .unwrap();
        fn count_renderers(value: &Value, counts: &mut std::collections::BTreeMap<String, usize>) {
            match value {
                Value::Object(map) => {
                    for (key, child) in map {
                        if key.ends_with("Renderer") {
                            *counts.entry(key.clone()).or_default() += 1;
                        }
                        count_renderers(child, counts);
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        count_renderers(child, counts);
                    }
                }
                _ => {}
            }
        }
        let mut counts = std::collections::BTreeMap::new();
        count_renderers(&payload, &mut counts);
        println!("playlist filter renderer types: {counts:?}");
        let items = parse_catalog_results(&payload);
        println!(
            "playlist filter parsed items: {}; collection targets: {}",
            items.len(),
            items
                .iter()
                .filter(|item| CatalogFilter::Playlists.matches(item.kind))
                .count()
        );
        assert!(
            items
                .iter()
                .any(|item| CatalogFilter::Playlists.matches(item.kind)),
            "live playlist-filter response contained no recognized collections"
        );

        let first_page = service
            .search_catalog_filtered_page("Pop's Biggest Hits", CatalogFilter::Playlists, None)
            .await
            .unwrap();
        let continuation = first_page
            .continuation
            .expect("filtered playlist search should expose a continuation");
        let next_page = service
            .search_catalog_filtered_page(
                "Pop's Biggest Hits",
                CatalogFilter::Playlists,
                Some(continuation),
            )
            .await
            .unwrap();
        let first_ids: std::collections::HashSet<_> = first_page
            .items
            .iter()
            .filter_map(|item| item.browse_id.as_deref())
            .collect();
        assert!(
            next_page
                .items
                .iter()
                .any(|item| !first_ids.contains(item.browse_id.as_deref().unwrap_or_default())),
            "the live next page should contain additional playlist results"
        );
        println!(
            "live playlist search continuation: {} first-page, {} next-page results",
            first_page.items.len(),
            next_page.items.len()
        );
    }
}

fn parse_home_shelf(section: &Value) -> Option<HomeShelf> {
    let renderer = section
        .get("musicCarouselShelfRenderer")
        .or_else(|| section.get("musicImmersiveCarouselShelfRenderer"))
        .or_else(|| section.get("musicShelfRenderer"))
        .or_else(|| section.get("musicCardShelfRenderer"))
        .or_else(|| section.get("gridRenderer"))?;
    let title = shelf_title(renderer)
        .or_else(|| text_value(renderer.get("title")?))
        .filter(|title| !title.trim().is_empty())?;
    let items = renderer
        .get("contents")
        .and_then(Value::as_array)?
        .iter()
        .take(MAX_FEED_ITEMS_PER_SHELF)
        .filter_map(parse_home_item)
        .collect();
    Some(HomeShelf { title, items })
}

fn shelf_title(renderer: &Value) -> Option<String> {
    let header = renderer.get("header")?;
    [
        "musicCarouselShelfBasicHeaderRenderer",
        "musicImmersiveCarouselShelfBasicHeaderRenderer",
        "musicShelfBasicHeaderRenderer",
        "musicCardShelfHeaderBasicRenderer",
        "musicHeaderRenderer",
        "musicCarouselShelfHeaderBasicRenderer",
    ]
    .iter()
    .filter_map(|name| header.get(*name))
    .find_map(|header| text_value(header.get("title")?))
}

fn parse_home_item(raw: &Value) -> Option<HomeItem> {
    let item = raw
        .get("musicTwoRowItemRenderer")
        .or_else(|| raw.get("musicImmersiveCarouselItemRenderer"))
        .or_else(|| raw.get("musicResponsiveListItemRenderer"))?;
    let entry = browse::parse_catalog_track(item);
    let duration = entry.as_ref().and_then(|entry| entry.duration);
    let track = entry.map(|entry| entry.track);
    let title = track
        .as_ref()
        .map(|track| track.title.clone())
        .or_else(|| item.get("title").and_then(text_value))
        .or_else(|| {
            item.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text")
                .and_then(text_value)
        })?;
    let subtitle = track
        .as_ref()
        .map(|track| track.artist.clone())
        .or_else(|| item.get("subtitle").and_then(text_value))
        .or_else(|| {
            item.pointer("/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text")
                .and_then(text_value)
        })
        .unwrap_or_default();
    let thumbnail_url = track
        .as_ref()
        .and_then(|track| track.thumbnail_url.clone())
        .or_else(|| home_item_thumbnail(item));

    let browse_target = browse::item_destination(item);
    let browse_id = browse_target
        .as_ref()
        .map(|target| target.browse_id.clone());
    let kind = if let Some(target) = &browse_target {
        match target.page_type.as_deref() {
            Some("MUSIC_PAGE_TYPE_ALBUM") => HomeItemKind::Album,
            Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => HomeItemKind::Artist,
            _ if target.is_public_collection() => {
                if target.browse_id.trim_start_matches("VL").starts_with("RD") {
                    HomeItemKind::Mix
                } else {
                    HomeItemKind::Playlist
                }
            }
            _ => HomeItemKind::Unknown,
        }
    } else if track.is_some() {
        if has_live_badge(item) {
            HomeItemKind::Live
        } else {
            HomeItemKind::Song
        }
    } else {
        HomeItemKind::Unknown
    };

    Some(HomeItem {
        title,
        subtitle,
        thumbnail_url,
        kind,
        browse_id,
        browse_target,
        track,
        duration,
    })
}

/// Search rows of live broadcasts carry a `liveBadgeRenderer` among their badges.
fn has_live_badge(item: &Value) -> bool {
    item.get("badges")
        .and_then(Value::as_array)
        .is_some_and(|badges| {
            badges
                .iter()
                .any(|badge| badge.get("liveBadgeRenderer").is_some())
        })
}

fn home_item_thumbnail(item: &Value) -> Option<String> {
    item.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
        .or_else(|| item.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"))
        .or_else(|| item.pointer("/thumbnail/thumbnails"))
        .and_then(Value::as_array)
        .and_then(|images| images.last())
        .and_then(|image| image.get("url"))
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 2_048)
        .map(str::to_owned)
}

fn text_value(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return Some(bound_catalog_text(text, MAX_CATALOG_TEXT_BYTES));
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
        .map(|text| bound_catalog_text(&text, MAX_CATALOG_TEXT_BYTES))
}

pub(super) fn bound_catalog_text(value: &str, max_bytes: usize) -> String {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn track_duration_round_trips_and_older_tracks_load_without_it() {
        let old: super::Track = serde_json::from_str(
            r#"{"video_id":"a","title":"t","artist":"x","thumbnail_url":null}"#,
        )
        .unwrap();
        assert_eq!(old.duration_secs, None);
        let without = serde_json::to_string(&old).unwrap();
        assert!(!without.contains("duration_secs"));

        let timed = super::Track {
            duration_secs: Some(215),
            ..old
        };
        let restored: super::Track =
            serde_json::from_str(&serde_json::to_string(&timed).unwrap()).unwrap();
        assert_eq!(restored.duration_secs, Some(215));
    }

    use serde_json::json;

    use super::{
        CatalogFilter, HomeItemKind, parse_catalog_results, parse_radio, parse_suggestions,
    };

    #[test]
    fn catalog_filters_have_distinct_remote_params_and_type_contracts() {
        let filters = [
            CatalogFilter::Songs,
            CatalogFilter::Albums,
            CatalogFilter::Artists,
            CatalogFilter::Playlists,
        ];
        let params = filters
            .into_iter()
            .map(|filter| filter.params().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(params.len(), filters.len());
        assert!(CatalogFilter::All.params().is_none());
        assert!(CatalogFilter::Playlists.matches(HomeItemKind::Mix));
        assert!(!CatalogFilter::Albums.matches(HomeItemKind::Song));
    }

    #[test]
    fn parses_search_continuation_rows_and_next_token() {
        let response = json!({
            "continuationContents": {
                "musicShelfContinuation": {
                    "contents": [{"musicResponsiveListItemRenderer": {
                        "title": {"simpleText": "Next Song"},
                        "navigationEndpoint": {"watchEndpoint": {"videoId": "next-song"}}
                    }}],
                    "continuations": [{"nextContinuationData": {"continuation": "next-token"}}]
                }
            }
        });
        let (items, continuation) = super::browse::parse_search_more(&response).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, HomeItemKind::Song);
        assert_eq!(items[0].track.as_ref().unwrap().video_id, "next-song");
        assert_eq!(continuation.unwrap().token, "next-token");
    }

    #[test]
    fn catalog_search_keeps_distinct_song_album_artist_and_playlist_destinations() {
        let payload = json!({"contents":[
          {"musicTwoRowItemRenderer":{"title":{"simpleText":"Album"},"subtitle":{"simpleText":"Artist"},"navigationEndpoint":{"browseEndpoint":{"browseId":"MPalbum","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_ALBUM"}}}}}},
          {"musicTwoRowItemRenderer":{"title":{"simpleText":"Artist"},"subtitle":{"simpleText":"Band"},"navigationEndpoint":{"browseEndpoint":{"browseId":"UCartist","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_ARTIST"}}}}}},
          {"musicTwoRowItemRenderer":{"title":{"simpleText":"Playlist"},"subtitle":{"simpleText":"Owner"},"navigationEndpoint":{"browseEndpoint":{"browseId":"VLplaylist","params":"opaque","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_PLAYLIST"}}}}}},
          {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"song"},"title":{"simpleText":"Song"},"subtitle":{"simpleText":"Singer"}}}
        ]});
        let items = parse_catalog_results(&payload);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].kind, super::HomeItemKind::Album);
        assert_eq!(items[1].kind, super::HomeItemKind::Artist);
        assert_eq!(
            items[2].browse_target.as_ref().unwrap().params.as_deref(),
            Some("opaque")
        );
        assert_eq!(items[3].track.as_ref().unwrap().video_id, "song");
    }
    #[test]
    fn song_rows_in_an_artist_card_take_the_artist_from_the_card() {
        // The subtitle comes in the language of the interface, here German.
        let payload = json!({"contents":[{"musicCardShelfRenderer":{
          "title":{"runs":[{"text":"Artist Name","navigationEndpoint":{"browseEndpoint":{"browseId":"UCartist","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"MUSIC_PAGE_TYPE_ARTIST"}}}}}]},
          "subtitle":{"runs":[{"text":"Künstler"},{"text":" • "},{"text":"1M monthly audience"}]},
          "contents":[
            {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"card1"},"flexColumns":[
              {"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"First"}]}}},
              {"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Song"},{"text":" • "},{"text":"3:45"}]}}}
            ]}}
          ]}},
          {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"plain1"},"title":{"simpleText":"Other"},"subtitle":{"simpleText":"Someone Else"}}}
        ]});
        let items = parse_catalog_results(&payload);
        let card_song = items
            .iter()
            .find(|item| {
                item.track
                    .as_ref()
                    .is_some_and(|track| track.video_id == "card1")
            })
            .unwrap();
        assert_eq!(card_song.track.as_ref().unwrap().artist, "Artist Name");
        assert_eq!(card_song.subtitle, "Artist Name");
        let plain = items
            .iter()
            .find(|item| {
                item.track
                    .as_ref()
                    .is_some_and(|track| track.video_id == "plain1")
            })
            .unwrap();
        assert_eq!(plain.track.as_ref().unwrap().artist, "Someone Else");
    }

    #[test]
    fn search_rows_with_a_live_badge_are_live_items_and_others_stay_songs() {
        let payload = json!({"contents":[
          {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"live1"},"title":{"simpleText":"Radio"},"subtitle":{"simpleText":"Channel"},"badges":[{"liveBadgeRenderer":{"label":{"runs":[{"text":"Live"}]}}}]}},
          {"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"song1"},"title":{"simpleText":"Song"},"subtitle":{"simpleText":"Singer"}}}
        ]});
        let items = parse_catalog_results(&payload);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, super::HomeItemKind::Live);
        assert_eq!(items[0].track.as_ref().unwrap().video_id, "live1");
        assert_eq!(items[1].kind, super::HomeItemKind::Song);
    }

    #[test]
    fn radio_parser_uses_only_playable_playlist_panel_rows() {
        let payload = json!({"contents":{"playlistPanelRenderer":{"contents":[
          {"playlistPanelVideoRenderer":{"videoId":"first","title":{"simpleText":"Song"},"longBylineText":{"simpleText":"Artist"}}},
          {"playlistPanelVideoRenderer":{"title":{"simpleText":"Unavailable"}}}
        ]}}});
        let tracks = parse_radio(&payload);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].video_id, "first");
        assert_eq!(tracks[0].artist, "Artist");
    }

    #[test]
    fn suggestion_cache_returns_recent_prefixes_and_evicts_the_oldest() {
        let mut cache = super::SuggestionCache::default();
        cache.put("da".into(), vec!["daft punk".into()]);
        assert_eq!(cache.get("da"), Some(vec!["daft punk".into()]));
        assert_eq!(cache.get("dae"), None);

        for index in 0..super::SUGGESTION_CACHE_ENTRIES {
            cache.put(format!("q{index}"), Vec::new());
        }
        assert_eq!(cache.get("da"), None);
    }

    #[test]
    fn suggestions_parse_runs_and_fallback_query_with_dedup_limit() {
        let payload = json!({
            "contents": [
                {"searchSuggestionRenderer": {"suggestion": {"runs": [{"text": "daft punk"}]}, "navigationEndpoint": {"searchEndpoint": {"query": "daft punk"}}}},
                {"searchSuggestionRenderer": {"suggestion": {"runs": [{"text": "daft punk "}, {"text": "touch"}]}, "navigationEndpoint": {"searchEndpoint": {"query": "daft punk touch"}}}},
                {"searchSuggestionRenderer": {"navigationEndpoint": {"searchEndpoint": {"query": "fallback"}}}},
                {"searchSuggestionRenderer": {"suggestion": {"runs": [{"text": "daft punk"}]}}}
            ]
        });
        assert_eq!(
            parse_suggestions(&payload),
            vec!["daft punk", "daft punk touch", "fallback"]
        );
        assert!(parse_suggestions(&json!({})).is_empty());
    }
    #[test]
    fn home_preserves_browse_parameters_and_treats_albums_as_collections() {
        let payload =
            serde_json::from_str(include_str!("../tests/fixtures/home_destinations.json")).unwrap();
        let shelves = super::parse_home_feed(&payload).unwrap();
        let items = &shelves[0].items;
        assert_eq!(items.len(), 4);
        let target = items[0].browse_target.as_ref().unwrap();
        assert_eq!(target.browse_id, "VLRDfixture");
        assert_eq!(target.params.as_deref(), Some("fixture-params"));
        assert!(target.is_public_collection());
        assert_eq!(items[0].kind, super::HomeItemKind::Mix);
        assert_eq!(items[1].track.as_ref().unwrap().video_id, "watch-video");
        assert_eq!(items[1].kind, super::HomeItemKind::Song);
        assert_eq!(items[2].kind, super::HomeItemKind::Album);
        assert!(
            items[2]
                .browse_target
                .as_ref()
                .unwrap()
                .is_public_collection()
        );
        assert_eq!(items[3].kind, super::HomeItemKind::Artist);
    }

    #[test]
    fn additional_real_feeds_merge_without_repeating_identical_shelves() {
        use super::{HomeItem, HomeItemKind, HomeShelf, append_unique_shelves};
        let shelf = |title: &str, first_id: &str| HomeShelf {
            title: title.into(),
            items: vec![HomeItem {
                title: first_id.into(),
                subtitle: String::new(),
                thumbnail_url: None,
                kind: HomeItemKind::Playlist,
                browse_id: Some(first_id.into()),
                browse_target: None,
                track: None,
                duration: None,
            }],
        };
        let mut combined = vec![shelf("Home mixes", "RDmix")];
        append_unique_shelves(
            &mut combined,
            vec![shelf("Home mixes", "RDmix"), shelf("New releases", "MPnew")],
        );

        assert_eq!(combined.len(), 2);
        assert_eq!(combined[1].title, "New releases");
    }
}
