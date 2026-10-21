//! Home: the search field, the "All-time essentials" carousel and search results.

use super::playlists::HEART_W;
use super::shell::{display_title, fit, meta_text, section_heading};
use super::theme::{
    self, DIM, FADED, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_SECTION, TEXT_TITLE, hrule,
    icon, label,
};
use super::{App, Message};
use crate::music::{HomeItem, HomeItemKind, Track};
use crate::preferences::UiDensity;
use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::{
    Space, button, column, container, mouse_area, pin, row, scrollable, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Size, Task};
use std::sync::Arc;

pub(super) const CAROUSEL_ID: &str = "essentials";
/// Card gaps: wide windows and phone-sized ones.
const CARD_GAP: f32 = 24.0;
const CARD_GAP_NARROW: f32 = 14.0;
/// Below this content width the page switches to its phone layout.
pub(super) const NARROW_W: f32 = 600.0;
/// Smallest and largest card edge.
const CARD_MIN: f32 = 104.0;
const CARD_MAX: f32 = 280.0;
/// The most cards the carousel shows at once.
const MAX_ACROSS: usize = 10;
/// The covers shrink to these before any panel is hidden: 130 px on desktop, 110 px on phones.
const CARD_FLOOR: f32 = 130.0;
/// With "Your lists" under the carousel, the covers may shrink this far before a panel goes.
const CARD_FLOOR_LISTS: f32 = 96.0;
const CARD_FLOOR_NARROW: f32 = 110.0;
/// Share of the page height the search field and carousel aim to fill.
const BLOCK_SHARE: f32 = 0.74;
const SEARCH_H: f32 = 54.0;
const SEARCH_MAX_W: f32 = 640.0;
/// From this content width the top result sits beside the first songs.
const TOP_RESULT_SPLIT_W: f32 = 1100.0;
const TOP_RESULT_GAP: f32 = 24.0;
const TOP_RESULT_SONGS: usize = 4;
/// Room the carousel arrows and the shortest rule take beside the shelf name.
const SHELF_ARROWS_W: f32 = 120.0;
/// Results page: the field floats over the rows, which scroll underneath it.
const SEARCH_TOP: f32 = 8.0;
const SEARCH_MARGIN: f32 = 24.0;
/// Scroll start of the results: the field's rectangle and its margin stay clear of the first row.
const SEARCH_HEADER: f32 = SEARCH_TOP + SEARCH_H + SEARCH_MARGIN;
/// Heading row, its gap, and the two caption lines under each cover.
const CAROUSEL_CHROME_H: f32 = 24.0 + 18.0 + 10.0 + 20.0 + 18.0;
const CAROUSEL_DURATION_MS: f32 = 320.0;
/// YouTube Music's "All-time essentials" shelf, by its title in English and Spanish. In the other
/// languages Home takes the first shelf of collections instead (see `HomePicks::rebuild`).
const ESSENTIALS_TITLES: &[&str] = &["all-time essentials", "esenciales de todos los tiempos"];
const MAX_ITEMS: usize = 30;
/// The cards of an artist's rows, the gap above each row of cards, and its heading.
const STRIP_CARD: f32 = 168.0;
const STRIP_CARD_NARROW: f32 = 132.0;
const STRIP_GAP: f32 = 24.0;
const STRIP_HEADING_H: f32 = 24.0;
const STRIP_HEADING_GAP: f32 = 14.0;
/// The room under a card's cover: the gap, the title and the subtitle.
const CARD_TEXT_H: f32 = 10.0 + 20.0 + 18.0;
/// The covers of "Your lists" on Home, against those of the carousel above them.
const LIST_CARD_SHARE: f32 = 0.65;
/// The least room between a list's name and the next cover in "Your lists".
const LIST_NAME_GAP: f32 = 10.0;
/// The height of the "Your lists" row besides its covers.
const LISTS_CHROME_H: f32 = STRIP_GAP + STRIP_HEADING_H + STRIP_HEADING_GAP + CARD_TEXT_H;
/// Songs, and collections of each kind, the results page lists.
const SEARCH_SONGS_SHOWN: usize = 8;
const SEARCH_COLLECTIONS_SHOWN: usize = 6;

/// The one Home shelf: "All-time essentials", or the first shelf of
/// collections when the feed does not include it.
#[derive(Default)]
pub(super) struct HomePicks {
    pub title: String,
    pub items: Vec<HomeItem>,
    /// Song items as a play queue, in shelf order.
    pub queue: Arc<[Track]>,
}

/// A shelf of the catalog shown as a row of cards, as an artist's albums.
pub(super) struct ShelfPicks {
    pub title: String,
    pub items: Vec<HomeItem>,
    /// Song items as a play queue, in shelf order.
    pub queue: Arc<[Track]>,
}

impl ShelfPicks {
    /// The songs and public collections of a shelf, which a card can play or open.
    pub(super) fn of(shelf: &crate::music::HomeShelf) -> Self {
        let items: Vec<HomeItem> = shelf
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
        let queue = items.iter().filter_map(|item| item.track.clone()).collect();
        Self {
            title: shelf.title.clone(),
            items,
            queue,
        }
    }
}

impl HomePicks {
    pub fn rebuild(&mut self, shelves: &[crate::music::HomeShelf]) {
        let first = shelves
            .iter()
            .position(|shelf| {
                ESSENTIALS_TITLES.contains(&shelf.title.trim().to_lowercase().as_str())
            })
            .or_else(|| {
                shelves.iter().position(|shelf| {
                    shelf.items.iter().any(|item| {
                        item.browse_target
                            .as_ref()
                            .is_some_and(|target| target.is_public_collection())
                    })
                })
            });
        let Some(first) = first else {
            *self = Self::default();
            return;
        };
        let picks = ShelfPicks::of(&shelves[first]);
        self.title = picks.title;
        self.items = picks.items;
        self.queue = picks.queue;
    }
}

/// A row of cards: the user's lists on Home, under the carousel, or one of an artist's shelves by
/// its place in `CollectionState::related`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Strip {
    Lists,
    Related(usize),
}

impl Strip {
    /// The id of the row's scrollable.
    pub(super) fn id(self) -> String {
        match self {
            Self::Lists => "home-lists".to_owned(),
            Self::Related(position) => format!("artist-shelf-{position}"),
        }
    }
}

/// The side of a card in an artist's rows.
fn strip_card(narrow: bool) -> f32 {
    if narrow {
        STRIP_CARD_NARROW
    } else {
        STRIP_CARD
    }
}

/// The height of a row of cards of `size`: the gap above it, its heading, and its cards with their
/// two lines.
fn strip_height(size: f32) -> f32 {
    LISTS_CHROME_H + size
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

/// What the Home page shows around the search field and the carousel, as far as the layout
/// needs to know. The lists themselves stay in the App.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct HomeExtras {
    /// Recent searches that fit on one line, before any are dropped for lack of height.
    pub chips: usize,
    /// Entries in "Jump back in", before any are dropped for lack of height.
    pub jump: usize,
    /// The user has lists, which "Your lists" shows under the carousel.
    pub lists: bool,
    /// The line that says Home could not be refreshed shows under the greeting.
    pub notice: bool,
}

/// The greeting is two lines while the page has room, and one line when it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GreetingSize {
    Full,
    Small,
}

/// Geometry of the Home page for a given content area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HomeLayout {
    /// Cards shown at once.
    pub visible: usize,
    pub card: f32,
    pub card_gap: f32,
    /// Width of the page's one column, centred: the card strip, the search field and every panel
    /// start and end at its edges.
    pub row_width: f32,
    pub search_width: f32,
    pub narrow: bool,
    /// Space above the greeting, or above the search field when the block is centered.
    pub top: f32,
    /// Space between the search field and the carousel; the chips take part of it.
    pub gap: f32,
    pub greeting: GreetingSize,
    pub greeting_font: f32,
    /// Chips shown under the search field (0 hides the row).
    pub chips: usize,
    /// Entries shown in "Jump back in" (0 hides the section).
    pub jump: usize,
    /// Whether "Your lists" shows, and the side of its covers.
    pub lists: bool,
    pub list_card: f32,
    /// Whether the line that says Home could not be refreshed shows.
    pub notice: bool,
}

/// Space between the greeting and the search field.
pub(super) const GREETING_GAP: f32 = 24.0;
pub(super) const GREETING_SUBLINE_GAP: f32 = 6.0;
const GREETING_SUBLINE_H: f32 = 18.0;
const SMALL_GREETING_H: f32 = 18.0;
/// Space between the search field and the chips, and the chips' height.
pub(super) const CHIPS_GAP: f32 = 8.0;
pub(super) const CHIPS_H: f32 = 30.0;
pub(super) const MAX_CHIPS: usize = 5;
/// Space above "Jump back in", its heading, and each row of entries.
pub(super) const JUMP_GAP: f32 = 16.0;
const JUMP_HEADING_H: f32 = 22.0;
pub(super) const JUMP_HEADING_GAP: f32 = 8.0;
pub(super) const JUMP_ROW_H: f32 = 40.0;
pub(super) const JUMP_ROW_GAP: f32 = 4.0;
pub(super) const MAX_JUMP: usize = 4;
/// Height of the line that says Home could not be refreshed, with the room under it.
pub(super) const NOTICE_H: f32 = 40.0;
/// Height of the status line at the foot of the page, rule included.
pub(super) const STATUS_H: f32 = 34.0;
/// Clear space kept above and below the Home block.
const EDGE_GAP: f32 = 8.0;

/// Height of the greeting block, including the gap that follows it.
fn greeting_height(size: GreetingSize, font: f32) -> f32 {
    match size {
        GreetingSize::Full => {
            (font * 1.3).ceil() + GREETING_SUBLINE_GAP + GREETING_SUBLINE_H + GREETING_GAP
        }
        GreetingSize::Small => SMALL_GREETING_H + GREETING_GAP,
    }
}

fn chips_height(count: usize) -> f32 {
    if count == 0 { 0.0 } else { CHIPS_GAP + CHIPS_H }
}

fn jump_height(count: usize, narrow: bool) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let rows = (if narrow { count } else { count.div_ceil(2) }) as f32;
    JUMP_GAP + JUMP_HEADING_H + JUMP_HEADING_GAP + rows * JUMP_ROW_H + (rows - 1.0) * JUMP_ROW_GAP
}

impl HomeLayout {
    /// The greeting, the search field, the chips, "Jump back in", the carousel and "Your lists"
    /// form one block, centered on the page (a little above the middle), with the status line at
    /// its foot; nothing scrolls. Cards grow with the height until the block fills about three
    /// quarters of the page, then as many as fit across share the width exactly, so neither axis
    /// is left mostly empty. The covers of "Your lists" are a share of the carousel's and shrink
    /// with them. On a short page the covers shrink first, down to their floor. Then the greeting
    /// takes one line, "Jump back in" keeps two entries and, with "Your lists", the covers shrink
    /// a little more; only then do "Jump back in" and the chips go, and "Your lists" last, when not
    /// even the smallest covers leave room for it. The search field is never resized.
    pub fn new(area: Size, items: usize, extras: HomeExtras) -> Self {
        let layout = Self::fit(area, items, extras);
        // A page too short for "Your lists" is laid out as if there were none, so the column
        // and the covers take the whole room.
        if extras.lists && !layout.lists {
            return Self::fit(
                area,
                items,
                HomeExtras {
                    lists: false,
                    ..extras
                },
            );
        }
        layout
    }

    fn fit(area: Size, items: usize, extras: HomeExtras) -> Self {
        let width = area.width.max(CARD_MIN);
        let page_h = (area.height - STATUS_H).max(0.0);
        // Before Home has loaded, or when it failed, the page has the column a full row of cards
        // would give it, so the search field keeps its width.
        let items = if items == 0 {
            8
        } else {
            items.clamp(1, MAX_ACROSS)
        } as f32;
        let narrow = width < NARROW_W;
        let card_gap = if narrow { CARD_GAP_NARROW } else { CARD_GAP };
        let gap = (page_h * 0.06).clamp(24.0, 56.0).floor();
        let fixed = SEARCH_H + gap + CAROUSEL_CHROME_H;
        // "Your lists" takes its chrome, and covers a share of the carousel's: the two rows of
        // covers count as one row that much taller.
        let lists_fixed = |lists: bool| if lists { LISTS_CHROME_H } else { 0.0 };
        let rows = |lists: bool| if lists { 1.0 + LIST_CARD_SHARE } else { 1.0 };
        // Largest card the height allows.
        let tallest = ((page_h * BLOCK_SHARE - fixed - lists_fixed(extras.lists))
            / rows(extras.lists))
        .clamp(CARD_MIN, CARD_MAX);
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
        card = card
            .min(
                ((page_h - fixed - lists_fixed(extras.lists) - 32.0) / rows(extras.lists))
                    .max(CARD_MIN * 0.75),
            )
            .floor();
        // The card row at this size sets the one column every part of the page shares. Covers
        // that shrink later for the panels leave it as it is: the strip keeps the column's width
        // and shows more of the next card.
        let column = (across * (card + card_gap) - card_gap).min(width).floor();
        // The covers never grow back past this size, once panels have made room.
        let widest = card;
        let greeting_font = (width * 0.035)
            .clamp(28.0, 44.0)
            .min(page_h * 0.06)
            .max(20.0)
            .floor();
        let chips_n = extras.chips.min(MAX_CHIPS);
        let jump_n = extras.jump.min(MAX_JUMP);
        let budget = page_h - 2.0 * EDGE_GAP;
        let notice = if extras.notice { NOTICE_H } else { 0.0 };
        let block = |greeting: GreetingSize, chips: usize, jump: usize, lists: bool, card: f32| {
            greeting_height(greeting, greeting_font)
                + notice
                + chips_height(chips)
                + fixed
                + card * rows(lists)
                + lists_fixed(lists)
                + jump_height(jump, narrow)
        };
        // The covers that make a block too tall by `over` give that room back together.
        let shrink = |card: f32, over: f32, lists: bool| card - over / rows(lists);
        // 1. The covers give room, down to their floor, while the greeting keeps two lines.
        let floor = if narrow {
            CARD_FLOOR_NARROW
        } else {
            CARD_FLOOR
        }
        .min(card);
        let lists_n = extras.lists;
        let full = block(GreetingSize::Full, chips_n, jump_n, lists_n, card);
        if full > budget {
            card = shrink(card, full - budget, lists_n).max(floor).floor();
        }
        // 2. Then the greeting takes one line.
        let mut greeting = GreetingSize::Full;
        if block(greeting, chips_n, jump_n, lists_n, card) > budget {
            greeting = GreetingSize::Small;
        }
        // 3. Then "Jump back in" keeps two entries, and with "Your lists" the covers shrink a
        // little more, so every panel still shows.
        let mut jump = jump_n;
        if block(greeting, chips_n, jump, lists_n, card) > budget {
            jump = jump.min(2);
        }
        let over = block(greeting, chips_n, jump, lists_n, card) - budget;
        if lists_n && over > 0.0 {
            card = shrink(card, over, lists_n)
                .max(CARD_FLOOR_LISTS.min(card))
                .floor();
        }
        // 4. Then "Jump back in" goes, and then the chips.
        let (mut chips, mut lists) = (chips_n, lists_n);
        if block(greeting, chips, jump, lists, card) > budget {
            jump = 0;
        }
        if block(greeting, chips, jump, lists, card) > budget {
            chips = 0;
        }
        // 5. "Your lists" stays while the covers can shrink below their floor for it; only a
        // window too short even for that loses it.
        let last_resort = (CARD_MIN * 0.75).min(card);
        let over = block(greeting, chips, jump, lists, card) - budget;
        if lists && over > 0.0 && shrink(card, over, lists).floor() < last_resort {
            lists = false;
        }
        // 6. The covers then take the height the panels left, up to the size the width gave
        // them; only a window too short even for the bare block takes them below their floor.
        let room = (budget - block(greeting, chips, jump, lists, 0.0)) / rows(lists);
        card = room.min(widest).max(last_resort).floor();
        let total = block(greeting, chips, jump, lists, card);
        let top = ((page_h - total) / 2.0 * 0.9).max(EDGE_GAP).floor();
        Self {
            visible: across as usize,
            card,
            card_gap,
            row_width: column,
            search_width: column,
            narrow,
            top,
            gap,
            greeting,
            greeting_font,
            chips,
            jump,
            lists,
            list_card: (card * LIST_CARD_SHARE).floor(),
            notice: extras.notice,
        }
    }

    pub fn stride(&self) -> f32 {
        self.card + self.card_gap
    }

    /// Largest scroll offset for `count` cards.
    pub fn max_offset(&self, count: usize) -> f32 {
        (count as f32 * self.stride() - self.card_gap - self.row_width).max(0.0)
    }

    /// Height of the greeting block, gap included.
    pub fn greeting_height(&self) -> f32 {
        greeting_height(self.greeting, self.greeting_font)
    }

    /// Y of the search field from the top of the page, under the greeting and the line that says
    /// Home could not be refreshed.
    pub fn search_top(&self) -> f32 {
        self.top + self.greeting_height() + if self.notice { NOTICE_H } else { 0.0 }
    }

    /// Y of the bottom edge of the whole block, from the top of the page. The tests check the
    /// whole stack against the page; the app itself only needs the parts.
    #[cfg(test)]
    pub fn block_bottom(&self) -> f32 {
        self.search_top()
            + SEARCH_H
            + chips_height(self.chips)
            + self.gap
            + CAROUSEL_CHROME_H
            + self.card
            + if self.lists {
                LISTS_CHROME_H + self.list_card
            } else {
                0.0
            }
            + jump_height(self.jump, self.narrow)
    }
}

/// Live broadcasts come before songs only when the top result is itself a broadcast.
fn live_first(top: Option<&HomeItem>) -> bool {
    top.is_some_and(|item| item.kind == HomeItemKind::Live)
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
    /// The Home geometry for `area`, counting the chips and entries the App has content for.
    pub(super) fn home_layout(&self, area: Size) -> HomeLayout {
        let width = area.width.max(CARD_MIN);
        let narrow = width < NARROW_W;
        let search_width = width.min(SEARCH_MAX_W);
        let chips = if self.preferences.remember_history {
            super::home_panels::chip_queries(&self.search_history, search_width, narrow).len()
        } else {
            0
        };
        let extras = HomeExtras {
            chips,
            jump: self.recent.len().min(MAX_JUMP),
            lists: !self.home_lists().is_empty(),
            notice: self.refresh_error().is_some(),
        };
        HomeLayout::new(area, self.home_picks.items.len(), extras)
    }

    /// The search field alone, without suggestions.
    fn search_field(&self, width: f32, hint: bool) -> Element<'_, Message> {
        // The placeholder is cut with an ellipsis rather than mid-word when the field is narrow:
        // the room left by the padding, the icon, the gaps and the hint or clear button.
        let placeholder = fit(
            self.tr_static("Search artist, album, tracks"),
            width - 2.0 * 18.0 - 20.0 - 2.0 * 14.0 - 44.0 - 8.0,
            TEXT_BODY,
        );
        container(
            row![
                icon("search", 20.0, INK),
                text_input(&placeholder, &self.search_query)
                    .id("search-field")
                    .on_input(Message::SearchChanged)
                    .on_submit(Message::SearchSubmitted)
                    .font(MONO)
                    .size(TEXT_BODY)
                    .padding([10, 4])
                    .style(theme::input_style),
                if self.search_query.is_empty() {
                    // The shortcut hint only where there is room and a keyboard.
                    let hint = hint && width >= NARROW_W;
                    Element::from(
                        label(
                            if hint {
                                // German keyboards label the key Strg.
                                self.tr(crate::i18n::SEARCH_SHORTCUT)
                            } else {
                                "".into()
                            },
                            TEXT_CAPTION,
                        )
                        .color(FADED),
                    )
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
                button(label(fit(suggestion, width - 48.0, TEXT_BODY), TEXT_BODY))
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
        // A press that no control takes dismisses the suggestions: controls capture their own
        // presses, so the field and the suggestion rows keep theirs.
        let base: Element<'a, Message> = mouse_area(base)
            .on_press(Message::DismissSuggestions)
            .into();
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
        let layout = self.home_layout(area);
        let search_x = ((area.width - layout.search_width) / 2.0).max(0.0);
        let carousel: Element<'_, Message> = if self.home_picks.items.is_empty() {
            let status: Element<'_, Message> = if self.home_loading {
                label(self.tr("[ LOADING YOUTUBE MUSIC ]").into_owned(), TEXT_BODY)
                    .color(DIM)
                    .into()
            } else if let Some(error) = &self.home_error {
                column![
                    label(self.tr("[ HOME COULD NOT LOAD ]").into_owned(), TEXT_BODY),
                    label(
                        fit(&self.tr(error), layout.row_width, TEXT_CAPTION),
                        TEXT_CAPTION
                    )
                    .color(DIM),
                    button(label(self.tr("Retry").into_owned(), TEXT_BODY))
                        .on_press(Message::ReloadHome)
                        .padding([8, 18])
                        .style(theme::outline_button),
                ]
                .spacing(10)
                .into()
            } else {
                label(
                    self.tr("Search to start listening.").into_owned(),
                    TEXT_BODY,
                )
                .color(DIM)
                .into()
            };
            // The notice starts where the search field does.
            container(container(status).width(layout.row_width))
                .center_x(Length::Fill)
                .into()
        } else {
            container(self.essentials(&layout))
                .width(Length::Fill)
                .center_x(Length::Fill)
                .into()
        };
        let mut head = column![Space::new().height(layout.top), self.greeting_view(&layout)];
        if let Some(line) = self.refresh_failure_line(&layout) {
            head = head.push(line);
        }
        let mut page = head.width(Length::Fill);
        page = page
            .push(container(self.search_field(layout.search_width, true)).center_x(Length::Fill));
        if layout.chips > 0 {
            page = page
                .push(Space::new().height(CHIPS_GAP))
                .push(self.recent_chips_view(&layout));
        }
        if layout.jump > 0 {
            page = page
                .push(Space::new().height(JUMP_GAP))
                .push(self.jump_view(&layout));
        }
        page = page.push(Space::new().height(layout.gap)).push(carousel);
        if layout.lists {
            page = page.push(
                container(self.strip_view(Strip::Lists))
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            );
        }
        page = page
            .height(Length::Fill)
            .push(Space::new().height(Length::Fill))
            .push(container(self.status_line(layout.row_width)).center_x(Length::Fill));
        self.with_suggestions(
            page.into(),
            layout.search_width,
            search_x,
            layout.search_top() + SEARCH_H,
        )
    }

    /// The card size, the gap between cards and the width of a row of cards.
    pub(super) fn strip_geometry(&self, strip: Strip) -> (f32, f32, f32) {
        match strip {
            Strip::Lists => {
                let layout = self.home_layout(self.area);
                (layout.list_card, layout.card_gap, layout.row_width)
            }
            Strip::Related(_) => {
                let narrow = self.area.width < NARROW_W;
                let gap = if narrow { CARD_GAP_NARROW } else { CARD_GAP };
                (strip_card(narrow), gap, self.area.width.max(CARD_MIN))
            }
        }
    }

    /// A row of cards: its heading with arrows that page through it, and its cards, which scroll
    /// sideways.
    pub(super) fn strip_view(&self, strip: Strip) -> Element<'_, Message> {
        let (size, gap, width) = self.strip_geometry(strip);
        let variant = crate::artwork::variant_for_pixels(size * self.pixel_scale());
        let text_width = size - 4.0;
        let (title, cards): (String, Vec<Element<'_, Message>>) = match strip {
            Strip::Lists => {
                let mut cards = Vec::new();
                for list in self.home_lists() {
                    let (name, count) = match list {
                        super::messages::ListId::Favorites => {
                            (self.tr("Favorite songs").into_owned(), self.favorites.len())
                        }
                        super::messages::ListId::Playlist(id) => {
                            let Some(playlist) = self.playlist(id) else {
                                continue;
                            };
                            (playlist.name.clone(), playlist.tracks.len())
                        }
                    };
                    // A list's name may run on into the gap after its cover, where the next one
                    // starts; the covers keep their places.
                    let wide = size + gap - LIST_NAME_GAP;
                    cards.push(card(
                        self.list_cover(list, size),
                        fit(&name, wide - 4.0, TEXT_BODY),
                        fit(&self.song_count(count, false), wide - 4.0, TEXT_CAPTION),
                        wide,
                        Message::Navigate(super::Page::List(list)),
                    ));
                }
                (self.tr("Your lists").into_owned(), cards)
            }
            Strip::Related(_) => {
                let Some(shelf) = self.strip_shelf(strip) else {
                    return Space::new().into();
                };
                let cards = shelf
                    .items
                    .iter()
                    .filter_map(|item| {
                        let action = match &item.track {
                            Some(track) => shelf
                                .queue
                                .iter()
                                .position(|queued| queued.video_id == track.video_id)
                                .map(|index| Message::PlayContext(Arc::clone(&shelf.queue), index)),
                            None => item.browse_target.clone().map(Message::OpenCollection),
                        }?;
                        Some(card(
                            self.cover(item.thumbnail_url.as_deref(), size, variant),
                            fit(&item.title, text_width, TEXT_BODY),
                            fit(&meta_text(&item.subtitle), text_width, TEXT_CAPTION),
                            size,
                            action,
                        ))
                    })
                    .collect();
                (shelf.title.clone(), cards)
            }
        };
        let count = cards.len();
        let offset = self.strip_offset(strip);
        let max_offset = strip_max_offset(count, size, gap, width);
        let can_back = offset > 1.0;
        let can_forward = offset < max_offset - 1.0;
        let arrow = |name: &'static str, enabled: bool, forward: bool| {
            button(icon(name, 18.0, if enabled { INK } else { FADED }))
                .on_press_maybe(enabled.then_some(Message::StripStep { strip, forward }))
                .padding(4)
                .style(theme::icon_button)
        };
        let arrows = row![
            arrow("chev-left", can_back, false),
            arrow("chev-right", can_forward, true)
        ]
        .spacing(6);
        let spacing = if strip == Strip::Lists {
            LIST_NAME_GAP
        } else {
            gap
        };
        let strip_row = scrollable(iced::widget::Row::with_children(cards).spacing(spacing))
            .id(strip.id())
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::hidden(),
            ))
            .on_scroll(move |viewport| Message::StripScrolled(strip, viewport.absolute_offset().x))
            .width(width)
            .height(size + CARD_TEXT_H);
        column![
            Space::new().height(STRIP_GAP),
            section_heading(
                fit(&title, width - SHELF_ARROWS_W, TEXT_SECTION),
                Some(arrows.into())
            ),
            Space::new().height(STRIP_HEADING_GAP),
            strip_row,
        ]
        .width(width)
        .into()
    }

    /// The lists the "Your lists" row shows: Favorite songs while it has songs, then the
    /// playlists in the Library's order.
    pub(super) fn home_lists(&self) -> Vec<super::messages::ListId> {
        let favorites = (!self.favorites.is_empty()).then_some(super::messages::ListId::Favorites);
        favorites
            .into_iter()
            .chain(
                self.playlists
                    .iter()
                    .map(|playlist| super::messages::ListId::Playlist(playlist.id)),
            )
            .collect()
    }

    /// The shelf an artist's row shows.
    fn strip_shelf(&self, strip: Strip) -> Option<&ShelfPicks> {
        match strip {
            Strip::Lists => None,
            Strip::Related(position) => self.collection.related.get(position),
        }
    }

    /// How many cards a row has.
    pub(super) fn strip_len(&self, strip: Strip) -> usize {
        match strip {
            Strip::Lists => self.home_lists().len(),
            _ => self.strip_shelf(strip).map_or(0, |shelf| shelf.items.len()),
        }
    }

    /// How far a row is scrolled.
    pub(super) fn strip_offset(&self, strip: Strip) -> f32 {
        self.strips.get(&strip).map_or(0.0, |motion| motion.offset)
    }

    /// The arrows of a row: one screen of its cards on, or back, ending on a card's edge.
    pub(super) fn step_strip(&mut self, strip: Strip, forward: bool) -> Task<Message> {
        let (size, gap, width) = self.strip_geometry(strip);
        let stride = size + gap;
        let max = strip_max_offset(self.strip_len(strip), size, gap, width);
        let shown = ((width + gap) / stride).floor().max(1.0);
        let offset = self.strip_offset(strip);
        let target = if forward {
            (offset + shown * stride).min(max)
        } else {
            (offset - shown * stride).max(0.0)
        };
        let target = if target >= max - 0.5 {
            max
        } else {
            (target / stride).round() * stride
        };
        let reduced = self.preferences.reduced_motion;
        let motion = self.strips.entry(strip).or_default();
        motion.start(target, reduced);
        if motion.animating() {
            return Task::none();
        }
        Task::batch([
            operation::scroll_to(strip.id(), AbsoluteOffset { x: target, y: 0.0 }),
            self.scroll_preload_task(),
        ])
    }

    /// The covers of "Your lists" in its view, when it shows.
    pub(super) fn lists_artwork(&self, layout: &HomeLayout) -> Vec<(String, u32)> {
        if !layout.lists {
            return Vec::new();
        }
        self.strips_artwork(&[Strip::Lists], 0.0, 0.0)
    }

    /// The covers of `strips`, laid one under the other from `top` in a page scrolled to `scroll`,
    /// that show or nearly: the rows in the window and the cards in each row's view.
    pub(super) fn strips_artwork(
        &self,
        strips: &[Strip],
        top: f32,
        scroll: f32,
    ) -> Vec<(String, u32)> {
        let mut y = top;
        let mut requests = Vec::new();
        for &strip in strips {
            let (size, gap, width) = self.strip_geometry(strip);
            let stride = size + gap;
            let height = strip_height(size);
            let shown = y + height >= scroll - height && y <= scroll + self.area.height + height;
            y += height;
            if !shown {
                continue;
            }
            let variant = crate::artwork::variant_for_pixels(size * self.pixel_scale());
            let offset = self.strip_offset(strip);
            let first = ((offset / stride).floor() as usize).saturating_sub(1);
            let count = ((width / stride).ceil() as usize) + 3;
            match strip {
                Strip::Lists => {
                    for list in self.home_lists().into_iter().skip(first).take(count) {
                        let variant = self.list_cover_variant(list, size);
                        requests.extend(
                            self.list_cover_urls(list)
                                .into_iter()
                                .map(|url| (url.to_owned(), variant)),
                        );
                    }
                }
                Strip::Related(_) => {
                    if let Some(shelf) = self.strip_shelf(strip) {
                        requests.extend(
                            shelf
                                .items
                                .iter()
                                .skip(first)
                                .take(count)
                                .filter_map(|item| item.thumbnail_url.clone())
                                .map(|url| (url, variant)),
                        );
                    }
                }
            }
        }
        requests
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
                fit(&item.title, text_width, TEXT_BODY),
                fit(&meta_text(&item.subtitle), text_width, TEXT_CAPTION),
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
            // One line: a long shelf name is cut before the arrows rather than wrapped, so the
            // layout's heights hold.
            section_heading(
                fit(
                    &self.home_picks.title,
                    layout.row_width - SHELF_ARROWS_W,
                    TEXT_SECTION
                ),
                Some(arrows.into())
            ),
            Space::new().height(18),
            strip,
        ]
        .width(layout.row_width)
        .into()
    }

    /// The results scroll under the search field, which stays at the top of the page.
    /// Whether Home is what the window shows: the Home page, or the search page before it has any
    /// results to show, which draws Home. Home's covers, clock and status line follow this.
    pub(super) fn home_on_screen(&self) -> bool {
        match self.page {
            super::Page::Home => true,
            super::Page::Search => {
                !self.search_committed && !self.search_loading && self.catalog_results.is_empty()
            }
            _ => false,
        }
    }

    pub(super) fn search_page(&self, area: Size) -> Element<'_, Message> {
        if self.home_on_screen() {
            return self.home_page(area);
        }
        let width = area.width.max(280.0);
        // The field spans the content width, and the heading, the count and every section
        // start at the same left edge under it.
        let search_width = width;
        let search_x = 0.0;
        let query = self.committed_query.trim();
        let mut content = column![Space::new().height(SEARCH_HEADER)].spacing(0);
        let count = self.catalog_results.len();
        content = content
            .push(text(self.tr("Search results")).size(TEXT_TITLE).font(MONO))
            .push(Space::new().height(4))
            .push(
                label(
                    if self.search_loading {
                        self.tr("Searching…").into_owned()
                    } else {
                        self.tr_string(format!("{count} results for {query}"))
                    },
                    TEXT_BODY,
                )
                .color(DIM),
            )
            .push(Space::new().height(10))
            .push(hrule(INK));
        if let Some(error) = &self.search_error {
            content = content.push(Space::new().height(16)).push(
                column![
                    label(self.tr("[ SEARCH FAILED ]").into_owned(), TEXT_BODY),
                    label(fit(&self.tr(error), width, TEXT_CAPTION), TEXT_CAPTION).color(DIM),
                    button(label(self.tr("Retry").into_owned(), TEXT_BODY))
                        .on_press(Message::SearchSubmitted)
                        .padding([8, 18])
                        .style(theme::outline_button),
                ]
                .spacing(10),
            );
        }
        let songs = self.search_songs();
        let song_queue: Arc<[Track]> = songs.iter().filter_map(|item| item.track.clone()).collect();
        let song_rows = |range: std::ops::Range<usize>, row_width: f32| {
            let mut rows = column![].spacing(0);
            for index in range {
                if let Some(track) = &songs[index].track {
                    let duration = songs[index]
                        .duration
                        .map(super::format_duration)
                        .or_else(|| subtitle_duration(&songs[index].subtitle).map(str::to_owned));
                    rows = rows.push(self.song_row(
                        track,
                        duration,
                        Message::PlayContext(Arc::clone(&song_queue), index),
                        row_width,
                        super::messages::RowMenu {
                            list: "songs",
                            index,
                        },
                    ));
                }
            }
            rows
        };
        let section = |title: &'static str| {
            column![
                label(self.tr(title).into_owned(), TEXT_BODY),
                Space::new().height(6),
                hrule(INK),
            ]
        };
        // Songs shown beside the top result are not listed again below it.
        let mut songs_below = 0..songs.len();
        let top = self.catalog_results.first();
        if let Some(top) = top {
            content = content.push(Space::new().height(18));
            if width >= TOP_RESULT_SPLIT_W && !songs.is_empty() {
                let left = (width * 0.4).floor();
                let right = width - left - TOP_RESULT_GAP;
                let beside = songs.len().min(TOP_RESULT_SONGS);
                songs_below = beside..songs.len();
                content = content.push(
                    row![
                        column![
                            label(self.tr("Top result").into_owned(), TEXT_BODY),
                            Space::new().height(8),
                            self.top_result(top, left),
                        ]
                        .width(left),
                        column![section("Songs"), song_rows(0..beside, right)].width(right),
                    ]
                    .spacing(TOP_RESULT_GAP),
                );
            } else {
                content = content
                    .push(label(self.tr("Top result").into_owned(), TEXT_BODY))
                    .push(Space::new().height(8))
                    .push(self.top_result(top, width));
            }
        }
        let live = self.search_live();
        let live_section = (!live.is_empty()).then(|| {
            let queue: Arc<[Track]> = live.iter().filter_map(|item| item.track.clone()).collect();
            let mut block = column![Space::new().height(22), section("Live")];
            for (index, item) in live.iter().enumerate() {
                if let Some(track) = &item.track {
                    block = block.push(self.song_row(
                        track,
                        Some("LIVE".to_owned()),
                        Message::PlayContext(Arc::clone(&queue), index),
                        width,
                        super::messages::RowMenu {
                            list: "live",
                            index,
                        },
                    ));
                }
            }
            block
        });
        let songs_section = (!songs_below.is_empty()).then(|| {
            let title = if songs_below.start > 0 {
                "More songs"
            } else {
                "Songs"
            };
            column![
                Space::new().height(22),
                section(title),
                song_rows(songs_below.clone(), width)
            ]
        });
        let (first, second) = if live_first(top) {
            (live_section, songs_section)
        } else {
            (songs_section, live_section)
        };
        for block in [first, second].into_iter().flatten() {
            content = content.push(block);
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
                .push(label(self.tr(title).into_owned(), TEXT_BODY))
                .push(Space::new().height(6))
                .push(hrule(INK))
                .push(Space::new().height(12))
                .push(self.collection_grid(&items, width));
        }
        content = content.push(Space::new().height(28));
        // The results fill the whole page and scroll under the field. Only the field's own
        // rectangle is opaque, so rows stay visible on both sides of it.
        let results = scrollable(content)
            .id("search-content")
            .on_scroll(|viewport| {
                let below = viewport.content_bounds().height
                    - viewport.absolute_offset().y
                    - viewport.bounds().height;
                Message::SearchScrolled {
                    offset: viewport.absolute_offset(),
                    near_end: below < super::layout::CATALOG_MORE_MARGIN,
                }
            })
            .direction(theme::vbar())
            .style(theme::scroll_style)
            .width(Length::Fill)
            .height(Length::Fill);
        let field = container(self.search_field(search_width, false))
            .width(Length::Fill)
            .padding(iced::Padding {
                top: SEARCH_TOP,
                ..iced::Padding::ZERO
            })
            .center_x(Length::Fill);
        let base: Element<'_, Message> = stack![results, field]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        self.with_suggestions(base, search_width, search_x, SEARCH_TOP + SEARCH_H)
    }

    /// Live broadcasts the results page lists above "Songs".
    pub(super) fn search_live(&self) -> Vec<&HomeItem> {
        self.catalog_results
            .iter()
            .filter(|item| item.kind == HomeItemKind::Live && item.track.is_some())
            .take(8)
            .collect()
    }

    /// The song and live rows of the results page, in the order they are listed: the songs beside
    /// the top result, then the two sections as the page orders them. Empty while the page still
    /// shows Home.
    pub(super) fn search_row_order(&self) -> Vec<super::messages::RowMenu> {
        if self.home_on_screen() {
            return Vec::new();
        }
        let songs = self.search_songs().len();
        let live = self.search_live().len();
        let top = self.catalog_results.first();
        let beside = if top.is_some() && self.area.width.max(280.0) >= TOP_RESULT_SPLIT_W {
            songs.min(TOP_RESULT_SONGS)
        } else {
            0
        };
        let row = |list: &'static str, index: usize| super::messages::RowMenu { list, index };
        let mut order: Vec<_> = (0..beside).map(|index| row("songs", index)).collect();
        let songs_below = (beside..songs).map(|index| row("songs", index));
        let live_rows = (0..live).map(|index| row("live", index));
        if live_first(top) {
            order.extend(live_rows.chain(songs_below));
        } else {
            order.extend(songs_below.chain(live_rows));
        }
        order
    }

    /// Whether more results could still show on the page: until the songs, albums and playlists are
    /// full, a further page of results may add to them. Artists rarely fill their row and do not
    /// count.
    pub(super) fn search_wants_more(&self) -> bool {
        self.search_songs().len() < SEARCH_SONGS_SHOWN
            || [HomeItemKind::Album, HomeItemKind::Playlist]
                .into_iter()
                .any(|kind| self.search_collections(kind).len() < SEARCH_COLLECTIONS_SHOWN)
    }

    /// Song results the results page lists under "Songs".
    pub(super) fn search_songs(&self) -> Vec<&HomeItem> {
        self.catalog_results
            .iter()
            .filter(|item| item.kind == HomeItemKind::Song && item.track.is_some())
            .take(SEARCH_SONGS_SHOWN)
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
            .take(SEARCH_COLLECTIONS_SHOWN)
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
                    text(fit(&title, text_width, TEXT_SECTION))
                        .size(TEXT_SECTION)
                        .font(MONO),
                    label(
                        fit(&meta_text(&item.subtitle), text_width, TEXT_CAPTION),
                        TEXT_CAPTION
                    )
                    .color(DIM),
                    Space::new().height(6),
                    button(
                        row![
                            icon("play", 14.0, INK),
                            label(self.tr("Play").into_owned(), TEXT_BODY)
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
        menu: super::messages::RowMenu,
    ) -> Element<'a, Message> {
        let open = self.row_menu.as_ref() == Some(&menu);
        let (list, index) = (menu.list, menu.index);
        let cursor = self.row_cursor.as_ref() == Some(&menu);
        let playing = self
            .playback_status
            .track
            .as_ref()
            .is_some_and(|current| current.video_id == track.video_id);
        let duration = duration.unwrap_or_default();
        let density = self.preferences.ui_density;
        let cover = density.result_cover();
        let padding = UiDensity::row_padding(cover, density.result_row_height());
        let text_width = width - cover - 16.0 - 60.0 - 44.0 - 24.0 - HEART_W - 16.0;
        let ink = theme::row_ink(playing);
        let title = byline(&track.title, &track.artist);
        let title = if playing {
            format!("▶ {title}")
        } else {
            title
        };
        let more = button(icon("more", 18.0, ink.icon))
            .on_press(Message::ToggleRowMenu(menu))
            .padding(self.row_icon_padding(cover))
            .style(theme::row_icon_button(playing, open));
        // The ••• button sits inside the row, so the hover fill covers the
        // whole line with even padding on both sides.
        let main = button(
            row![
                self.cover(
                    track.thumbnail_url.as_deref(),
                    cover,
                    crate::artwork::THUMB_MAX
                ),
                text(fit(&title, text_width, TEXT_BODY))
                    .size(TEXT_BODY)
                    .font(MONO)
                    .color(ink.title)
                    .width(Length::Fill),
                label(duration, TEXT_CAPTION).color(ink.secondary),
                more,
                self.row_heart(
                    track,
                    ink.icon,
                    playing,
                    cover,
                    Message::OpenSaveDialog(track.clone())
                ),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .on_press(play)
        .width(Length::Fill)
        .padding([padding, 8.0])
        .style(if playing {
            theme::row_button_current
        } else if cursor {
            theme::row_button_cursor
        } else {
            theme::row_button
        });
        super::keyboard::mark_row(list, index, column![main, hrule(RULE)])
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
                                label(
                                    fit(display_title(&item.title), text_width, TEXT_BODY),
                                    TEXT_BODY
                                ),
                                label(
                                    fit(&meta_text(&item.subtitle), text_width, TEXT_CAPTION),
                                    TEXT_CAPTION
                                )
                                .color(DIM),
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

/// The farthest a row of `count` cards of `size`, `gap` apart, scrolls in `width`.
fn strip_max_offset(count: usize, size: f32, gap: f32, width: f32) -> f32 {
    (count as f32 * (size + gap) - gap - width).max(0.0)
}

/// `Title — Artist`, or just the title when the catalog omitted the artist.
fn byline(title: &str, artist: &str) -> String {
    let title = super::shell::display_title(title);
    if artist.trim().is_empty() {
        title.to_owned()
    } else {
        format!("{title} — {artist}")
    }
}

/// A card: its cover over two lines of text, `width` wide.
fn card<'a>(
    cover: Element<'a, Message>,
    title: String,
    subtitle: String,
    width: f32,
    message: Message,
) -> Element<'a, Message> {
    button(
        column![
            cover,
            Space::new().height(10),
            label(title, TEXT_BODY),
            label(subtitle, TEXT_CAPTION).color(DIM),
        ]
        .spacing(2)
        .width(width),
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
    fn live_broadcasts_come_first_only_after_a_live_top_result() {
        let item = |kind| HomeItem {
            title: "Item".into(),
            subtitle: String::new(),
            thumbnail_url: None,
            kind,
            browse_id: None,
            browse_target: None,
            track: None,
            duration: None,
        };
        assert!(live_first(Some(&item(HomeItemKind::Live))));
        assert!(!live_first(Some(&item(HomeItemKind::Song))));
        assert!(!live_first(Some(&item(HomeItemKind::Album))));
        assert!(!live_first(None));
    }

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
        for ((width, height), lists) in [
            (326.0, 650.0),
            (480.0, 500.0),
            (960.0, 600.0),
            (1206.0, 1250.0),
            (1214.0, 600.0),
            (1270.0, 1040.0),
            (1856.0, 860.0),
            (2496.0, 1300.0),
        ]
        .into_iter()
        .flat_map(|size| [(size, false), (size, true)])
        {
            let layout = HomeLayout::new(
                Size::new(width, height),
                12,
                // The worst case: "Your lists" and the notice that Home could not be refreshed.
                HomeExtras {
                    chips: 5,
                    jump: 4,
                    lists,
                    notice: lists,
                },
            );
            // The covers never grow past their limit, and "Your lists" keeps its share of them.
            assert!(layout.card <= CARD_MAX, "{width}x{height}: {layout:?}");
            if layout.lists {
                assert_eq!(
                    layout.list_card,
                    (layout.card * LIST_CARD_SHARE).floor(),
                    "{width}x{height}: {layout:?}"
                );
            }
            let page_h = height - STATUS_H;
            assert!(
                layout.block_bottom() <= page_h + 0.5,
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
            let below = page_h - layout.block_bottom();
            assert!(below + 1.0 >= layout.top, "{width}x{height}: {layout:?}");
            assert!(layout.top <= page_h, "{width}x{height}: {layout:?}");
            // The cards span the width unless the height caps them, which is the case at or
            // below the cover floor, or there are as many across as there can be.
            if layout.card > CARD_FLOOR + 0.5 && layout.visible < MAX_ACROSS {
                assert!(
                    layout.row_width >= width - layout.card - layout.card_gap,
                    "{width}x{height}: {layout:?}"
                );
            }
        }
        // Phone width: at least two cards, so the row reads as a carousel.
        assert!(HomeLayout::new(Size::new(326.0, 650.0), 12, HomeExtras::default()).visible >= 2);
    }

    #[test]
    fn a_window_of_the_usual_size_shows_every_panel_with_your_lists() {
        // The page area of a 1418x797 window: the header, the player bar and the gutters gone.
        let area = Size::new(1354.0, 671.0);
        let layout = HomeLayout::new(
            area,
            30,
            HomeExtras {
                chips: 1,
                jump: 4,
                lists: true,
                notice: false,
            },
        );
        assert!(layout.lists, "{layout:?}");
        assert!(layout.jump >= 2, "{layout:?}");
        assert_eq!(layout.chips, 1, "{layout:?}");
        assert!(layout.card >= CARD_FLOOR_LISTS, "{layout:?}");
        assert!(
            layout.block_bottom() <= area.height - STATUS_H + 0.5,
            "{layout:?}"
        );
    }

    #[test]
    fn a_home_that_has_not_loaded_keeps_the_column_of_a_full_row() {
        let area = Size::new(1024.0, 720.0);
        let empty = HomeLayout::new(area, 0, HomeExtras::default());
        let loaded = HomeLayout::new(area, 12, HomeExtras::default());
        assert_eq!(empty.search_width, loaded.search_width);
        assert_eq!(empty.row_width, loaded.row_width);
        assert!(empty.search_width > area.width * 0.8);
    }

    #[test]
    fn short_pages_shrink_covers_then_the_greeting_then_hide_jump_then_chips() {
        for width in [360.0, 390.0, 1206.0, 1354.0, 1856.0] {
            let narrow = width < NARROW_W;
            let floor = if narrow {
                CARD_FLOOR_NARROW
            } else {
                CARD_FLOOR
            };
            for height in (560..=1100).step_by(4) {
                // The page area is the window minus the title bar, the navigation row, the
                // player bar and the status line's rule.
                let area = Size::new(width, height as f32 - 195.0);
                let layout = HomeLayout::new(
                    area,
                    12,
                    HomeExtras {
                        chips: 5,
                        jump: 4,
                        lists: false,
                        notice: false,
                    },
                );
                let page_h = area.height - STATUS_H;
                let label = format!("{width}x{height}: {layout:?}");
                assert!(layout.block_bottom() <= page_h + 0.5, "{label}");
                if layout.chips == 0 {
                    assert_eq!(layout.jump, 0, "{label}");
                }
                // A panel goes only when it does not fit beside covers at their floor: with it
                // back, the block would not fit.
                if layout.jump == 0 {
                    let with_jump = HomeLayout::new(
                        area,
                        12,
                        HomeExtras {
                            chips: layout.chips,
                            jump: 2,
                            lists: false,
                            notice: false,
                        },
                    );
                    assert!(with_jump.jump == 0 || with_jump.card < floor, "{label}");
                }
                // The greeting takes one line only when two would not fit beside covers at their
                // floor.
                if layout.greeting == GreetingSize::Small && layout.jump == 4 {
                    let saved = greeting_height(GreetingSize::Full, layout.greeting_font)
                        - greeting_height(GreetingSize::Small, layout.greeting_font);
                    assert!(layout.card - saved < floor + 0.5, "{label}");
                }
                // The covers take the room the panels leave: the block fills the page, or the
                // covers are as large as the width lets them be.
                let bare = HomeLayout::new(area, 12, HomeExtras::default());
                assert!(
                    layout.top <= EDGE_GAP + 1.0 || layout.card >= bare.card - 0.5,
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn the_panels_only_shrink_the_covers_down_to_their_floor() {
        for (width, height) in [(390.0, 649.0), (1206.0, 525.0), (1354.0, 602.0)] {
            let area = Size::new(width, height);
            let bare = HomeLayout::new(area, 12, HomeExtras::default());
            let full = HomeLayout::new(
                area,
                12,
                HomeExtras {
                    chips: 5,
                    jump: 4,
                    lists: false,
                    notice: false,
                },
            );
            let floor = if width < NARROW_W {
                CARD_FLOOR_NARROW
            } else {
                CARD_FLOOR
            };
            assert_eq!(bare.gap, full.gap, "{width}x{height}");
            assert_eq!(bare.search_width, full.search_width, "{width}x{height}");
            assert!(full.card <= bare.card, "{width}x{height}: {full:?}");
            assert!(
                full.card >= floor.min(bare.card),
                "{width}x{height}: {full:?}"
            );
        }
    }

    #[test]
    fn reference_windows_show_chips_and_jump_back_in_with_history() {
        // Page areas: the window minus the chrome, for the sizes the owner uses.
        for (width, height) in [
            (1206.0, 525.0),
            (1354.0, 602.0),
            (1856.0, 885.0),
            (358.0, 649.0),
        ] {
            let area = Size::new(width, height);
            let layout = HomeLayout::new(
                area,
                12,
                HomeExtras {
                    chips: 1,
                    jump: 1,
                    lists: false,
                    notice: false,
                },
            );
            let floor = if width < NARROW_W {
                CARD_FLOOR_NARROW
            } else {
                CARD_FLOOR
            };
            let label = format!("{width}x{height}: {layout:?}");
            assert_eq!((layout.chips, layout.jump), (1, 1), "{label}");
            assert!(layout.card >= floor, "{label}");
            assert!(
                layout.block_bottom() <= area.height - STATUS_H + 0.5,
                "{label}"
            );
        }
    }

    #[test]
    fn tall_windows_show_every_panel_and_keep_the_block_centered() {
        let layout = HomeLayout::new(
            Size::new(1920.0, 1080.0),
            12,
            HomeExtras {
                chips: 5,
                jump: 4,
                lists: false,
                notice: false,
            },
        );
        assert_eq!(layout.greeting, GreetingSize::Full);
        assert_eq!((layout.chips, layout.jump), (5, 4), "{layout:?}");
        let page_h = 1080.0 - STATUS_H;
        let below = page_h - layout.block_bottom();
        assert!(below >= layout.top, "{layout:?}");
        assert!(below - layout.top <= page_h * 0.2, "{layout:?}");
        // The search field sits below the greeting, and the status line is never covered.
        assert!(layout.search_top() > layout.top, "{layout:?}");
        assert!(layout.block_bottom() <= page_h, "{layout:?}");
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
