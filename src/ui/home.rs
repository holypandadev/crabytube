//! Home: the search field, the "All-time essentials" carousel and search results.

use super::shell::{fit, section_heading};
use super::theme::{self, DIM, FADED, INK, MONO, RULE, hrule, icon, label};
use super::{App, Message};
use crate::music::{HomeItem, HomeItemKind, Track};
use iced::widget::{
    Space, button, column, container, pin, row, scrollable, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Size};
use std::sync::Arc;

pub(super) const CAROUSEL_ID: &str = "essentials";
/// Card gaps: wide windows and phone-sized ones.
const CARD_GAP: f32 = 24.0;
const CARD_GAP_NARROW: f32 = 14.0;
/// Below this content width the page switches to its phone layout.
pub(super) const NARROW_W: f32 = 600.0;
/// Smallest and largest card edge.
const CARD_MIN: f32 = 104.0;
const CARD_MAX: f32 = 400.0;
/// Share of the page height the search field and carousel aim to fill.
const BLOCK_SHARE: f32 = 0.74;
const SEARCH_H: f32 = 54.0;
const SEARCH_MAX_W: f32 = 640.0;
/// Heading row, its gap, and the two caption lines under each cover.
const CAROUSEL_CHROME_H: f32 = 24.0 + 18.0 + 10.0 + 20.0 + 18.0;
const CAROUSEL_DURATION_MS: f32 = 320.0;
/// YouTube Music's "All-time essentials" shelf in each catalog language the app requests.
const ESSENTIALS_TITLES: &[&str] = &["all-time essentials", "esenciales de todos los tiempos"];
const MAX_ITEMS: usize = 30;

/// The one Home shelf: "All-time essentials", or the first shelf of
/// collections when the feed does not include it.
#[derive(Default)]
pub(super) struct HomePicks {
    pub title: String,
    pub items: Vec<HomeItem>,
    /// Song items as a play queue, in shelf order.
    pub queue: Arc<[Track]>,
}

impl HomePicks {
    pub fn rebuild(&mut self, shelves: &[crate::music::HomeShelf]) {
        let shelf = shelves
            .iter()
            .find(|shelf| ESSENTIALS_TITLES.contains(&shelf.title.trim().to_lowercase().as_str()))
            .or_else(|| {
                shelves.iter().find(|shelf| {
                    shelf.items.iter().any(|item| {
                        item.browse_target
                            .as_ref()
                            .is_some_and(|target| target.is_public_collection())
                    })
                })
            });
        let Some(shelf) = shelf else {
            *self = Self::default();
            return;
        };
        self.title = shelf.title.clone();
        self.items = shelf
            .items
            .iter()
            .filter(|item| {
                item.track.is_some()
                    || item
                        .browse_target
                        .as_ref()
                        .is_some_and(|target| target.is_public_collection())
            })
            .take(MAX_ITEMS)
            .cloned()
            .collect();
        self.queue = self
            .items
            .iter()
            .filter_map(|item| item.track.clone())
            .collect();
    }
}

/// Smooth horizontal scrolling of the Home carousel.
#[derive(Default)]
pub(super) struct Carousel {
    pub offset: f32,
    from: f32,
    to: f32,
    started: Option<std::time::Instant>,
}

impl Carousel {
    pub fn animating(&self) -> bool {
        self.started.is_some()
    }

    /// Starts easing towards `to`; with reduced motion it jumps there.
    pub fn start(&mut self, to: f32, reduced_motion: bool) {
        if reduced_motion {
            self.offset = to;
            self.started = None;
            return;
        }
        self.from = self.offset;
        self.to = to;
        self.started = Some(std::time::Instant::now());
    }

    /// Advances the animation and returns the offset to scroll to.
    pub fn step(&mut self, now: std::time::Instant) -> f32 {
        let Some(started) = self.started else {
            return self.offset;
        };
        let elapsed_ms = now.saturating_duration_since(started).as_secs_f32() * 1000.0;
        let t = (elapsed_ms / CAROUSEL_DURATION_MS).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.offset = self.from + (self.to - self.from) * eased;
        if t >= 1.0 {
            self.offset = self.to;
            self.started = None;
        }
        self.offset
    }
}

/// Geometry of the Home page for a given content area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HomeLayout {
    /// Cards shown at once.
    pub visible: usize,
    pub card: f32,
    pub card_gap: f32,
    /// Width of the card row (and of the carousel block, which is centered).
    pub row_width: f32,
    pub search_width: f32,
    /// Space above the search field, and between it and the carousel.
    pub top: f32,
    pub gap: f32,
}

impl HomeLayout {
    /// The search field and the carousel form one block, centered on the
    /// page (a little above the middle). Cards grow with the height until the
    /// block fills about three quarters of the page, then as many as fit
    /// across share the width exactly, so neither axis is left mostly empty.
    pub fn new(area: Size, items: usize) -> Self {
        let width = area.width.max(CARD_MIN);
        let height = area.height.max(0.0);
        let items = items.clamp(1, 8) as f32;
        let narrow = width < NARROW_W;
        let card_gap = if narrow { CARD_GAP_NARROW } else { CARD_GAP };
        let gap = (height * 0.06).clamp(24.0, 56.0).floor();
        let fixed = SEARCH_H + gap + CAROUSEL_CHROME_H;
        // Largest card the height allows.
        let tallest = (height * BLOCK_SHARE - fixed).clamp(CARD_MIN, CARD_MAX);
        // As many cards as needed for that size to fill the width (at least
        // two on narrow screens, so the row reads as a carousel).
        let mut across = ((width + card_gap) / (tallest + card_gap)).ceil();
        if narrow {
            across = across.max(2.0);
        }
        across = across.clamp(1.0, items);
        let mut card = ((width - card_gap * (across - 1.0)) / across).min(tallest);
        while card < CARD_MIN && across > 1.0 {
            across -= 1.0;
            card = ((width - card_gap * (across - 1.0)) / across).min(tallest);
        }
        // Very short windows: shrink the cards so the block still fits.
        card = card
            .min((height - fixed - 32.0).max(CARD_MIN * 0.75))
            .floor();
        let block = fixed + card;
        let top = ((height - block) / 2.0 * 0.9).max(16.0).floor();
        Self {
            visible: across as usize,
            card,
            card_gap,
            row_width: across * (card + card_gap) - card_gap,
            search_width: width.min(SEARCH_MAX_W),
            top,
            gap,
        }
    }

    pub fn stride(&self) -> f32 {
        self.card + self.card_gap
    }

    /// Largest scroll offset for `count` cards.
    pub fn max_offset(&self, count: usize) -> f32 {
        (count as f32 * self.stride() - self.card_gap - self.row_width).max(0.0)
    }
}

/// `3:45` from a subtitle such as `Song • Artist • Album • 3:45`.
pub(super) fn subtitle_duration(subtitle: &str) -> Option<&str> {
    let last = subtitle.rsplit('•').next()?.trim();
    let (minutes, seconds) = last.rsplit_once(':')?;
    let valid = !minutes.is_empty()
        && minutes
            .split(':')
            .all(|part| part.chars().all(|c| c.is_ascii_digit()))
        && seconds.len() == 2
        && seconds.chars().all(|c| c.is_ascii_digit());
    valid.then_some(last)
}

impl App {
    /// The search field alone, without suggestions.
    fn search_field(&self, width: f32, hint: bool) -> Element<'_, Message> {
        container(
            row![
                icon("search", 20.0, INK),
                text_input(
                    self.tr_static("Search artist, album, tracks"),
                    &self.search_query
                )
                .id("search-field")
                .on_input(Message::SearchChanged)
                .on_submit(Message::SearchSubmitted)
                .font(MONO)
                .size(16)
                .padding([10, 4])
                .style(theme::input_style),
                if self.search_query.is_empty() {
                    // The shortcut hint only where there is room and a keyboard.
                    let hint = hint && width >= NARROW_W;
                    Element::from(label(if hint { "Ctrl+K" } else { "" }, 11.0).color(FADED))
                } else {
                    button(icon("close", 16.0, INK))
                        .on_press(Message::SearchChanged(String::new()))
                        .padding(4)
                        .style(theme::icon_button)
                        .into()
                },
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .width(width)
        .height(SEARCH_H)
        .padding([0, 18])
        .center_y(SEARCH_H)
        .style(theme::boxed)
        .into()
    }

    /// Suggestions drawn over the page, so typing never moves the layout.
    fn suggestions_overlay(&self, width: f32, x: f32, y: f32) -> Option<Element<'_, Message>> {
        if !self.show_suggestions || self.suggestions.is_empty() {
            return None;
        }
        let mut list = column![].spacing(0);
        for (index, suggestion) in self.suggestions.iter().enumerate() {
            let selected = self.suggestion_index == Some(index);
            list = list.push(
                button(label(fit(suggestion, width - 48.0, 14.0), 14.0))
                    .on_press(Message::SuggestionChosen(suggestion.clone()))
                    .width(Length::Fill)
                    .padding([8, 20])
                    .style(if selected {
                        theme::inverted_button
                    } else {
                        theme::row_button
                    }),
            );
        }
        Some(
            pin(container(list)
                .width(width)
                .padding(theme::hairline())
                .style(theme::boxed))
            .x(x)
            .y(y)
            .into(),
        )
    }

    /// Page content with the suggestions floating over it.
    fn with_suggestions<'a>(
        &'a self,
        base: Element<'a, Message>,
        width: f32,
        x: f32,
        y: f32,
    ) -> Element<'a, Message> {
        // Always a stack: swapping the tree's shape when suggestions appear
        // would rebuild the search field and drop its focus mid-word.
        let overlay = self
            .suggestions_overlay(width, x, y)
            .unwrap_or_else(|| Space::new().into());
        stack![base, overlay]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub(super) fn home_page(&self, area: Size) -> Element<'_, Message> {
        let layout = HomeLayout::new(area, self.home_picks.items.len());
        let search_x = ((area.width - layout.search_width) / 2.0).max(0.0);
        let carousel: Element<'_, Message> = if self.home_picks.items.is_empty() {
            let status: Element<'_, Message> = if self.home_loading {
                label(self.tr("[ LOADING YOUTUBE MUSIC ]").into_owned(), 13.0)
                    .color(DIM)
                    .into()
            } else if let Some(error) = &self.home_error {
                column![
                    label(self.tr("[ HOME COULD NOT LOAD ]").into_owned(), 13.0),
                    label(fit(error, area.width, 12.0), 12.0).color(DIM),
                    button(label(self.tr("Retry").into_owned(), 13.0))
                        .on_press(Message::ReloadHome)
                        .padding([8, 18])
                        .style(theme::outline_button),
                ]
                .spacing(10)
                .into()
            } else {
                label(self.tr("Search to start listening.").into_owned(), 13.0)
                    .color(DIM)
                    .into()
            };
            container(status).center_x(Length::Fill).into()
        } else {
            container(self.essentials(&layout))
                .width(Length::Fill)
                .center_x(Length::Fill)
                .into()
        };
        let base = column![
            Space::new().height(layout.top),
            container(self.search_field(layout.search_width, true)).center_x(Length::Fill),
            Space::new().height(layout.gap),
            carousel,
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        self.with_suggestions(
            base.into(),
            layout.search_width,
            search_x,
            layout.top + SEARCH_H,
        )
    }

    fn essentials(&self, layout: &HomeLayout) -> Element<'_, Message> {
        let count = self.home_picks.items.len();
        let max_offset = layout.max_offset(count);
        let offset = self.carousel.offset.clamp(0.0, max_offset);
        let can_back = offset > 1.0;
        let can_forward = offset < max_offset - 1.0;
        let arrows = row![
            button(icon("chev-left", 18.0, if can_back { INK } else { FADED }))
                .on_press_maybe(can_back.then_some(Message::CarouselStep { forward: false }))
                .padding(4)
                .style(theme::icon_button),
            button(icon(
                "chev-right",
                18.0,
                if can_forward { INK } else { FADED }
            ))
            .on_press_maybe(can_forward.then_some(Message::CarouselStep { forward: true }))
            .padding(4)
            .style(theme::icon_button),
        ]
        .spacing(6);
        let variant = crate::artwork::variant_for_pixels(layout.card * self.pixel_scale());
        let text_width = layout.card - 4.0;
        let mut cards = row![].spacing(layout.card_gap);
        for item in &self.home_picks.items {
            let action = match &item.track {
                Some(track) => self
                    .home_picks
                    .queue
                    .iter()
                    .position(|queued| queued.video_id == track.video_id)
                    .map(|index| Message::PlayContext(Arc::clone(&self.home_picks.queue), index)),
                None => item.browse_target.clone().map(Message::OpenCollection),
            };
            let Some(action) = action else {
                continue;
            };
            cards = cards.push(card(
                self.cover(item.thumbnail_url.as_deref(), layout.card, variant),
                fit(&item.title, text_width, 14.0),
                fit(&item.subtitle, text_width, 12.0),
                layout.card,
                action,
            ));
        }
        let strip = scrollable(cards)
            .id(CAROUSEL_ID)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::hidden(),
            ))
            .on_scroll(|viewport| Message::CarouselScrolled(viewport.absolute_offset().x))
            .width(layout.row_width)
            .height(layout.card + 10.0 + 20.0 + 18.0);
        column![
            section_heading(self.home_picks.title.clone(), Some(arrows.into())),
            Space::new().height(18),
            strip,
        ]
        .width(layout.row_width)
        .into()
    }

    pub(super) fn search_page(&self, area: Size) -> Element<'_, Message> {
        if !self.search_committed && self.catalog_results.is_empty() {
            return self.home_page(area);
        }
        let width = area.width.max(280.0);
        let search_width = area.width.min(SEARCH_MAX_W);
        let search_x = ((area.width - search_width) / 2.0).max(0.0);
        let query = self.search_query.trim();
        let mut content = column![].spacing(0);
        let count = self.catalog_results.len();
        content = content
            .push(text(self.tr("Search Results")).size(22).font(MONO))
            .push(
                label(
                    if self.search_loading {
                        self.tr("Searching…").into_owned()
                    } else {
                        self.tr_string(format!("{count} results for {query}"))
                    },
                    13.0,
                )
                .color(DIM),
            )
            .push(Space::new().height(10))
            .push(hrule(INK));
        if let Some(error) = &self.search_error {
            content = content.push(Space::new().height(16)).push(
                column![
                    label(self.tr("[ SEARCH FAILED ]").into_owned(), 13.0),
                    label(fit(error, width, 12.0), 12.0).color(DIM),
                    button(label(self.tr("Retry").into_owned(), 13.0))
                        .on_press(Message::SearchSubmitted)
                        .padding([8, 18])
                        .style(theme::outline_button),
                ]
                .spacing(10),
            );
        }
        if let Some(top) = self.catalog_results.first() {
            content = content
                .push(Space::new().height(18))
                .push(label(self.tr("Top Result").into_owned(), 15.0))
                .push(Space::new().height(8))
                .push(self.top_result(top, width));
        }
        let songs = self.search_songs();
        if !songs.is_empty() {
            let queue: Arc<[Track]> = songs.iter().filter_map(|item| item.track.clone()).collect();
            content = content
                .push(Space::new().height(22))
                .push(label(self.tr("Songs").into_owned(), 15.0))
                .push(Space::new().height(6))
                .push(hrule(INK));
            for (index, item) in songs.iter().enumerate() {
                if let Some(track) = &item.track {
                    let duration = item
                        .duration
                        .map(super::format_duration)
                        .or_else(|| subtitle_duration(&item.subtitle).map(str::to_owned));
                    content = content.push(self.song_row(
                        track,
                        duration,
                        Message::PlayContext(Arc::clone(&queue), index),
                        width,
                    ));
                }
            }
        }
        for (kind, title) in [
            (HomeItemKind::Album, "Albums"),
            (HomeItemKind::Playlist, "Playlists"),
            (HomeItemKind::Artist, "Artists"),
        ] {
            let items = self.search_collections(kind);
            if items.is_empty() {
                continue;
            }
            content = content
                .push(Space::new().height(22))
                .push(label(self.tr(title).into_owned(), 15.0))
                .push(Space::new().height(6))
                .push(hrule(INK))
                .push(Space::new().height(12))
                .push(self.collection_grid(&items, width));
        }
        content = content.push(Space::new().height(28));
        let base = column![
            Space::new().height(8),
            container(self.search_field(search_width, false)).center_x(Length::Fill),
            Space::new().height(24),
            scrollable(content)
                .id("search-content")
                .on_scroll(|viewport| Message::SearchScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        self.with_suggestions(base.into(), search_width, search_x, 8.0 + SEARCH_H)
    }

    /// Song results the results page lists under "Songs".
    pub(super) fn search_songs(&self) -> Vec<&HomeItem> {
        self.catalog_results
            .iter()
            .filter(|item| item.kind == HomeItemKind::Song && item.track.is_some())
            .take(8)
            .collect()
    }

    /// Collection results of one kind that the results page shows in its grid.
    pub(super) fn search_collections(&self, kind: HomeItemKind) -> Vec<&HomeItem> {
        self.catalog_results
            .iter()
            .filter(|item| {
                (item.kind == kind
                    || (kind == HomeItemKind::Playlist && item.kind == HomeItemKind::Mix))
                    && item
                        .browse_target
                        .as_ref()
                        .is_some_and(|target| target.is_public_collection())
            })
            .take(6)
            .collect()
    }

    fn top_result(&self, item: &HomeItem, width: f32) -> Element<'_, Message> {
        let action = item.track.clone().map(Message::SelectTrack).or_else(|| {
            item.browse_target
                .clone()
                .filter(|target| target.is_public_collection())
                .map(Message::OpenCollection)
        });
        let text_width = width - 96.0 - 64.0;
        let title = match &item.track {
            Some(track) => byline(&track.title, &track.artist),
            None => item.title.clone(),
        };
        container(
            row![
                self.cover(
                    item.thumbnail_url.as_deref(),
                    96.0,
                    crate::artwork::CARD_MID_MAX
                ),
                column![
                    text(fit(&title, text_width, 18.0)).size(18).font(MONO),
                    label(fit(&item.subtitle, text_width, 12.0), 12.0).color(DIM),
                    Space::new().height(6),
                    button(
                        row![
                            icon("play", 14.0, INK),
                            label(self.tr("Play").into_owned(), 13.0)
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center)
                    )
                    .on_press_maybe(action)
                    .padding([8, 18])
                    .style(theme::outline_button),
                ]
                .spacing(6),
            ]
            .spacing(20)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding(14)
        .style(theme::boxed)
        .into()
    }

    /// One song row: cover, "Title — Artist", duration and the ••• actions.
    pub(super) fn song_row<'a>(
        &'a self,
        track: &'a Track,
        duration: Option<String>,
        play: Message,
        width: f32,
    ) -> Element<'a, Message> {
        let open = self.row_menu.as_deref() == Some(track.video_id.as_str());
        let playing = self
            .playback_status
            .track
            .as_ref()
            .is_some_and(|current| current.video_id == track.video_id);
        let duration = duration.unwrap_or_default();
        let text_width = width - 36.0 - 16.0 - 60.0 - 44.0 - 24.0;
        let title = byline(&track.title, &track.artist);
        let more = button(icon("more", 18.0, INK))
            .on_press(Message::ToggleRowMenu(track.video_id.clone()))
            .padding(6)
            .style(if open {
                theme::icon_button_active
            } else {
                theme::icon_button
            });
        // The ••• button sits inside the row, so the hover fill covers the
        // whole line with even padding on both sides.
        let main = button(
            row![
                self.cover(
                    track.thumbnail_url.as_deref(),
                    36.0,
                    crate::artwork::THUMB_MAX
                ),
                text(fit(&title, text_width, 14.0))
                    .size(14)
                    .font(MONO)
                    .width(Length::Fill),
                label(if playing { "▶ " } else { "" }, 12.0),
                label(duration, 12.0).color(DIM),
                more,
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .on_press(play)
        .width(Length::Fill)
        .padding([6, 8])
        .style(theme::row_button);
        let mut block = column![main];
        if open {
            block = block.push(self.row_actions(track));
        }
        block.push(hrule(RULE)).into()
    }

    /// Inline actions under an opened ••• row.
    pub(super) fn row_actions<'a>(&'a self, track: &'a Track) -> Element<'a, Message> {
        self.row_actions_with(track, None)
    }

    /// Inline actions; `queue_index` adds "Remove from queue" for a row in Up Next.
    pub(super) fn row_actions_with<'a>(
        &'a self,
        track: &'a Track,
        queue_index: Option<usize>,
    ) -> Element<'a, Message> {
        let liked = self.liked.contains(&track.video_id);
        let action = |title: String, message: Message| {
            button(label(title, 12.0))
                .on_press(message)
                .padding([6, 12])
                .style(theme::outline_button)
        };
        let actions = row![
            action(
                self.tr("Play next").into_owned(),
                Message::PlayNext(track.clone())
            ),
            action(
                self.tr("Add to queue").into_owned(),
                Message::AddToQueue(track.clone())
            ),
            action(
                self.tr(if liked {
                    "Remove favorite"
                } else {
                    "Add to favorites"
                })
                .into_owned(),
                Message::LikeTrack(track.clone()),
            ),
        ]
        .spacing(10);
        let actions = match queue_index {
            Some(index) => actions.push(action(
                self.tr("Remove from queue").into_owned(),
                Message::RemoveFromQueue(index),
            )),
            None => actions,
        };
        container(actions).padding([4, 52]).into()
    }

    fn collection_grid<'a>(&'a self, items: &[&'a HomeItem], width: f32) -> Element<'a, Message> {
        let columns = if width >= 900.0 {
            3
        } else if width >= 560.0 {
            2
        } else {
            1
        };
        let gap = 20.0;
        let box_width = ((width - gap * (columns as f32 - 1.0)) / columns as f32).floor();
        let mut grid = column![].spacing(gap);
        for chunk in items.chunks(columns) {
            let mut line = row![].spacing(gap);
            for item in chunk {
                let Some(target) = item.browse_target.clone() else {
                    continue;
                };
                let text_width = box_width - 72.0 - 16.0 - 24.0;
                line = line.push(
                    button(
                        row![
                            self.cover(
                                item.thumbnail_url.as_deref(),
                                72.0,
                                crate::artwork::THUMB_MAX
                            ),
                            column![
                                label(fit(&item.title, text_width, 14.0), 14.0),
                                label(fit(&item.subtitle, text_width, 11.0), 11.0).color(DIM),
                            ]
                            .spacing(6),
                        ]
                        .spacing(16)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::OpenCollection(target))
                    .width(box_width)
                    .padding(0)
                    .style(theme::row_button),
                );
            }
            grid = grid.push(line);
        }
        grid.into()
    }
}

/// `Title — Artist`, or just the title when the catalog omitted the artist.
fn byline(title: &str, artist: &str) -> String {
    if artist.trim().is_empty() {
        title.to_owned()
    } else {
        format!("{title} — {artist}")
    }
}

fn card<'a>(
    cover: Element<'a, Message>,
    title: String,
    subtitle: String,
    size: f32,
    message: Message,
) -> Element<'a, Message> {
    button(
        column![
            cover,
            Space::new().height(10),
            label(title, 14.0),
            label(subtitle, 12.0).color(DIM),
        ]
        .spacing(2)
        .width(size),
    )
    .on_press(message)
    .padding(0)
    .style(theme::row_button)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_duration_reads_only_a_trailing_clock() {
        assert_eq!(
            subtitle_duration("Song • Orion • Distant • 3:45"),
            Some("3:45")
        );
        assert_eq!(subtitle_duration("Song • Orion • 1:02:03"), Some("1:02:03"));
        assert_eq!(subtitle_duration("Album • Orion • 2023"), None);
        assert_eq!(subtitle_duration("Song • 3:4"), None);
    }

    #[test]
    fn home_layout_fills_the_page_without_overflowing() {
        for (width, height) in [
            (326.0, 650.0),
            (480.0, 500.0),
            (960.0, 600.0),
            (1206.0, 1250.0),
            (1214.0, 600.0),
            (1856.0, 860.0),
            (2496.0, 1300.0),
        ] {
            let layout = HomeLayout::new(Size::new(width, height), 12);
            let block = SEARCH_H + layout.gap + layout.card + CAROUSEL_CHROME_H;
            assert!(
                layout.top + block <= height + 1.0,
                "{width}x{height}: {layout:?}"
            );
            assert!(
                layout.row_width <= width + 0.5,
                "{width}x{height}: {layout:?}"
            );
            assert!(
                layout.card >= CARD_MIN * 0.75,
                "{width}x{height}: {layout:?}"
            );
            // The block is centered: the space below is never much smaller
            // than the space above, and neither dwarfs the block.
            let below = height - layout.top - block;
            assert!(below + 1.0 >= layout.top, "{width}x{height}: {layout:?}");
            assert!(layout.top <= block, "{width}x{height}: {layout:?}");
            // The cards span the width unless the height caps them.
            assert!(
                layout.row_width >= width - layout.card - layout.card_gap,
                "{width}x{height}: {layout:?}"
            );
        }
        // Phone width: at least two cards, so the row reads as a carousel.
        assert!(HomeLayout::new(Size::new(326.0, 650.0), 12).visible >= 2);
    }

    #[test]
    fn carousel_eases_to_its_target_and_jumps_with_reduced_motion() {
        let mut carousel = Carousel::default();
        carousel.start(300.0, false);
        let start = carousel.started.unwrap();
        let early = carousel.step(start + std::time::Duration::from_millis(160));
        assert!(
            early > 150.0 && early < 300.0,
            "ease-out passes the midpoint early"
        );
        assert_eq!(
            carousel.step(start + std::time::Duration::from_millis(400)),
            300.0
        );
        assert!(!carousel.animating());
        carousel.start(0.0, true);
        assert_eq!(carousel.offset, 0.0);
        assert!(!carousel.animating());
    }

    #[test]
    fn home_picks_prefer_the_essentials_shelf_in_either_language() {
        use crate::music::{BrowseTarget, HomeShelf};
        let collection = |id: &str| HomeItem {
            title: id.into(),
            subtitle: String::new(),
            thumbnail_url: None,
            kind: HomeItemKind::Playlist,
            browse_id: Some(id.into()),
            browse_target: Some(BrowseTarget {
                browse_id: format!("VL{id}"),
                params: None,
                page_type: Some("MUSIC_PAGE_TYPE_PLAYLIST".into()),
            }),
            track: None,
            duration: None,
        };
        let shelves = vec![
            HomeShelf {
                title: "Throwbacks".into(),
                items: vec![collection("a")],
            },
            HomeShelf {
                title: "Esenciales de todos los tiempos".into(),
                items: vec![collection("b")],
            },
        ];
        let mut picks = HomePicks::default();
        picks.rebuild(&shelves);
        assert_eq!(picks.title, "Esenciales de todos los tiempos");
        picks.rebuild(&shelves[..1]);
        assert_eq!(picks.title, "Throwbacks");
    }
}
