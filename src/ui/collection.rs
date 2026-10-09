use super::shell::fit;
use super::theme::{self, DIM, FADED, INK, MONO, RULE, hrule, icon, label};
use super::{App, Message, format_duration};
use iced::widget::{
    Space, button, column, container,
    operation::{self, AbsoluteOffset},
    row, scrollable, text,
};
use iced::{Alignment, Element, Length, Size, Task};
use std::{collections::HashSet, sync::Arc};
use tokio_util::sync::CancellationToken;

use crate::music::{BrowsePage, BrowseTarget, Collection};

/// One independent collection request. Every exit/retry invalidates its generation.
#[derive(Default)]
pub(super) struct CollectionState {
    pub target: Option<BrowseTarget>,
    pub data: Option<Arc<Collection>>,
    pub loading: bool,
    pub more_loading: bool,
    pub more_error: Option<String>,
    pub error: Option<String>,
    pub generation: u64,
    pub cancellation: Option<CancellationToken>,
    pub tracks_context: Arc<[crate::music::Track]>,
}

impl CollectionState {
    pub fn cancel(&mut self) {
        if let Some(token) = self.cancellation.take() {
            token.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.more_loading = false;
        self.more_error = None;
    }

    pub fn begin(&mut self, target: BrowseTarget) -> (u64, CancellationToken) {
        // Same-target reloads keep stale data visible underneath the
        // skeleton instead of flashing an empty loading page.
        let same_target = self.target.as_ref() == Some(&target);
        self.cancel();
        self.target = Some(target);
        if !same_target {
            self.data = None;
            self.tracks_context = Arc::from([]);
        }
        self.error = None;
        self.loading = true;
        self.more_loading = false;
        self.more_error = None;
        let token = CancellationToken::new();
        self.cancellation = Some(token.clone());
        (self.generation, token)
    }

    /// Starts applying a response for `generation`; false when it was superseded.
    fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation || !self.loading {
            return false;
        }
        self.loading = false;
        self.cancellation = None;
        true
    }

    /// Shows `data` without copying it: the cache and the page share one allocation.
    fn show(&mut self, data: Arc<Collection>) {
        self.tracks_context = data
            .tracks
            .iter()
            .map(|entry| entry.track.clone())
            .collect::<Vec<_>>()
            .into();
        self.data = Some(data);
    }

    pub fn complete(&mut self, generation: u64, result: Result<BrowsePage, String>) -> bool {
        if !self.finish(generation) {
            return false;
        }
        match result {
            Ok(BrowsePage::Collection(data)) => self.show(Arc::new(data)),
            Ok(BrowsePage::Unsupported { .. }) => {
                self.error = Some("This collection type is not supported yet.".into());
            }
            Err(error) => self.error = Some(error),
        }
        true
    }

    /// Completes with a collection already held by the cache.
    pub fn complete_cached(&mut self, generation: u64, cached: Arc<Collection>) -> bool {
        if !self.finish(generation) {
            return false;
        }
        self.show(cached);
        true
    }
}

impl App {
    pub(super) fn load_collection_more(&mut self) -> Task<Message> {
        if self.collection.more_loading || self.collection.more_error.is_some() {
            return Task::none();
        }
        if self
            .collection
            .data
            .as_ref()
            .is_some_and(|data| data.tracks.len() >= crate::music::MAX_COLLECTION_TRACKS)
        {
            if let Some(data) = self.collection.data.as_mut() {
                Arc::make_mut(data).continuation = None;
            }
            return Task::none();
        }
        let Some(continuation) = self
            .collection
            .data
            .as_ref()
            .and_then(|data| data.continuation.clone())
        else {
            return Task::none();
        };
        self.collection.more_loading = true;
        let generation = self.collection.generation;
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                service
                    .browse_more(&continuation)
                    .await
                    .map_err(|error| error.user_message())
            },
            move |result| Message::CollectionMoreLoaded { generation, result },
        )
    }

    pub(super) fn load_collection(&mut self, target: BrowseTarget) -> Task<Message> {
        let (generation, cancellation) = self.collection.begin(target.clone());
        if let Some((_, cached)) = self
            .collection_cache
            .iter()
            .find(|(key, _)| key == &target)
            .cloned()
        {
            cancellation.cancel();
            let _ = self.collection.complete_cached(generation, cached);
            return Task::batch([
                operation::scroll_to("collection-content", AbsoluteOffset::<f32>::default()),
                self.preload_collection_artwork(0),
            ]);
        }
        let service = Arc::clone(&self.music_service);
        let request = Task::perform(
            async move {
                tokio::select! {
                    _ = cancellation.cancelled() => Err("Request cancelled.".into()),
                    result = service.browse(&target) => result.map_err(|error| error.user_message()),
                }
            },
            move |result| Message::CollectionLoaded { generation, result },
        );
        Task::batch([
            operation::scroll_to("collection-content", AbsoluteOffset::<f32>::default()),
            request,
        ])
    }

    /// Y offset of the first track row inside the collection scrollable.
    /// The header sits outside the scrollable, so rows start at the top.
    pub(super) fn collection_rows_offset(&self) -> f32 {
        0.0
    }

    pub(super) fn preload_collection_artwork(&mut self, row: usize) -> Task<Message> {
        let Some(data) = &self.collection.data else {
            return Task::none();
        };
        let start = (row / 6 * 6).saturating_sub(12);
        // The hero is drawn at CARD_MAX and the rows at THUMB_MAX; preload each at its own size.
        let hero: Vec<String> = data.thumbnail_url.iter().cloned().collect();
        let rows: HashSet<_> = data
            .tracks
            .iter()
            .skip(start)
            .take(36)
            .filter_map(|entry| entry.track.thumbnail_url.clone())
            .collect();
        Task::batch([
            self.preload_artwork_urls_sized(hero, crate::artwork::CARD_MAX),
            self.preload_artwork_urls_sized(rows.into_iter().collect(), crate::artwork::THUMB_MAX),
        ])
    }

    pub(super) fn collection_content(&self, area: Size) -> Element<'_, Message> {
        let width = area.width.max(240.0);
        if let Some(error) = &self.collection.error {
            return column![
                label(self.tr("[ COULD NOT LOAD ]").into_owned(), 14.0),
                label(fit(error, width, 12.0), 12.0).color(DIM),
                button(label(self.tr("Retry").into_owned(), 13.0))
                    .on_press(Message::RetryCollection)
                    .padding([8, 18])
                    .style(theme::outline_button),
            ]
            .spacing(10)
            .into();
        }
        let Some(data) = &self.collection.data else {
            return label(self.tr("[ LOADING ]").into_owned(), 14.0)
                .color(DIM)
                .into();
        };
        let queue_tracks = Arc::clone(&self.collection.tracks_context);
        let queue_source = super::QueueSource {
            title: data.title.clone(),
            track_count: data.tracks.len(),
        };
        let text_width = width - 160.0 - 32.0;
        let mut metadata =
            column![text(fit(&data.title, text_width, 34.0)).size(34).font(MONO),].spacing(6);
        if let Some(subtitle) = data.subtitle.as_deref().or(data.owner.as_deref()) {
            metadata = metadata.push(label(fit(subtitle, text_width, 16.0), 16.0));
        }
        if let Some(summary) = &data.summary {
            metadata = metadata.push(label(fit(summary, text_width, 12.0), 12.0).color(DIM));
        }
        let has_tracks = !queue_tracks.is_empty();
        metadata = metadata.push(Space::new().height(8)).push(
            row![
                button(
                    row![
                        icon("play", 14.0, INK),
                        label(self.tr("Play").into_owned(), 14.0)
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                )
                .on_press_maybe(has_tracks.then(|| {
                    Message::PlayCollection(Arc::clone(&queue_tracks), 0, queue_source.clone())
                }))
                .padding([10, 22])
                .style(theme::outline_button),
                button(
                    row![
                        icon("shuffle", 14.0, INK),
                        label(self.tr("Shuffle").into_owned(), 14.0)
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                )
                .on_press_maybe(
                    has_tracks.then(|| Message::ShuffleContext(Arc::clone(&queue_tracks)))
                )
                .padding([10, 22])
                .style(theme::outline_button),
            ]
            .spacing(16),
        );
        let header = row![
            self.cover(
                data.thumbnail_url.as_deref(),
                160.0,
                crate::artwork::CARD_MAX
            ),
            metadata.width(Length::Fill),
        ]
        .spacing(32)
        .align_y(Alignment::End);

        let duration_w = 96.0;
        let index_w = 48.0;
        let cover = 44.0;
        let title_w = width - index_w - cover - duration_w - 3.0 * 16.0 - 8.0;
        let table_head = row![
            container(label(self.tr("#").into_owned(), 11.0).color(DIM)).width(index_w),
            container(label(self.tr("TITLE").into_owned(), 11.0).color(DIM)).width(Length::Fill),
            container(label(self.tr("DURATION").into_owned(), 11.0).color(DIM)).width(duration_w),
        ]
        .spacing(16)
        .padding([0, 4]);
        let mut page = column![
            Space::new().height(8),
            header,
            Space::new().height(28),
            table_head,
            Space::new().height(10),
            hrule(INK),
        ];
        if self.collection.loading {
            page = page.push(label(self.tr("Updating…").into_owned(), 11.0).color(FADED));
        }
        if data.tracks.is_empty() {
            page = page.push(
                container(
                    label(
                        self.tr("No playable songs in this collection.")
                            .into_owned(),
                        13.0,
                    )
                    .color(DIM),
                )
                .padding([20, 4]),
            );
            return page.into();
        }
        let pitch = super::layout::COLLECTION_ROW_PITCH;
        let range = super::layout::virtual_range(
            data.tracks.len(),
            self.collection_scroll,
            area.height,
            pitch,
            8,
        );
        let mut rows = column![].spacing(0).width(Length::Fill);
        if range.start > 0 {
            rows = rows.push(
                Space::new().height(super::layout::virtual_spacer_height(range.start, pitch)),
            );
        }
        let playing = self
            .playback_status
            .track
            .as_ref()
            .map(|track| track.video_id.as_str());
        for index in range.clone() {
            let entry = &data.tracks[index];
            let track = &entry.track;
            let current = playing == Some(track.video_id.as_str());
            let duration = entry
                .duration
                .map(format_duration)
                .unwrap_or_else(|| "—".into());
            let cells = row![
                container(label(
                    if current {
                        "▶".to_owned()
                    } else {
                        (index + 1).to_string()
                    },
                    14.0
                ))
                .width(index_w),
                self.cover(
                    track.thumbnail_url.as_deref(),
                    cover,
                    crate::artwork::THUMB_MAX
                ),
                column![
                    label(fit(&track.title, title_w, 14.0), 14.0),
                    label(fit(&track.artist, title_w, 12.0), 12.0).color(DIM),
                ]
                .spacing(4)
                .width(Length::Fill),
                container(label(duration, 14.0)).width(duration_w),
            ]
            .spacing(16)
            .align_y(Alignment::Center);
            rows = rows.push(
                column![
                    button(cells)
                        .on_press(Message::PlayCollection(
                            Arc::clone(&queue_tracks),
                            index,
                            queue_source.clone(),
                        ))
                        .width(Length::Fill)
                        .padding([10, 4])
                        .style(if current {
                            theme::row_button_current
                        } else {
                            theme::row_button
                        }),
                    hrule(RULE),
                ]
                .height(pitch),
            );
        }
        let remaining = data.tracks.len().saturating_sub(range.end);
        if remaining > 0 {
            rows = rows
                .push(Space::new().height(super::layout::virtual_spacer_height(remaining, pitch)));
        }
        if data.continuation.is_some() {
            rows = rows.push(
                container(if self.collection.more_error.is_some() {
                    Element::from(
                        button(label(self.tr("Retry loading more").into_owned(), 12.0))
                            .on_press(Message::RetryCollectionMore)
                            .padding([6, 12])
                            .style(theme::outline_button),
                    )
                } else {
                    label(self.tr("Loading more songs…").into_owned(), 12.0)
                        .color(DIM)
                        .into()
                })
                .padding([12, 4]),
            );
        }
        page.push(
            scrollable(rows)
                .id("collection-content")
                .on_scroll(|viewport| Message::CollectionScrolled(viewport.absolute_offset().y))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::{CollectionTrack, Track};

    fn target(id: &str) -> BrowseTarget {
        BrowseTarget {
            browse_id: id.into(),
            params: None,
            page_type: None,
        }
    }

    fn track(id: &str) -> Track {
        Track {
            duration_secs: None,
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            thumbnail_url: None,
        }
    }

    fn entry(id: &str) -> CollectionTrack {
        CollectionTrack {
            track: track(id),
            duration: None,
            references: Vec::new(),
        }
    }

    fn collection_with(
        tracks: Vec<CollectionTrack>,
        continuation: Option<String>,
    ) -> Arc<Collection> {
        Arc::new(Collection {
            title: "Fixture".into(),
            subtitle: None,
            description: None,
            owner: None,
            thumbnail_url: None,
            summary: None,
            tracks,
            continuation: continuation.map(|token| crate::music::Continuation { token }),
            unavailable_tracks: 0,
            related: Vec::new(),
        })
    }

    #[test]
    fn replacing_request_cancels_it_and_rejects_its_response() {
        let mut state = CollectionState::default();
        let (old, token) = state.begin(target("VLfirst"));
        let (current, _) = state.begin(target("VLsecond"));
        assert!(token.is_cancelled());
        assert!(!state.complete(old, Err("obsolete error".into())));
        assert!(state.loading);
        assert!(state.error.is_none());
        assert!(state.complete(current, Err("current error".into())));
        assert_eq!(state.error.as_deref(), Some("current error"));
    }

    #[test]
    fn leaving_detail_invalidates_even_an_already_queued_response() {
        let mut state = CollectionState::default();
        let (generation, token) = state.begin(target("VLfirst"));
        state.cancel();
        assert!(token.is_cancelled());
        assert!(!state.complete(generation, Err("late response".into())));
        assert!(state.error.is_none());
    }

    #[test]
    fn collection_more_dedupes_tracks_and_detects_real_end() {
        use crate::music::BrowseMore;
        let mut app = App::default();
        app.collection.data = Some(collection_with(
            vec![entry("a"), entry("b")],
            Some("tok-1".into()),
        ));
        app.collection.generation = 7;
        let generation = app.collection.generation;
        // Duplicate b + new c: dedup keeps a,b,c.
        let _ = app.update(Message::CollectionMoreLoaded {
            generation,
            result: Ok(BrowseMore {
                tracks: vec![entry("b"), entry("c")],
                continuation: Some(crate::music::Continuation {
                    token: "tok-2".into(),
                }),
            }),
        });
        let ids: Vec<_> = app
            .collection
            .data
            .as_ref()
            .unwrap()
            .tracks
            .iter()
            .map(|e| e.track.video_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert!(!app.collection.more_loading);
        assert!(app.collection.more_error.is_none());
        // No new tracks + same token: real end, continuation cleared.
        let before = app.collection.data.as_ref().unwrap().tracks.len();
        let _ = app.update(Message::CollectionMoreLoaded {
            generation,
            result: Ok(BrowseMore {
                tracks: vec![entry("b"), entry("c")],
                continuation: Some(crate::music::Continuation {
                    token: "tok-2".into(),
                }),
            }),
        });
        assert_eq!(app.collection.data.as_ref().unwrap().tracks.len(), before);
        assert!(app.collection.data.as_ref().unwrap().continuation.is_none());
    }

    #[test]
    fn collection_more_retry_clears_error_and_scroll_stores_position() {
        let mut app = App::default();
        app.collection.data = Some(collection_with(vec![entry("a")], Some("tok-1".into())));
        app.collection.more_error = Some("offline".into());
        let _ = app.update(Message::RetryCollectionMore);
        assert!(app.collection.more_error.is_none());
        let _ = app.update(Message::CollectionScrolled(320.0));
        assert_eq!(app.collection_scroll, 320.0);
    }
}
