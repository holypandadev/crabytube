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
const CARD_MAX: f32 = 240.0;
/// The covers shrink to these before any section is given up: 150 px on desktop, 130 px on
/// phones; only a page too short for the bare block takes them to the last size.
const CARD_FLOOR: f32 = 150.0;
const CARD_FLOOR_NARROW: f32 = 130.0;
const LAST_RESORT_CARD: f32 = 96.0;
/// The part of the next card that shows at the column's edge, the cue that the carousel
/// scrolls: what a row of cards is sized to show, and the least and the most of it that may show
/// before the cards are sized again.
const PEEK: f32 = 0.3;
const MIN_PEEK: f32 = 0.2;
const MAX_PEEK: f32 = 0.6;
const SEARCH_H: f32 = 54.0;
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
/// The height of an artist's row of cards besides its covers.
const STRIP_CHROME_H: f32 = STRIP_GAP + STRIP_HEADING_H + STRIP_HEADING_GAP + CARD_TEXT_H;
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

fn is_essentials(shelf: &crate::music::HomeShelf) -> bool {
    ESSENTIALS_TITLES.contains(&shelf.title.trim().to_lowercase().as_str())
}

impl HomePicks {
    /// Whether `shelves` include "All-time essentials" itself, which YouTube Music usually sends
    /// with the supplemental sections rather than the first feed.
    pub fn has_essentials(shelves: &[crate::music::HomeShelf]) -> bool {
        shelves.iter().any(is_essentials)
    }

    pub fn rebuild(&mut self, shelves: &[crate::music::HomeShelf]) {
        let first = shelves.iter().position(is_essentials).or_else(|| {
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

/// A row of cards on an artist's page: one of the artist's shelves by its place in
/// `CollectionState::related`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Strip {
    Related(usize),
}

impl Strip {
    /// The id of the row's scrollable.
    pub(super) fn id(self) -> String {
        match self {
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
    STRIP_CHROME_H + size
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

/// What the Home page has to show besides the carousel, as far as the layout needs to know. The
/// lists themselves stay in the App.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct HomeExtras {
    /// Recent searches that fit on one line, before any are dropped for lack of height.
    pub chips: usize,
    /// Entries "Jump back in" has.
    pub jump: usize,
    /// Lists "Your lists" has: Favorite songs while it has songs, and the playlists. With none,
    /// a short welcome stands in their place.
    pub lists: usize,
    /// The line that says Home could not be refreshed shows under the greeting.
    pub notice: bool,
}

/// The greeting: large, over the date, while the page has room; one line beside the date when it
/// does not. On a phone it is always small, over the date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GreetingSize {
    Full,
    Small,
}

/// Geometry of the Home page for a given content area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct HomeLayout {
    /// Cards the carousel shows whole at once.
    pub visible: usize,
    pub card: f32,
    pub card_gap: f32,
    /// Width of the page's one column, centred: the search field, every section and the status
    /// line start and end at its edges. It follows the window, not the number of cards.
    pub row_width: f32,
    pub search_width: f32,
    pub narrow: bool,
    /// Space above the greeting: what the page has left over, partly, so a tall page keeps the
    /// block a little above its middle.
    pub top: f32,
    /// The page's gaps: relaxed on a page with room to spare.
    pub spacing: Spacing,
    pub greeting: GreetingSize,
    /// Chips shown under the search field (0 hides the row).
    pub chips: usize,
    /// Entries shown in "Jump back in" (0 hides the section), and how many share a row.
    pub jump: usize,
    pub jump_columns: usize,
    /// Lists shown in "Your lists" (0 hides the section), and how many share a row.
    pub lists: usize,
    pub list_columns: usize,
    /// The welcome that stands in for "Your lists" while the user has none.
    pub welcome: bool,
    /// Whether the line that says Home could not be refreshed shows.
    pub notice: bool,
}

/// The page's gaps: compact, or relaxed on a page with room to spare once the covers are as
/// large as they get.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Spacing {
    /// Between the greeting and the search field.
    pub greeting: f32,
    /// Above each section.
    pub section: f32,
    /// Between a section's heading and its rows.
    pub heading: f32,
    /// Between the carousel's heading and its cards.
    pub carousel: f32,
}

impl Spacing {
    pub const COMPACT: Self = Self {
        greeting: 18.0,
        section: 22.0,
        heading: 10.0,
        carousel: 12.0,
    };
    pub const RELAXED: Self = Self {
        greeting: 24.0,
        section: 28.0,
        heading: 12.0,
        carousel: 16.0,
    };
}

/// The greeting's sizes: large over the date, one line beside it, and a phone's.
pub(super) const GREETING_FULL_FONT: f32 = 40.0;
pub(super) const GREETING_SMALL_FONT: f32 = 30.0;
pub(super) const GREETING_PHONE_FONT: f32 = 22.0;
/// Space between the greeting and the date under it.
pub(super) const GREETING_SUBLINE_GAP: f32 = 6.0;
const GREETING_SUBLINE_H: f32 = 16.0;
/// Space between the search field and the chips, and the chips' height.
pub(super) const CHIPS_GAP: f32 = 8.0;
pub(super) const CHIPS_H: f32 = 30.0;
pub(super) const MAX_CHIPS: usize = 5;
/// The height of a section's heading.
pub(super) const HEADING_H: f32 = 24.0;
/// A row of "Jump back in" or "Your lists": a cover with two lines beside it, and the space between
/// rows.
pub(super) const ROW_H: f32 = 48.0;
pub(super) const ROW_COVER: f32 = 40.0;
pub(super) const ROW_GAP: f32 = 4.0;
/// "Jump back in" shows at most two rows of entries, and "Your lists" two rows of lists.
pub(super) const MAX_JUMP_ROWS: usize = 2;
const MAX_LIST_ROWS: usize = 2;
/// On a phone "Your lists" is a grid of outlined tiles, two across.
pub(super) const TILE_H: f32 = 52.0;
pub(super) const TILE_COVER: f32 = 38.0;
pub(super) const TILE_GAP: f32 = 8.0;
/// The narrowest a list takes in "Your lists" on a wide window.
const LIST_COLUMN_W: f32 = 300.0;
/// The welcome that stands in for "Your lists": one line high on a wide window, a few on a phone.
pub(super) const WELCOME_H: f32 = 56.0;
const WELCOME_H_PHONE: f32 = 128.0;
/// Height of the line that says Home could not be refreshed, with the room under it.
pub(super) const NOTICE_H: f32 = 40.0;
/// Height of the status line at the foot of the page, rule included.
pub(super) const STATUS_H: f32 = 34.0;
/// The widest the page's column gets, centred on a wider window.
const MAX_COLUMN: f32 = 1600.0;
/// The most room a tall page leaves above the greeting.
const MAX_TOP: f32 = 64.0;

/// Height of the greeting, without the space under it.
fn greeting_height(size: GreetingSize, narrow: bool) -> f32 {
    match (size, narrow) {
        (_, true) => (GREETING_PHONE_FONT * 1.3).ceil() + GREETING_SUBLINE_GAP + GREETING_SUBLINE_H,
        (GreetingSize::Full, false) => {
            (GREETING_FULL_FONT * 1.3).ceil() + GREETING_SUBLINE_GAP + GREETING_SUBLINE_H
        }
        (GreetingSize::Small, false) => 40.0,
    }
}

fn chips_height(count: usize) -> f32 {
    if count == 0 { 0.0 } else { CHIPS_GAP + CHIPS_H }
}

/// Height of a section of `count` rows of `row` height, `columns` across, its heading included.
fn rows_height(count: usize, columns: usize, row: f32, gap: f32, spacing: Spacing) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let rows = count.div_ceil(columns.max(1)) as f32;
    spacing.section + HEADING_H + spacing.heading + rows * row + (rows - 1.0) * gap
}

/// The size a row of `items` cards takes: `cap` while the whole row fits, or while the part of the
/// next card at the column's edge reads as the cue to scroll; otherwise a little less, so that a
/// whole number of cards and `PEEK` of the next fill the column.
fn fit_cards(column: f32, gap: f32, cap: f32, items: usize) -> f32 {
    let whole = ((column + gap) / (cap + gap)).floor().max(1.0);
    if items as f32 <= whole {
        return cap;
    }
    let peek = (column - whole * (cap + gap)) / cap;
    let across = if peek < MIN_PEEK {
        whole
    } else if peek > MAX_PEEK {
        whole + 1.0
    } else {
        return cap;
    };
    ((column - across * gap) / (across + PEEK)).floor().min(cap)
}

/// What a Home page shows: the greeting's size, how much of each section, and how many rows of
/// "Your lists", or of the welcome in its place (none hides it).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    greeting: GreetingSize,
    chips: usize,
    jump: usize,
    list_rows: usize,
}

impl HomeLayout {
    /// The greeting, the search field, the chips, "Jump back in", the carousel and "Your lists"
    /// stack in one column as wide as the window, with the status line at the foot; nothing
    /// scrolls. The covers take the height the rest leaves, up to a size that keeps about three
    /// of them across (two and a bit on a phone), and a whole number of them fills the column
    /// with the start of the next one at the edge. When the rest leaves them too little, the page
    /// gives up, in turn: the large greeting, the second row of "Your lists", the second row of
    /// "Jump back in", "Jump back in", the chips, and last "Your lists". A phone gives up the rows
    /// of "Jump back in" before the second row of its tiles. A page with room to spare after that
    /// opens its gaps up a little.
    pub fn new(area: Size, items: usize, extras: HomeExtras) -> Self {
        let width = area.width.max(CARD_MIN);
        let page_h = (area.height - STATUS_H).max(0.0);
        let narrow = width < NARROW_W;
        let column = width.min(MAX_COLUMN).floor();
        let card_gap = if narrow { CARD_GAP_NARROW } else { CARD_GAP };
        // Before Home has loaded, the carousel counts as full.
        let items = if items == 0 { 8 } else { items };
        // The largest cover: three and a bit across on a wide window, two and a bit on a phone.
        let widest = if narrow {
            (column - card_gap * 1.35) / 2.35
        } else {
            (column - card_gap * 2.3) / 3.3
        }
        .clamp(CARD_MIN, CARD_MAX)
        .floor();
        let floor = if narrow {
            CARD_FLOOR_NARROW
        } else {
            CARD_FLOOR
        }
        .min(widest);
        let jump_columns = if narrow { 1 } else { 2 };
        let list_columns = if narrow {
            2
        } else {
            ((column / LIST_COLUMN_W).floor() as usize).clamp(2, 5)
        };
        let welcome_n = extras.lists == 0;
        // The rows "Your lists" needs for every list, up to its limit, and the lists `rows` hold.
        let list_rows = extras.lists.div_ceil(list_columns).clamp(1, MAX_LIST_ROWS);
        let lists_in = |rows: usize| extras.lists.min(list_columns * rows);
        let notice = if extras.notice { NOTICE_H } else { 0.0 };
        let around = |shown: Shown, spacing: Spacing| {
            let lists = match (shown.list_rows, welcome_n) {
                (0, _) => 0.0,
                (_, true) => spacing.section + if narrow { WELCOME_H_PHONE } else { WELCOME_H },
                (rows, false) if narrow => {
                    rows_height(lists_in(rows), list_columns, TILE_H, TILE_GAP, spacing)
                }
                (rows, false) => rows_height(lists_in(rows), list_columns, ROW_H, ROW_GAP, spacing),
            };
            greeting_height(shown.greeting, narrow)
                + notice
                + spacing.greeting
                + SEARCH_H
                + chips_height(shown.chips)
                + rows_height(shown.jump, jump_columns, ROW_H, ROW_GAP, spacing)
                + spacing.section
                + HEADING_H
                + spacing.carousel
                + CARD_TEXT_H
                + lists
        };
        let room = |shown: Shown, spacing: Spacing| page_h - around(shown, spacing);
        let chips = extras.chips.min(MAX_CHIPS);
        let jump = extras.jump.min(MAX_JUMP_ROWS * jump_columns);
        let small = GreetingSize::Small;
        let show = |greeting, chips, jump, list_rows| Shown {
            greeting,
            chips,
            jump,
            list_rows,
        };
        // In order of preference; the large greeting only while the covers are as large as they
        // get anyway, and never on a phone.
        let choices = if narrow {
            vec![
                show(small, chips, jump, list_rows),
                show(small, chips, jump.min(jump_columns), list_rows),
                show(small, chips, 0, list_rows),
                show(small, chips, 0, 1),
                show(small, 0, 0, 1),
                show(small, 0, 0, 0),
            ]
        } else {
            vec![
                show(GreetingSize::Full, chips, jump, list_rows),
                show(small, chips, jump, list_rows),
                show(small, chips, jump, 1),
                show(small, chips, jump.min(jump_columns), 1),
                show(small, chips, 0, 1),
                show(small, 0, 0, 1),
                show(small, 0, 0, 0),
            ]
        };
        let shown = choices
            .iter()
            .copied()
            .find(|shown| {
                let need = if shown.greeting == GreetingSize::Full {
                    widest
                } else {
                    floor
                };
                room(*shown, Spacing::COMPACT) >= need
            })
            .unwrap_or(choices[choices.len() - 1]);
        let cap = room(shown, Spacing::COMPACT)
            .min(widest)
            .max(LAST_RESORT_CARD.min(widest))
            .floor();
        let card = fit_cards(column, card_gap, cap, items);
        let spacing = if !narrow && room(shown, Spacing::RELAXED) >= card {
            Spacing::RELAXED
        } else {
            Spacing::COMPACT
        };
        let left = (room(shown, spacing) - card).max(0.0);
        let top = (left * 0.4).min(MAX_TOP).floor();
        let visible = (((column + card_gap) / (card + card_gap)).floor() as usize).clamp(1, items);
        Self {
            visible,
            card,
            card_gap,
            row_width: column,
            search_width: column,
            narrow,
            top,
            spacing,
            greeting: shown.greeting,
            chips: shown.chips,
            jump: shown.jump,
            jump_columns,
            lists: lists_in(shown.list_rows),
            list_columns,
            welcome: welcome_n && shown.list_rows > 0,
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

    /// Height of the greeting, without the space under it.
    pub fn greeting_height(&self) -> f32 {
        greeting_height(self.greeting, self.narrow)
    }

    /// Y of the search field from the top of the page, under the greeting and the line that says
    /// Home could not be refreshed.
    pub fn search_top(&self) -> f32 {
        self.top
            + self.greeting_height()
            + self.spacing.greeting
            + if self.notice { NOTICE_H } else { 0.0 }
    }

    /// Y of the bottom edge of the whole block, from the top of the page. The tests check the
    /// whole stack against the page; the app itself only needs the parts.
    #[cfg(test)]
    pub fn block_bottom(&self) -> f32 {
        let spacing = self.spacing;
        let lists = if self.lists > 0 {
            if self.narrow {
                rows_height(self.lists, 2, TILE_H, TILE_GAP, spacing)
            } else {
                rows_height(self.lists, self.list_columns, ROW_H, ROW_GAP, spacing)
            }
        } else if self.welcome {
            spacing.section
                + if self.narrow {
                    WELCOME_H_PHONE
                } else {
                    WELCOME_H
                }
        } else {
            0.0
        };
        self.search_top()
            + SEARCH_H
            + chips_height(self.chips)
            + rows_height(self.jump, self.jump_columns, ROW_H, ROW_GAP, spacing)
            + spacing.section
            + HEADING_H
            + spacing.carousel
            + self.card
            + CARD_TEXT_H
            + lists
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
        let column = width.min(MAX_COLUMN).floor();
        let chips = if self.preferences.remember_history {
            super::home_panels::chip_queries(&self.search_history, column, narrow).len()
        } else {
            0
        };
        let extras = HomeExtras {
            chips,
            jump: self.recent.len(),
            lists: self.home_lists().len(),
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
            // The supplemental sections may still bring "All-time essentials".
            let loading = self.home_loading || self.home_more_cancel.is_some();
            let status: Element<'_, Message> = if loading {
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
                .push(Space::new().height(layout.spacing.section))
                .push(self.jump_view(&layout));
        }
        page = page
            .push(Space::new().height(layout.spacing.section))
            .push(carousel);
        if layout.lists > 0 {
            page = page
                .push(Space::new().height(layout.spacing.section))
                .push(self.lists_view(&layout));
        } else if layout.welcome {
            page = page
                .push(Space::new().height(layout.spacing.section))
                .push(self.welcome_view(&layout));
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
    pub(super) fn strip_geometry(&self, _strip: Strip) -> (f32, f32, f32) {
        let narrow = self.area.width < NARROW_W;
        let gap = if narrow { CARD_GAP_NARROW } else { CARD_GAP };
        (strip_card(narrow), gap, self.area.width.max(CARD_MIN))
    }

    /// A row of cards: its heading with arrows that page through it, and its cards, which scroll
    /// sideways.
    pub(super) fn strip_view(&self, strip: Strip) -> Element<'_, Message> {
        let (size, gap, width) = self.strip_geometry(strip);
        let variant = crate::artwork::variant_for_pixels(size * self.pixel_scale());
        let text_width = size - 4.0;
        let (title, cards): (String, Vec<Element<'_, Message>>) = match strip {
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
        let strip_row = scrollable(iced::widget::Row::with_children(cards).spacing(gap))
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

    /// The lists "Your lists" shows: Favorite songs while it has songs, then the playlists in the
    /// Library's order.
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
            Strip::Related(position) => self.collection.related.get(position),
        }
    }

    /// How many cards a row has.
    pub(super) fn strip_len(&self, strip: Strip) -> usize {
        self.strip_shelf(strip).map_or(0, |shelf| shelf.items.len())
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

    /// The covers of the lists "Your lists" shows.
    pub(super) fn lists_artwork(&self, layout: &HomeLayout) -> Vec<(String, u32)> {
        let size = if layout.narrow { TILE_COVER } else { ROW_COVER };
        self.home_lists()
            .into_iter()
            .take(layout.lists)
            .flat_map(|list| {
                let variant = self.list_cover_variant(list, size);
                self.list_cover_urls(list)
                    .into_iter()
                    .map(move |url| (url.to_owned(), variant))
            })
            .collect()
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
        // A shelf that fits whole has nothing to scroll, and no arrows.
        let arrows: Option<Element<'_, Message>> = (max_offset > 0.5).then(|| {
            let can_back = offset > 1.0;
            let can_forward = offset < max_offset - 1.0;
            row![
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
            .spacing(6)
            .into()
        });
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
                arrows
            ),
            Space::new().height(layout.spacing.carousel),
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
            label(title, TEXT_BODY).wrapping(text::Wrapping::None),
            label(subtitle, TEXT_CAPTION)
                .color(DIM)
                .wrapping(text::Wrapping::None),
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

    /// The extras of a user with history, lists and recent searches.
    fn full(chips: usize, jump: usize, lists: usize) -> HomeExtras {
        HomeExtras {
            chips,
            jump,
            lists,
            notice: false,
        }
    }

    #[test]
    fn home_fits_the_page_at_any_size() {
        for width in [326.0, 358.0, 480.0, 960.0, 1214.0, 1354.0, 1856.0, 2496.0] {
            for height in (300..=1300).step_by(20) {
                for extras in [
                    full(5, 4, 6),
                    full(0, 0, 0),
                    HomeExtras {
                        notice: true,
                        ..full(5, 4, 6)
                    },
                ] {
                    let area = Size::new(width, height as f32);
                    let layout = HomeLayout::new(area, 12, extras);
                    let label = format!("{width}x{height} {extras:?}: {layout:?}");
                    let page_h = area.height - STATUS_H;
                    // Nothing runs under the status line unless the page is too short for even
                    // the bare block with the smallest covers.
                    if layout.card > LAST_RESORT_CARD.min(layout.card) + 0.5
                        || layout.chips + layout.jump + layout.lists > 0
                    {
                        assert!(layout.block_bottom() <= page_h + 0.5, "{label}");
                    }
                    assert!(layout.card <= CARD_MAX, "{label}");
                    // The column follows the window, whatever the number of cards.
                    assert_eq!(layout.row_width, width.min(MAX_COLUMN).floor(), "{label}");
                }
            }
        }
    }

    #[test]
    fn sections_go_in_order_when_the_page_is_short() {
        for width in [358.0, 1354.0] {
            for height in (300..=1100).step_by(10) {
                let layout = HomeLayout::new(Size::new(width, height as f32), 12, full(3, 4, 6));
                let label = format!("{width}x{height}: {layout:?}");
                // "Your lists" goes last, the chips before it, and "Jump back in" first.
                if layout.lists == 0 {
                    assert_eq!((layout.chips, layout.jump), (0, 0), "{label}");
                }
                if layout.chips == 0 {
                    assert_eq!(layout.jump, 0, "{label}");
                }
                if width < NARROW_W {
                    // A phone keeps both rows of its tiles while it has any "Jump back in".
                    if layout.jump > 0 {
                        assert_eq!(layout.lists, 4, "{label}");
                    }
                } else {
                    // A wide window keeps both rows of "Jump back in" while it has a second row
                    // of lists, and the large greeting only with all of both.
                    if layout.lists > layout.list_columns {
                        assert_eq!(layout.jump, 4, "{label}");
                    }
                    if layout.greeting == GreetingSize::Full {
                        assert_eq!((layout.jump, layout.lists), (4, 6), "{label}");
                    }
                }
                // The gaps open up only on a wide page with room to spare.
                if layout.spacing == Spacing::RELAXED {
                    assert!(!layout.narrow && layout.card >= CARD_FLOOR, "{label}");
                }
            }
        }
    }

    #[test]
    fn the_usual_window_shows_every_section_with_large_covers() {
        // The page area of a 1418x797 window: the header, the player bar and the gutters gone.
        let layout = HomeLayout::new(Size::new(1354.0, 671.0), 30, full(4, 2, 4));
        assert_eq!(layout.greeting, GreetingSize::Small, "{layout:?}");
        assert_eq!(
            (layout.chips, layout.jump, layout.lists),
            (4, 2, 4),
            "{layout:?}"
        );
        assert_eq!(layout.list_columns, 4, "{layout:?}");
        assert_eq!(layout.spacing, Spacing::COMPACT, "{layout:?}");
        // Seven covers across and the start of the eighth, rather than six and all but a sliver
        // of the seventh.
        assert!(layout.card >= 160.0, "{layout:?}");
        assert_eq!(layout.visible, 7, "{layout:?}");
        let peek = (layout.row_width - layout.visible as f32 * layout.stride()) / layout.card;
        assert!((MIN_PEEK..=MAX_PEEK).contains(&peek), "{layout:?}");
        // A shelf that fits whole takes the full height instead, and has nothing to scroll.
        let short = HomeLayout::new(Size::new(1354.0, 671.0), 6, full(4, 2, 4));
        assert!(short.card > layout.card, "{short:?}");
        assert_eq!(short.max_offset(6), 0.0, "{short:?}");
    }

    #[test]
    fn a_tall_window_has_the_large_greeting_and_the_largest_covers() {
        let layout = HomeLayout::new(Size::new(1214.0, 954.0), 30, full(4, 2, 4));
        assert_eq!(layout.greeting, GreetingSize::Full, "{layout:?}");
        assert_eq!(layout.spacing, Spacing::RELAXED, "{layout:?}");
        // Five covers across, and the next one showing at the edge.
        assert!(layout.card >= 205.0, "{layout:?}");
        assert_eq!(layout.visible, 5, "{layout:?}");
        assert!(layout.max_offset(30) > 0.0, "{layout:?}");
        // The leftover room sits partly above the block, never more than its limit.
        assert!(layout.top > 0.0 && layout.top <= MAX_TOP, "{layout:?}");
    }

    #[test]
    fn a_narrow_desktop_window_keeps_its_covers_large() {
        // The page area of an 800x800 window: four covers across rather than five and a bit.
        let layout = HomeLayout::new(Size::new(736.0, 658.0), 30, full(4, 4, 4));
        assert_eq!(layout.visible, 4, "{layout:?}");
        assert!(layout.card >= 145.0, "{layout:?}");
        // One row each of "Jump back in" and "Your lists" went before the covers shrank further.
        assert_eq!((layout.jump, layout.lists), (2, 2), "{layout:?}");
    }

    #[test]
    fn cards_fill_the_column_with_the_start_of_the_next_one_at_the_edge() {
        // A sliver of the sixth card would show: the five shrink so a third of it does.
        assert_eq!(fit_cards(1354.0, 24.0, 240.0, 30), 232.0);
        // All but a sliver of the seventh would show: seven fit, a little smaller.
        assert_eq!(fit_cards(1354.0, 24.0, 173.0, 30), 162.0);
        // A third of the next card shows already: the size stands.
        assert_eq!(fit_cards(736.0, 24.0, 194.0, 30), 194.0);
        // A shelf that fits whole has nothing to cue.
        assert_eq!(fit_cards(1354.0, 24.0, 173.0, 6), 173.0);
    }

    #[test]
    fn a_phone_keeps_your_lists_and_gives_up_jump_back_in() {
        // The page area of a 390x844 window: the header, the mini player and the tab bar gone.
        let layout = HomeLayout::new(Size::new(358.0, 661.0), 30, full(4, 2, 4));
        assert_eq!(layout.jump, 0, "{layout:?}");
        assert_eq!((layout.chips, layout.lists), (4, 4), "{layout:?}");
        assert_eq!(layout.spacing, Spacing::COMPACT, "{layout:?}");
        // Two covers and the start of a third.
        assert_eq!(layout.visible, 2, "{layout:?}");
        assert!(layout.card >= 140.0, "{layout:?}");
    }

    #[test]
    fn a_new_user_sees_the_welcome_and_the_largest_covers() {
        let layout = HomeLayout::new(Size::new(1354.0, 740.0), 30, HomeExtras::default());
        assert!(layout.welcome, "{layout:?}");
        assert_eq!(layout.lists, 0, "{layout:?}");
        assert_eq!(layout.greeting, GreetingSize::Full, "{layout:?}");
        assert_eq!(layout.spacing, Spacing::RELAXED, "{layout:?}");
        // As large as five and a bit across allow.
        assert!(layout.card >= 230.0, "{layout:?}");
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
