//! Artwork requests, the artwork cache and the cover fades.

use super::*;

impl App {
    pub(super) fn load_artwork(&mut self, url: Option<String>) -> Task<Message> {
        if let Some(previous) = self.artwork_cancel.take() {
            previous.cancel();
        }
        let request_id = self.next_artwork_request_id();
        self.artwork_selected_request_id = request_id;
        self.artwork = None;
        let Some(url) = url else {
            return Task::none();
        };
        if let Some(cached) = self
            .artwork_cache
            .get_exact(&url, crate::artwork::LARGE_MAX)
        {
            self.artwork = Some(cached.clone());
            return Task::none();
        }
        let cancellation = CancellationToken::new();
        self.artwork_cancel = Some(cancellation.clone());
        let request_url = url.clone();
        let max_dim = crate::artwork::LARGE_MAX;
        Task::perform(
            async move { fetch_artwork(request_url, cancellation, max_dim).await },
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
        let (urls, max_dim) = self.current_page_artwork_urls();
        self.preload_artwork_urls_sized(urls, max_dim)
    }

    /// Artwork the current page shows, and the variant it draws it at.
    pub(super) fn current_page_artwork_urls(&self) -> (Vec<String>, u32) {
        match &self.page {
            Page::Home => {
                let layout = home::HomeLayout::new(self.area, self.home_picks.items.len());
                (
                    self.home_picks
                        .items
                        .iter()
                        .filter_map(|item| item.thumbnail_url.clone())
                        .collect(),
                    crate::artwork::variant_for_pixels(layout.card * self.pixel_scale()),
                )
            }
            Page::Search => (
                self.catalog_results
                    .iter()
                    .take(40)
                    .filter_map(|item| item.thumbnail_url.clone())
                    .collect(),
                crate::artwork::CARD_MID_MAX,
            ),
            Page::Library => {
                let pitch = favorites::favorites_row_pitch(self.preferences.ui_density);
                let range = layout::virtual_range(
                    self.favorites.len(),
                    self.library_scroll.y,
                    self.area.height,
                    pitch,
                    8,
                );
                (
                    self.favorites
                        .iter()
                        .skip(range.start)
                        .take(range.len())
                        .filter_map(|track| track.thumbnail_url.clone())
                        .collect(),
                    crate::artwork::THUMB_MAX,
                )
            }
            Page::NowPlaying => {
                let start = ((self.queue_scroll.y.max(0.0) / 81.0) as usize).saturating_sub(4);
                (
                    self.queue
                        .iter()
                        .skip(start)
                        .take(16)
                        .filter_map(|track| track.thumbnail_url.clone())
                        .collect(),
                    crate::artwork::THUMB_MAX,
                )
            }
            Page::CollectionDetail(_) => {
                let start = (self.collection_scroll / layout::COLLECTION_ROW_PITCH) as usize;
                let rows = self
                    .collection
                    .data
                    .as_ref()
                    .map(|data| {
                        data.tracks
                            .iter()
                            .skip(start.saturating_sub(4))
                            .take(24)
                            .filter_map(|entry| entry.track.thumbnail_url.clone())
                            .chain(data.thumbnail_url.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                (rows, crate::artwork::THUMB_MAX)
            }
            _ => (Vec::new(), crate::artwork::CARD_MAX),
        }
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

    pub(super) fn cancel_artwork_batch(&mut self) {
        for (_, (_, token)) in self.artwork_batch_tokens.drain() {
            token.cancel();
        }
        self.artwork_requested.clear();
        self.artwork_last_wanted.clear();
    }

    pub(super) fn next_artwork_request_id(&mut self) -> u64 {
        self.artwork_request_id = self.artwork_request_id.wrapping_add(1);
        self.artwork_request_id
    }

    pub(super) fn preload_artwork_urls_sized(
        &mut self,
        urls: Vec<String>,
        max_dim: u32,
    ) -> Task<Message> {
        // Reconcile by (URL, variant). Keep in-flight work that is
        // still wanted, cancel only what left the window, and never bump the
        // generation on scroll (generation changes only on navigation/clear).
        // The byte budget bounds resident pixels via LRU; the queue itself is
        // capped by entry count (visible-first order) so visible covers are
        // never permanently dropped by `take(capacity)`.
        let mut seen = HashSet::new();
        let ordered: Vec<String> = urls
            .into_iter()
            .filter(|url| seen.insert(url.clone()))
            .take(crate::artwork::MAX_ARTWORK_ENTRIES)
            .collect();
        // Adapt variant so the whole window fits the byte budget when possible.
        let effective_dim = Self::effective_variant_for_count(max_dim, ordered.len().max(1));
        let wanted: HashSet<(String, u32)> = ordered
            .iter()
            .cloned()
            .map(|url| (url, effective_dim))
            .collect();
        // Coalesce: identical window, nothing new to do.
        if wanted == self.artwork_last_wanted {
            // Still touch to keep MRU order.
            for (url, dim) in &wanted {
                self.artwork_cache.touch(url, *dim);
            }
            return Task::none();
        }
        // Cancel only work that left the window of interest.
        let stale: Vec<(String, u32)> = self
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
        self.artwork_last_wanted = wanted.clone();
        // Keep Now Playing resident when possible.
        if let Some(url) = self
            .playback_status
            .track
            .as_ref()
            .and_then(|t| t.thumbnail_url.clone())
        {
            self.artwork_cache.touch(&url, crate::artwork::LARGE_MAX);
        }
        for (url, dim) in &wanted {
            self.artwork_cache.touch(url, *dim);
        }
        let generation = self.artwork_generation;
        let to_fetch: Vec<String> = ordered
            .into_iter()
            .filter(|url| {
                !self.artwork_cache.contains_variant(url, effective_dim)
                    && !self
                        .artwork_requested
                        .contains(&(url.clone(), effective_dim))
            })
            .collect();
        self.fetch_artwork_batch(to_fetch, effective_dim, generation)
    }

    pub(super) fn cache_artwork_handle(
        &mut self,
        url: String,
        max_dim: u32,
        handle: image::Handle,
    ) {
        self.artwork_cache.insert(url, max_dim, handle);
    }

    pub(super) fn fetch_artwork_batch(
        &mut self,
        urls: Vec<String>,
        max_dim: u32,
        generation: u64,
    ) -> Task<Message> {
        if urls.is_empty() {
            return Task::none();
        }
        // One token per (URL, variant) so leaving URLs can be cancelled
        // without aborting still-useful work. Each fetch is an independent
        // Task; Task::batch keeps them alive even across scroll events.
        let mut tasks = Vec::with_capacity(urls.len());
        for url in urls {
            let token = CancellationToken::new();
            let key = (url.clone(), max_dim);
            // Skip if already in flight (reconciliation guarantees this, but
            // keep the guard for direct callers).
            if self.artwork_requested.contains(&key) {
                continue;
            }
            let request_id = self.next_artwork_request_id();
            self.artwork_batch_tokens
                .insert(key.clone(), (request_id, token.clone()));
            self.artwork_requested.insert(key);
            let fetch_url = url.clone();
            let message_url = url;
            tasks.push(Task::perform(
                async move { fetch_artwork(fetch_url, token, max_dim).await },
                move |handle| Message::ArtworkLoaded {
                    url: message_url.clone(),
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
        let (mut keep, _) = self.current_page_artwork_urls();
        self.page = current;
        keep.extend(
            self.playback_status
                .track
                .as_ref()
                .and_then(|track| track.thumbnail_url.clone()),
        );
        let keep: HashSet<String> = keep.into_iter().collect();
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
                if selected {
                    self.artwork_cancel = None;
                }
                if handle.is_none() && self.artwork_retried.insert(url.clone()) {
                    let retry_url = url.clone();
                    let retry_cancellation = CancellationToken::new();
                    let retry_request_id = self.next_artwork_request_id();
                    if selected {
                        self.artwork_selected_request_id = retry_request_id;
                        self.artwork_cancel = Some(retry_cancellation.clone());
                    } else {
                        self.artwork_batch_tokens
                            .insert(key.clone(), (retry_request_id, retry_cancellation.clone()));
                        self.artwork_requested.insert(key);
                    }
                    return Task::perform(
                        async move {
                            tokio::select! {
                                _ = retry_cancellation.cancelled() => None,
                                _ = tokio::time::sleep(Duration::from_millis(700)) => {
                                    fetch_artwork(retry_url.clone(), retry_cancellation, max_dim)
                                        .await
                                }
                            }
                        },
                        move |handle| Message::ArtworkLoaded {
                            url,
                            max_dim,
                            generation,
                            request_id: retry_request_id,
                            handle,
                            selected,
                        },
                    );
                }
                if let Some(handle) = handle {
                    self.artwork_retried.remove(&url);
                    if !self.preferences.reduced_motion {
                        self.fade_until = Some(std::time::Instant::now() + shell::COVER_FADE);
                    }
                    self.cache_artwork_handle(url.clone(), max_dim, handle.clone());
                    if self
                        .playback_status
                        .track
                        .as_ref()
                        .and_then(|track| track.thumbnail_url.as_ref())
                        == Some(&url)
                    {
                        self.artwork = Some(handle);
                    }
                }
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
