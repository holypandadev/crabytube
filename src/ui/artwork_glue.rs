//! Artwork requests, the artwork cache and the cover fades.

use super::*;
use std::ops::Range;

/// A cover a page draws: its URL and the variant it is drawn at.
type CoverRequest = (String, u32);

/// Rows requested above and below the rows on screen, so a small scroll
/// finds its covers already on the way.
const COVER_MARGIN_ROWS: usize = 2;
/// Pause before a cover that failed to download is asked for again.
const COVER_RETRY_COOLDOWN: Duration = Duration::from_secs(4);
/// Attempts per cover on one visit to a page. A cover that keeps failing
/// waits for the next visit instead of being requested forever.
const MAX_COVER_ATTEMPTS: u8 = 3;

/// A cover whose download failed.
#[derive(Clone, Copy)]
pub(super) struct FailedCover {
    /// Earliest moment the cover may be requested again.
    pub(super) retry_at: std::time::Instant,
    pub(super) attempts: u8,
}

fn cover_request(url: Option<&str>, variant: u32) -> Option<CoverRequest> {
    url.map(|url| (url.to_owned(), variant))
}

/// Indices of `window`, with the rows in `visible` first so the ones on screen
/// are downloaded before the margin around them.
fn rows_by_urgency(window: Range<usize>, visible: Range<usize>) -> impl Iterator<Item = usize> {
    visible
        .clone()
        .chain(window.filter(move |index| !visible.contains(index)))
}

impl App {
    /// The cover of the restored track, read from the disk cache when the session starts. Nothing is
    /// downloaded here: without a saved file the player keeps its placeholder until playback starts.
    pub(super) fn restore_cover_task(&self) -> Task<Message> {
        if self.playback_status.track.is_some() {
            return Task::none();
        }
        let Some(url) = self
            .shown_track()
            .and_then(|track| track.thumbnail_url.clone())
        else {
            return Task::none();
        };
        let max_dim = crate::artwork::THUMB_MAX;
        if self.artwork_cache.contains_variant(&url, max_dim) {
            return Task::none();
        }
        let read = url.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || crate::artwork::saved_sized(&read, max_dim))
                    .await
                    .ok()
                    .flatten()
            },
            move |handle| Message::RestoredCoverLoaded { url, handle },
        )
    }

    pub(super) fn load_artwork(&mut self, url: Option<String>) -> Task<Message> {
        if let Some(previous) = self.artwork_cancel.take() {
            previous.cancel();
        }
        let request_id = self.next_artwork_request_id();
        self.artwork_selected_request_id = request_id;
        self.artwork_selected_retried = false;
        self.artwork = None;
        let Some(url) = url else {
            return Task::none();
        };
        let max_dim = crate::artwork::LARGE_MAX;
        if let Some(cached) = self.artwork_cache.get_exact(&url, max_dim) {
            self.artwork = Some(cached.clone());
            return Task::none();
        }
        // Until the large cover arrives, the best smaller one already in memory stands in.
        self.artwork = self.artwork_cache.get_for(&url, max_dim).cloned();
        let cancellation = CancellationToken::new();
        self.artwork_cancel = Some(cancellation.clone());
        let request_url = url.clone();
        Task::perform(
            async move { fetch_player_artwork(request_url, cancellation, max_dim).await },
            move |handle| Message::ArtworkLoaded {
                url,
                max_dim,
                generation: 0,
                request_id,
                handle,
                selected: true,
            },
        )
    }

    pub(super) fn preload_current_page_artwork(&mut self) -> Task<Message> {
        let requests = self.current_page_artwork();
        self.reconcile_artwork(requests)
    }

    /// The covers the current page draws, most urgent first, each with the
    /// variant it is drawn at. Lists use the windows their views render, so the
    /// covers requested are the ones on screen.
    pub(super) fn current_page_artwork(&self) -> Vec<CoverRequest> {
        match &self.page {
            Page::Home => {
                let layout = self.home_layout(self.area);
                let variant = crate::artwork::variant_for_pixels(layout.card * self.pixel_scale());
                let mut requests: Vec<CoverRequest> = self
                    .home_picks
                    .items
                    .iter()
                    .filter_map(|item| cover_request(item.thumbnail_url.as_deref(), variant))
                    .collect();
                let jump_variant = crate::artwork::variant_for_pixels(
                    super::home_panels::JUMP_THUMB * self.pixel_scale(),
                );
                requests.extend(
                    self.recent
                        .iter()
                        .take(layout.jump)
                        .filter_map(|play| cover_request(play.thumbnail_url(), jump_variant)),
                );
                requests
            }
            Page::Search => self.search_artwork(),
            Page::Library => {
                let pitch = favorites::favorites_row_pitch(self.preferences.ui_density);
                let range = layout::virtual_range(
                    self.favorites.len(),
                    self.library_scroll.y,
                    self.area.height,
                    pitch,
                    8,
                );
                self.favorites
                    .iter()
                    .skip(range.start)
                    .take(range.len())
                    .filter_map(|track| {
                        cover_request(track.thumbnail_url.as_deref(), crate::artwork::THUMB_MAX)
                    })
                    .collect()
            }
            Page::NowPlaying => {
                let pitch = self.preferences.ui_density.queue_row_pitch();
                let height = self.page_height();
                let visible =
                    layout::virtual_range(self.queue.len(), self.queue_scroll.y, height, pitch, 0);
                let window = layout::virtual_range(
                    self.queue.len(),
                    self.queue_scroll.y,
                    height,
                    pitch,
                    COVER_MARGIN_ROWS,
                );
                rows_by_urgency(window, visible)
                    .filter_map(|index| {
                        cover_request(
                            self.queue[index].thumbnail_url.as_deref(),
                            crate::artwork::THUMB_MAX,
                        )
                    })
                    .collect()
            }
            Page::CollectionDetail(_) => {
                let Some(data) = self.collection.data.as_ref() else {
                    return Vec::new();
                };
                let pitch = self.preferences.ui_density.collection_row_pitch();
                let visible = layout::virtual_range(
                    data.tracks.len(),
                    self.collection_scroll,
                    self.area.height,
                    pitch,
                    0,
                );
                let window = layout::virtual_range(
                    data.tracks.len(),
                    self.collection_scroll,
                    self.area.height,
                    pitch,
                    COVER_MARGIN_ROWS,
                );
                // The header is drawn at the card size and comes first.
                data.thumbnail_url
                    .iter()
                    .map(|url| (url.clone(), crate::artwork::CARD_MAX))
                    .chain(rows_by_urgency(window, visible).filter_map(|index| {
                        cover_request(
                            data.tracks[index].track.thumbnail_url.as_deref(),
                            crate::artwork::THUMB_MAX,
                        )
                    }))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The covers of the results page, in the order the page draws them.
    fn search_artwork(&self) -> Vec<CoverRequest> {
        let top = self.catalog_results.first().and_then(|item| {
            cover_request(item.thumbnail_url.as_deref(), crate::artwork::CARD_MID_MAX)
        });
        // Same order as the results page: live broadcasts, then songs, then the grids.
        let songs = self.search_live().into_iter().chain(self.search_songs());
        let collections = [
            HomeItemKind::Album,
            HomeItemKind::Playlist,
            HomeItemKind::Artist,
        ]
        .into_iter()
        .flat_map(|kind| self.search_collections(kind));
        top.into_iter()
            .chain(songs.chain(collections).filter_map(|item| {
                cover_request(item.thumbnail_url.as_deref(), crate::artwork::THUMB_MAX)
            }))
            .collect()
    }

    /// The cover of the track the player shows. It stays in memory while the cache is trimmed. For a
    /// restored track that is the queue's current entry, until the engine reports a track.
    pub(super) fn playing_cover_url(&self) -> Option<&str> {
        self.shown_track()
            .and_then(|track| track.thumbnail_url.as_deref())
    }

    /// Chooses the largest variant that still lets the whole visible set
    /// fit in the decoded-byte budget (with a reserve for Now Playing).
    /// This keeps large 2560x1440 viewports covered instead of truncating
    /// them to 10 covers at 320px.
    pub(super) fn effective_variant_for_count(requested: u32, count: usize) -> u32 {
        const RESERVE: usize = 2 * 1024 * 1024;
        let usable = crate::artwork::MAX_ARTWORK_BYTES.saturating_sub(RESERVE);
        let mut candidate = requested.clamp(64, crate::artwork::LARGE_MAX);
        // Downgrade path: 640 -> 320 -> 192 -> 128.
        let downgrade = |dim: u32| match dim {
            d if d > crate::artwork::CARD_MAX => crate::artwork::CARD_MAX,
            d if d > crate::artwork::CARD_MID_MAX => crate::artwork::CARD_MID_MAX,
            d if d > crate::artwork::THUMB_MAX => crate::artwork::THUMB_MAX,
            _ => crate::artwork::THUMB_MAX,
        };
        for _ in 0..4 {
            let bytes = (candidate as usize)
                .saturating_mul(candidate as usize)
                .saturating_mul(4);
            if count.saturating_mul(bytes) <= usable || candidate <= crate::artwork::THUMB_MAX {
                break;
            }
            let next = downgrade(candidate);
            if next >= candidate {
                break;
            }
            candidate = next;
        }
        candidate
    }

    /// Leaving a page cancels its downloads and gives failed covers a fresh start on the next one.
    pub(super) fn cancel_artwork_batch(&mut self) {
        for (_, (_, token)) in self.artwork_batch_tokens.drain() {
            token.cancel();
        }
        self.artwork_requested.clear();
        self.artwork_failed.clear();
    }

    pub(super) fn next_artwork_request_id(&mut self) -> u64 {
        self.artwork_request_id = self.artwork_request_id.wrapping_add(1);
        self.artwork_request_id
    }

    /// Brings the covers in flight in line with `requests`. Work for covers
    /// that left the window is cancelled, and missing covers are fetched in
    /// the order given. The window is reconciled on every call, so a cover
    /// that failed or was evicted is asked for again.
    pub(super) fn reconcile_artwork(&mut self, requests: Vec<CoverRequest>) -> Task<Message> {
        let mut seen = HashSet::new();
        let requests: Vec<CoverRequest> = requests
            .into_iter()
            .filter(|request| seen.insert(request.clone()))
            .take(crate::artwork::MAX_ARTWORK_ENTRIES)
            .collect();
        // Each variant group adapts to the decoded-byte budget as a whole.
        let mut group_sizes: HashMap<u32, usize> = HashMap::new();
        for (_, variant) in &requests {
            *group_sizes.entry(*variant).or_default() += 1;
        }
        let mut seen = HashSet::new();
        let wanted_in_order: Vec<CoverRequest> = requests
            .iter()
            .map(|(url, variant)| {
                let effective = Self::effective_variant_for_count(*variant, group_sizes[variant]);
                (url.clone(), effective)
            })
            .filter(|request| seen.insert(request.clone()))
            .collect();
        let wanted: HashSet<CoverRequest> = wanted_in_order.iter().cloned().collect();

        // Cancel only work that left the window.
        let stale: Vec<CoverRequest> = self
            .artwork_requested
            .iter()
            .filter(|key| !wanted.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            if let Some((_, token)) = self.artwork_batch_tokens.remove(&key) {
                token.cancel();
            }
            self.artwork_requested.remove(&key);
        }
        if let Some(url) = self.playing_cover_url().map(str::to_owned) {
            self.artwork_cache.touch(&url, crate::artwork::LARGE_MAX);
        }
        for (url, variant) in &wanted_in_order {
            self.artwork_cache.touch(url, *variant);
        }

        let now = std::time::Instant::now();
        let to_fetch: Vec<CoverRequest> = wanted_in_order
            .into_iter()
            .filter(|(url, variant)| {
                let key = (url.clone(), *variant);
                !self.artwork_cache.contains_variant(url, *variant)
                    && !self.artwork_requested.contains(&key)
                    && self.artwork_failed.get(&key).is_none_or(|failed| {
                        failed.attempts < MAX_COVER_ATTEMPTS && now >= failed.retry_at
                    })
            })
            .collect();
        let generation = self.artwork_generation;
        self.fetch_artwork_batch(to_fetch, generation)
    }

    pub(super) fn cache_artwork_handle(
        &mut self,
        url: String,
        max_dim: u32,
        handle: image::Handle,
    ) {
        self.artwork_cache.insert(url, max_dim, handle);
    }

    /// Starts one download per cover that is not already in flight. Each one
    /// has its own token, so it can be cancelled when it leaves the window.
    pub(super) fn fetch_artwork_batch(
        &mut self,
        requests: Vec<CoverRequest>,
        generation: u64,
    ) -> Task<Message> {
        let mut tasks = Vec::with_capacity(requests.len());
        for (url, max_dim) in requests {
            let key = (url.clone(), max_dim);
            if self.artwork_requested.contains(&key) {
                continue;
            }
            let token = CancellationToken::new();
            let request_id = self.next_artwork_request_id();
            self.artwork_batch_tokens
                .insert(key.clone(), (request_id, token.clone()));
            self.artwork_requested.insert(key);
            let message_url = url.clone();
            tasks.push(Task::perform(
                async move { fetch_artwork(url, token, max_dim).await },
                move |handle| Message::ArtworkLoaded {
                    url: message_url,
                    max_dim,
                    generation,
                    request_id,
                    handle,
                    selected: false,
                },
            ));
        }
        Task::batch(tasks)
    }

    /// Drops cached covers that `page` will not show (keeping the playing track's).
    pub(super) fn release_offscreen_artwork_for(&mut self, page: &Page) {
        let current = std::mem::replace(&mut self.page, page.clone());
        let mut keep: HashSet<String> = self
            .current_page_artwork()
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        self.page = current;
        keep.extend(self.playing_cover_url().map(str::to_owned));
        self.artwork_cache.retain_urls(|url| keep.contains(url));
    }

    /// Trailing throttle for artwork preload during scrolling. Each scroll event only marks a
    /// preload as pending; at most one 100 ms timer is in flight, so a fast scroll runs about
    /// ten preloads per second and one more when it stops.
    pub(super) fn scroll_preload_task(&mut self) -> Task<Message> {
        self.scroll_preload_pending = true;
        if self.scroll_preload_timer {
            return Task::none();
        }
        self.scroll_preload_timer = true;
        Task::perform(
            async { tokio::time::sleep(Duration::from_millis(100)).await },
            |()| Message::ScrollPreloadDue,
        )
    }

    /// Wakes the page when the earliest failed cover may be asked for again.
    fn schedule_cover_retry(&mut self) -> Task<Message> {
        if self.artwork_retry_pending {
            return Task::none();
        }
        let now = std::time::Instant::now();
        let next = self
            .artwork_failed
            .values()
            .filter(|failed| failed.attempts < MAX_COVER_ATTEMPTS && failed.retry_at > now)
            .map(|failed| failed.retry_at)
            .min();
        let Some(next) = next else {
            return Task::none();
        };
        self.artwork_retry_pending = true;
        let delay = next.saturating_duration_since(now);
        Task::perform(async move { tokio::time::sleep(delay).await }, |()| {
            Message::ArtworkRetryDue
        })
    }

    /// Records a list cover that did not download and schedules another try.
    fn cover_failed(&mut self, key: CoverRequest) -> Task<Message> {
        let now = std::time::Instant::now();
        let failed = self.artwork_failed.entry(key).or_insert(FailedCover {
            retry_at: now,
            attempts: 0,
        });
        failed.attempts = failed.attempts.saturating_add(1);
        failed.retry_at = now + COVER_RETRY_COOLDOWN;
        self.schedule_cover_retry()
    }

    /// The player's cover gets one more attempt, shortly after the first failure.
    fn retry_selected_artwork(
        &mut self,
        url: String,
        max_dim: u32,
        generation: u64,
    ) -> Task<Message> {
        if std::mem::replace(&mut self.artwork_selected_retried, true) {
            return Task::none();
        }
        let cancellation = CancellationToken::new();
        let request_id = self.next_artwork_request_id();
        self.artwork_selected_request_id = request_id;
        self.artwork_cancel = Some(cancellation.clone());
        let fetch_url = url.clone();
        Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => None,
                    _ = tokio::time::sleep(Duration::from_millis(700)) => {
                        fetch_player_artwork(fetch_url, cancellation, max_dim).await
                    }
                }
            },
            move |handle| Message::ArtworkLoaded {
                url,
                max_dim,
                generation,
                request_id,
                handle,
                selected: true,
            },
        )
    }

    /// The artwork messages of `update_inner`, which routes them here.
    pub(super) fn update_artwork(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ScrollPreloadDue => {
                self.scroll_preload_timer = false;
                if !std::mem::take(&mut self.scroll_preload_pending) {
                    return Task::none();
                }
                return self.preload_current_page_artwork();
            }
            Message::ArtworkRetryDue => {
                self.artwork_retry_pending = false;
                let reload = self.preload_current_page_artwork();
                return Task::batch([reload, self.schedule_cover_retry()]);
            }
            Message::ArtworkLoaded {
                url,
                max_dim,
                generation,
                request_id,
                handle,
                selected,
            } => {
                let key = (url.clone(), max_dim);
                if selected {
                    if request_id != self.artwork_selected_request_id {
                        return Task::none();
                    }
                    self.artwork_cancel = None;
                } else {
                    let current_request = self
                        .artwork_batch_tokens
                        .get(&key)
                        .is_some_and(|(active_id, _)| *active_id == request_id);
                    if generation != self.artwork_generation || !current_request {
                        return Task::none();
                    }
                    self.artwork_requested.remove(&key);
                    self.artwork_batch_tokens.remove(&key);
                }
                let Some(handle) = handle else {
                    return if selected {
                        self.retry_selected_artwork(url, max_dim, generation)
                    } else {
                        self.cover_failed(key)
                    };
                };
                self.artwork_failed.remove(&key);
                if !self.preferences.reduced_motion {
                    self.fade_until = Some(std::time::Instant::now() + shell::COVER_FADE);
                }
                // Only the player's own request replaces the cover on screen. A list
                // cover of the same track is smaller and must not overwrite it.
                if selected && self.playing_cover_url() == Some(url.as_str()) {
                    self.artwork = Some(handle.clone());
                }
                self.cache_artwork_handle(url, max_dim, handle);
            }
            Message::FadeFrame(now) => {
                if self.fade_until.is_some_and(|until| now >= until) {
                    self.fade_until = None;
                }
            }
            _ => unreachable!("update_inner routes only this group here"),
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{BrowseTarget, Collection, CollectionTrack, Track};
    use iced::widget::image;

    fn track(id: &str) -> Track {
        Track {
            duration_secs: None,
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: Some(format!("row-{id}")),
        }
    }

    /// A playlist with a header cover and `rows` tracks, each with its own cover.
    fn playlist(rows: usize) -> Arc<Collection> {
        Arc::new(Collection {
            title: "Long playlist".into(),
            subtitle: None,
            description: None,
            owner: None,
            thumbnail_url: Some("hero".into()),
            summary: None,
            tracks: (0..rows)
                .map(|index| CollectionTrack {
                    track: track(&index.to_string()),
                    duration: None,
                    references: Vec::new(),
                })
                .collect(),
            continuation: None,
            unavailable_tracks: 0,
            related: Vec::new(),
        })
    }

    fn playlist_app(rows: usize) -> App {
        let mut app = App {
            page: Page::CollectionDetail(BrowseTarget {
                browse_id: "VLlong".into(),
                params: None,
                page_type: None,
            }),
            area: iced::Size::new(1270.0, 720.0),
            ..App::default()
        };
        app.collection.data = Some(playlist(rows));
        app
    }

    fn cover() -> image::Handle {
        image::Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    #[test]
    fn clearing_the_artwork_cache_requests_the_playing_cover_again() {
        let mut app = App::default();
        app.playback_status.track = Some(track("now"));
        drop(app.update(Message::ClearArtworkCache));
        // Loading the cover starts a request for it, which is what keeps it from staying blank.
        assert!(app.artwork_cancel.is_some());
    }

    #[test]
    fn playlist_header_cover_is_requested_on_scroll() {
        let mut app = playlist_app(200);
        drop(app.update(Message::CollectionScrolled(0.0)));
        drop(app.update(Message::ScrollPreloadDue));
        // The header is drawn at the card size, so its key carries that variant.
        assert!(
            app.artwork_requested
                .contains(&("hero".into(), crate::artwork::CARD_MAX)),
            "the header cover must stay requested while the rows are reconciled"
        );
    }

    #[test]
    fn scrolling_a_playlist_keeps_in_flight_row_covers() {
        let mut app = playlist_app(200);
        drop(app.update(Message::CollectionScrolled(0.0)));
        drop(app.update(Message::ScrollPreloadDue));
        let key = ("row-0".to_owned(), crate::artwork::THUMB_MAX);
        let (_, first_token) = app
            .artwork_batch_tokens
            .get(&key)
            .cloned()
            .expect("the first visible row is requested");
        // A second scroll event at the same place must not restart the download.
        drop(app.update(Message::CollectionScrolled(0.0)));
        drop(app.update(Message::ScrollPreloadDue));
        assert!(!first_token.is_cancelled());
    }

    #[test]
    fn queue_covers_follow_the_rows_the_list_renders() {
        let app = App {
            page: Page::NowPlaying,
            area: iced::Size::new(1270.0, 1440.0),
            queue: (0..200)
                .map(|index| track(&index.to_string()))
                .collect::<Vec<_>>()
                .into(),
            queue_scroll: AbsoluteOffset {
                x: 0.0,
                y: 81.0 * 60.0,
            },
            ..App::default()
        };
        let urls: Vec<String> = app
            .current_page_artwork()
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        // Rows that fit on screen at this scroll position and height, without overscan.
        let visible = crate::ui::layout::virtual_range(
            app.queue.len(),
            app.queue_scroll.y,
            app.area.height - 24.0,
            81.0,
            0,
        );
        assert!(!visible.is_empty());
        for index in visible {
            assert!(
                urls.contains(&format!("row-{index}")),
                "row {index} is on screen but has no cover request"
            );
        }
    }

    #[test]
    fn a_failed_cover_is_asked_for_again_after_its_cooldown_then_waits_for_the_next_visit() {
        let mut app = App::default();
        let key = ("flaky".to_owned(), crate::artwork::THUMB_MAX);
        let requests = vec![key.clone()];
        for attempt in 1..=3 {
            drop(app.reconcile_artwork(requests.clone()));
            assert!(
                app.artwork_requested.contains(&key),
                "attempt {attempt} is requested"
            );
            let (request_id, _) = app.artwork_batch_tokens[&key].clone();
            drop(app.update(Message::ArtworkLoaded {
                url: "flaky".into(),
                max_dim: crate::artwork::THUMB_MAX,
                generation: app.artwork_generation,
                request_id,
                handle: None,
                selected: false,
            }));
            // Nothing is in flight, and the cooldown keeps the cover out of the next reconcile.
            assert!(!app.artwork_requested.contains(&key));
            drop(app.reconcile_artwork(requests.clone()));
            assert!(
                !app.artwork_requested.contains(&key),
                "cooldown still running"
            );
            // Pretend the cooldown has passed.
            if let Some(failed) = app.artwork_failed.get_mut(&key) {
                failed.retry_at = std::time::Instant::now();
            }
        }
        drop(app.reconcile_artwork(requests));
        assert!(
            !app.artwork_requested.contains(&key),
            "after the last attempt the cover waits for the next visit"
        );
    }

    #[test]
    fn player_cover_shows_a_cached_variant_while_the_large_one_loads() {
        let mut app = App::default();
        app.cache_artwork_handle(
            "track".into(),
            crate::artwork::THUMB_MAX,
            image::Handle::from_rgba(128, 128, vec![0; 128 * 128 * 4]),
        );
        drop(app.load_artwork(Some("track".into())));
        assert!(
            app.artwork.is_some(),
            "the player shows the best cached cover at once"
        );
        let _ = cover();
    }
}
