//! The Home panels around the carousel: the greeting, recent searches, "Jump back in" and the
//! status line. `HomeLayout` decides which of them fit; this module draws them.

use super::home::{
    CHIPS_H, GREETING_GAP, GREETING_SUBLINE_GAP, GreetingSize, HomeLayout, JUMP_HEADING_GAP,
    JUMP_ROW_GAP, MAX_CHIPS, STATUS_H,
};
use super::messages::RowMenu;
use super::playlists::HEART_W;
use super::shell::{fit, section_heading};
use super::theme::{
    self, BLACK, DIM, FADED, MONO, MONO_BOLD, RULE, TEXT_BODY, TEXT_CAPTION, hrule, icon, label,
};
use super::{App, Message, format_duration};
use crate::local_clock::LocalClock;
use crate::preferences::{RecentPlay, UiLanguage};
use crate::process_stats::{self, ProcessReading};
use futures_util::{SinkExt, Stream};
use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Padding};
use std::sync::Arc;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

/// Edge of the thumbnails in "Jump back in".
pub(super) const JUMP_THUMB: f32 = 36.0;
/// Width of the duration or track count at the end of a "Jump back in" entry.
const JUMP_META_W: f32 = 96.0;
/// An entry narrower than this leaves the length out.
const JUMP_META_MIN_W: f32 = 440.0;
/// Gap between the two columns of "Jump back in".
const JUMP_COLUMN_GAP: f32 = 16.0;
const CHIP_TEXT_SIZE: f32 = TEXT_CAPTION;
/// Estimated advance of one monospace character at chip size, and the chip's padding and border.
const CHIP_CHAR_W: f32 = 8.0;
const CHIP_PADDING: f32 = 26.0 + CHIP_CLOSE + CHIP_CLOSE_GAP;
/// The × that removes a recent search; its room is kept even while it is hidden.
const CHIP_CLOSE: f32 = 10.0;
const CHIP_CLOSE_GAP: f32 = 8.0;
const CHIP_GAP: f32 = 8.0;
/// Button padding (12 px each side), its border and the gap before it, beside the failed-refresh text.
const REFRESH_RETRY_EXTRA: f32 = 38.0;

const WEEKDAYS: [&str; 7] = [
    "SUNDAY",
    "MONDAY",
    "TUESDAY",
    "WEDNESDAY",
    "THURSDAY",
    "FRIDAY",
    "SATURDAY",
];
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// The part of the day the greeting names. Night runs from 00:00 to 04:59; evening runs to midnight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DayPart {
    Morning,
    Afternoon,
    Evening,
    Night,
}

pub(super) fn day_part(hour: u8) -> DayPart {
    match hour {
        0..=4 => DayPart::Night,
        5..=11 => DayPart::Morning,
        12..=17 => DayPart::Afternoon,
        _ => DayPart::Evening,
    }
}

fn greeting_for(part: DayPart) -> &'static str {
    match part {
        DayPart::Morning => "GOOD MORNING.",
        DayPart::Afternoon => "GOOD AFTERNOON.",
        DayPart::Evening => "GOOD EVENING.",
        DayPart::Night => "STILL UP?",
    }
}

/// "21:14 · JUEVES 09 OCT" in Spanish, "9:14 PM · THURSDAY 09 OCT" in English: the time, then the
/// date as the language writes it.
fn clock_line(clock: LocalClock, twelve_hour: bool, language: UiLanguage) -> String {
    let weekday = WEEKDAYS[usize::from(clock.weekday % 7)];
    let month_number = clock.month.clamp(1, 12);
    let month = MONTHS[usize::from(month_number - 1)];
    format!(
        "{} · {}",
        clock_time(clock, twelve_hour),
        crate::i18n::date_line(language, weekday, clock.day, month, month_number)
    )
}

/// The time of day: 24 hours, or 12 hours with AM or PM.
fn clock_time(clock: LocalClock, twelve_hour: bool) -> String {
    if !twelve_hour {
        return format!("{:02}:{:02}", clock.hour, clock.minute);
    }
    let hour = match clock.hour % 12 {
        0 => 12,
        hour => hour,
    };
    let period = if clock.hour < 12 { "AM" } else { "PM" };
    format!("{hour}:{:02} {period}", clock.minute)
}

/// English reads the clock on 12 hours, Spanish on 24.
fn uses_twelve_hour_clock(language: UiLanguage) -> bool {
    language == UiLanguage::English
}

/// The recent searches that get a chip, in order and at most `MAX_CHIPS`. On a desktop width
/// only the ones that fit on one line; on a phone the row scrolls, so all of them count.
pub(super) fn chip_queries(searches: &[String], width: f32, narrow: bool) -> Vec<String> {
    let mut chips: Vec<String> = Vec::new();
    let mut used = 0.0;
    for query in searches.iter().take(MAX_CHIPS) {
        let chip = query.width() as f32 * CHIP_CHAR_W + CHIP_PADDING;
        let next = if chips.is_empty() {
            chip
        } else {
            used + CHIP_GAP + chip
        };
        if !narrow && next > width {
            break;
        }
        used = next;
        chips.push(query.clone());
    }
    chips
}

impl App {
    /// The greeting, the local time and the date. Two lines when the layout has room for them,
    /// one line when it does not.
    pub(super) fn greeting_view(&self, layout: &HomeLayout) -> Element<'_, Message> {
        let clock = self.clock;
        let twelve_hour = uses_twelve_hour_clock(self.preferences.ui_language);
        let stamp = clock_line(clock, twelve_hour, self.preferences.ui_language);
        let greeting = self.tr_static(greeting_for(day_part(clock.hour)));
        let content: Element<'_, Message> = match layout.greeting {
            GreetingSize::Full => column![
                text(greeting).size(layout.greeting_font).font(MONO_BOLD),
                Space::new().height(GREETING_SUBLINE_GAP),
                label(stamp, TEXT_BODY).color(DIM),
            ]
            .into(),
            GreetingSize::Small => row![
                text(greeting).size(TEXT_BODY).font(MONO_BOLD),
                label(stamp, TEXT_CAPTION).color(DIM),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        };
        container(container(content).width(layout.search_width))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(Padding {
                bottom: GREETING_GAP,
                ..Padding::ZERO
            })
            .into()
    }

    /// The error of a refresh that failed while shelves from before are still shown. Without shelves
    /// the empty state reports the same error with its own Retry button.
    pub(super) fn refresh_error(&self) -> Option<&str> {
        if self.home_picks.items.is_empty() {
            return None;
        }
        self.home_error.as_deref()
    }

    /// The line under the greeting that reports a failed refresh and offers to try it again.
    pub(super) fn refresh_failure_line(&self, layout: &HomeLayout) -> Option<Element<'_, Message>> {
        let error = self.refresh_error()?;
        let text = self
            .tr_static("Could not refresh Home: {}")
            .replacen("{}", &self.tr(error), 1);
        let retry = self.tr("Retry").into_owned();
        // The button is as wide as its label, plus its padding and border, and the gap before it.
        let retry_room = retry.width() as f32 * TEXT_CAPTION * 0.6 + REFRESH_RETRY_EXTRA;
        let line = row![
            label(
                fit(&text, layout.search_width - retry_room, TEXT_CAPTION),
                TEXT_CAPTION
            )
            .color(DIM),
            button(label(retry, TEXT_CAPTION))
                .on_press(Message::ReloadHome)
                .padding([5, 12])
                .style(theme::outline_button),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        Some(
            container(container(line).width(layout.search_width))
                .width(Length::Fill)
                .center_x(Length::Fill)
                .padding(Padding {
                    bottom: 12.0,
                    ..Padding::ZERO
                })
                .into(),
        )
    }

    /// The recent searches as outlined chips. A click searches for that query again.
    pub(super) fn recent_chips_view(&self, layout: &HomeLayout) -> Element<'_, Message> {
        let chips: Vec<Element<'_, Message>> =
            chip_queries(&self.search_history, layout.search_width, layout.narrow)
                .into_iter()
                .take(layout.chips)
                .map(|query| {
                    let shown = fit(&query, layout.search_width * 0.8, CHIP_TEXT_SIZE);
                    // The chip inverts while hovered, and only then shows its ×.
                    let hovered = self.hovered_chip.as_deref() == Some(query.as_str());
                    let remove = button(icon(
                        "close",
                        CHIP_CLOSE,
                        if hovered { BLACK } else { Color::TRANSPARENT },
                    ))
                    .on_press(Message::RemoveSearchHistoryItem(query.clone()))
                    .padding(0)
                    .style(theme::bare_button);
                    let chip = button(
                        row![text(shown).size(CHIP_TEXT_SIZE).font(MONO), remove]
                            .spacing(CHIP_CLOSE_GAP)
                            .align_y(Alignment::Center),
                    )
                    .on_press(Message::SearchFor(query.clone()))
                    .padding([6, 12])
                    .style(theme::outline_button);
                    mouse_area(chip)
                        .on_enter(Message::ChipHovered(Some(query)))
                        .on_exit(Message::ChipHovered(None))
                        .into()
                })
                .collect();
        let strip = row(chips).spacing(CHIP_GAP).align_y(Alignment::Center);
        let line: Element<'_, Message> = if layout.narrow {
            scrollable(strip)
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::hidden(),
                ))
                .width(layout.search_width)
                .into()
        } else {
            container(strip).width(layout.search_width).into()
        };
        container(line)
            .width(Length::Fill)
            .height(CHIPS_H)
            .center_x(Length::Fill)
            .into()
    }

    /// "Jump back in": the last things started, as one-line entries in two columns (one on a
    /// phone). Tracks play; collections open.
    pub(super) fn jump_view(&self, layout: &HomeLayout) -> Element<'_, Message> {
        let per_row = if layout.narrow { 1 } else { 2 };
        let cell_width = if layout.narrow {
            layout.search_width
        } else {
            (layout.search_width - JUMP_COLUMN_GAP) / 2.0
        };
        let entries: Vec<&RecentPlay> = self.recent.iter().take(layout.jump).collect();
        let mut grid = column![].spacing(JUMP_ROW_GAP);
        for (chunk_index, chunk) in entries.chunks(per_row).enumerate() {
            let cells: Vec<Element<'_, Message>> = chunk
                .iter()
                .enumerate()
                .map(|(offset, play)| {
                    let menu = RowMenu {
                        list: "jump",
                        index: chunk_index * per_row + offset,
                    };
                    self.jump_cell(play, menu, cell_width)
                })
                .collect();
            grid = grid.push(row(cells).spacing(JUMP_COLUMN_GAP));
        }
        let heading = section_heading(self.tr_static("Jump back in").to_owned(), None);
        container(
            column![heading, Space::new().height(JUMP_HEADING_GAP), grid]
                .width(layout.search_width),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    }

    /// One "Jump back in" entry. A track entry has its ••• button; its actions open under it.
    fn jump_cell<'a>(
        &'a self,
        play: &'a RecentPlay,
        menu: RowMenu,
        width: f32,
    ) -> Element<'a, Message> {
        self.jump_entry(play, width, Some(menu))
    }

    fn jump_entry(
        &self,
        play: &RecentPlay,
        width: f32,
        menu: Option<RowMenu>,
    ) -> Element<'_, Message> {
        let variant = crate::artwork::variant_for_pixels(JUMP_THUMB * self.pixel_scale());
        let (url, title, detail, meta, action) = match play {
            RecentPlay::Track(track) => (
                track.thumbnail_url.as_deref(),
                super::shell::display_title(&track.title).to_owned(),
                track.artist.clone(),
                track
                    .duration_secs
                    .map(|secs| format_duration(Duration::from_secs(u64::from(secs))))
                    .unwrap_or_default(),
                Message::PlayContext(Arc::from(vec![track.clone()]), 0),
            ),
            RecentPlay::Collection(collection) => (
                collection.thumbnail_url.as_deref(),
                collection.title.clone(),
                self.tr_static(if collection.album {
                    "Album"
                } else {
                    "Playlist"
                })
                .to_owned(),
                self.song_count(collection.track_count, collection.more),
                Message::OpenCollection(collection.browse.clone()),
            ),
        };
        let line = if detail.trim().is_empty() {
            title
        } else {
            format!("{title} — {detail}")
        };
        // A track entry's ••• button and heart take room from the title.
        let more_width = if menu.is_some() && matches!(play, RecentPlay::Track(_)) {
            self.row_more_width() + 12.0 + HEART_W
        } else {
            0.0
        };
        // On a phone the length gives its room to the title.
        let show_meta = width >= JUMP_META_MIN_W;
        let meta_width = if show_meta { JUMP_META_W } else { 0.0 };
        let text_width = (width - JUMP_THUMB - meta_width - 36.0 - more_width).max(40.0);
        let mut content = row![
            self.cover(url, JUMP_THUMB, variant),
            container(label(fit(&line, text_width, TEXT_BODY), TEXT_BODY)).width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        if show_meta {
            content = content.push(label(meta, TEXT_CAPTION).color(DIM));
        }
        // The entry under the keyboard cursor is outlined.
        let index = menu.as_ref().map_or(0, |menu| menu.index);
        let cursor = self.row_is_cursor("recent", index);
        if let (Some(menu), RecentPlay::Track(track)) = (menu, play) {
            // The entry keeps the plain row style, so its ••• button and heart do too, even for
            // the track that is playing.
            let open = self.row_menu.as_ref() == Some(&menu);
            let ink = theme::row_ink(false).icon;
            content = content
                .push(
                    button(icon("more", 18.0, ink))
                        .on_press(Message::ToggleRowMenu(menu))
                        .padding(self.row_icon_padding(JUMP_THUMB))
                        .style(theme::row_icon_button(false, open)),
                )
                .push(self.row_heart(
                    track,
                    ink,
                    false,
                    JUMP_THUMB,
                    Message::OpenSaveDialog(track.clone()),
                ));
        }
        let entry = button(content)
            .on_press(action)
            .width(Length::Fill)
            .padding([2, 6])
            .style(if cursor {
                theme::row_button_cursor
            } else {
                theme::row_button
            });
        super::keyboard::mark_row("recent", index, entry)
    }

    /// The status line at the foot of Home: memory and CPU of this process on the left, the
    /// version on the right, after a link to Settings when a newer one is out.
    pub(super) fn status_line(&self, width: f32) -> Element<'_, Message> {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let update: Element<'_, Message> = if self.update.available().is_some() {
            button(
                text(self.tr("Update available"))
                    .size(TEXT_CAPTION)
                    .font(MONO),
            )
            .on_press(Message::OpenUpdates)
            .padding(0)
            .style(theme::text_link(theme::INK))
            .into()
        } else {
            Space::new().width(0).into()
        };
        column![
            hrule(RULE),
            Space::new().height(8.0),
            row![
                label(self.status_text(), TEXT_CAPTION).color(FADED),
                Space::new().width(Length::Fill),
                update,
                label(version, TEXT_CAPTION).color(FADED),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        ]
        .width(width)
        .height(STATUS_H)
        .into()
    }

    /// Memory and CPU, or nothing before the first reading.
    fn status_text(&self) -> String {
        match self.process_reading {
            Some(reading) => {
                let cpu = match reading.cpu_percent {
                    Some(cpu) => format!("CPU {cpu}%"),
                    None => "CPU --".to_owned(),
                };
                format!("RAM {} MB · {cpu}", reading.mib)
            }
            None => String::new(),
        }
    }

    /// Opens Home: the clock is read now.
    /// Home opens again where it was left: its carousel and "Your lists".
    pub(super) fn enter_home(&mut self) -> iced::Task<Message> {
        self.clock = LocalClock::now();
        let mut tasks = vec![iced::widget::operation::scroll_to(
            super::home::CAROUSEL_ID,
            iced::widget::operation::AbsoluteOffset {
                x: self.carousel.offset,
                y: 0.0,
            },
        )];
        for (strip, motion) in &self.strips {
            tasks.push(iced::widget::operation::scroll_to(
                strip.id(),
                iced::widget::operation::AbsoluteOffset {
                    x: motion.offset,
                    y: 0.0,
                },
            ));
        }
        iced::Task::batch(tasks)
    }
}

/// How often the status line reads the process.
const STATUS_INTERVAL: Duration = Duration::from_secs(5);

/// Reads memory and CPU every few seconds. It reports only when a figure the status line shows
/// changes, so an idle Home gets no messages and does not redraw.
pub(super) fn status_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(1, async |mut output| {
        let mut previous: Option<process_stats::ProcessSample> = None;
        let mut shown: Option<ProcessReading> = None;
        loop {
            if let Some(sample) = process_stats::sample() {
                let reading = ProcessReading::from_samples(&sample, previous.as_ref());
                if changed(&mut shown, reading)
                    && output.send(Message::StatusReading(reading)).await.is_err()
                {
                    break;
                }
                previous = Some(sample);
            }
            tokio::time::sleep(STATUS_INTERVAL).await;
        }
    })
}

/// Whether `reading` differs from the figures last reported. Records it when it does.
fn changed(shown: &mut Option<ProcessReading>, reading: ProcessReading) -> bool {
    if *shown == Some(reading) {
        return false;
    }
    *shown = Some(reading);
    true
}

/// The Home clock wakes just after each minute begins, so the minute on screen is never out of
/// date for long. Minute boundaries are the same in every time zone.
pub(super) fn clock_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(1, async |mut output| {
        loop {
            tokio::time::sleep(time_to_next_minute(std::time::SystemTime::now())).await;
            if output.send(Message::ClockTick).await.is_err() {
                break;
            }
        }
    })
}

/// The time from `now` until just after the next minute begins. The margin keeps the wake-up on the
/// new minute when the timer fires a little early.
fn time_to_next_minute(now: std::time::SystemTime) -> Duration {
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let into_minute = (since_epoch.as_millis() % 60_000) as u64;
    Duration::from_millis(60_000 - into_minute + 50)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_line_shows_memory_and_cpu_and_nothing_before_a_reading() {
        let mut app = App::default();
        assert_eq!(app.status_text(), "");
        app.process_reading = Some(ProcessReading {
            mib: 45,
            cpu_percent: Some(1),
        });
        assert_eq!(app.status_text(), "RAM 45 MB · CPU 1%");
    }

    #[test]
    fn a_hovered_chip_is_remembered_until_the_pointer_leaves() {
        let mut app = App::default();
        let _ = app.update(Message::ChipHovered(Some("daft punk".into())));
        assert_eq!(app.hovered_chip.as_deref(), Some("daft punk"));
        let _ = app.update(Message::ChipHovered(None));
        assert_eq!(app.hovered_chip, None);
    }

    #[test]
    fn the_clock_wakes_just_after_each_minute_begins() {
        let at = |millis: u64| std::time::UNIX_EPOCH + Duration::from_millis(millis);
        // A minute begins 120 s after the epoch, so the next one is a whole minute away.
        assert_eq!(
            time_to_next_minute(at(120_000)),
            Duration::from_millis(60_050)
        );
        // Half a second before the next minute, the wake-up follows in half a second.
        assert_eq!(time_to_next_minute(at(179_500)), Duration::from_millis(550));
        assert_eq!(
            time_to_next_minute(at(150_000)),
            Duration::from_millis(30_050)
        );
    }

    #[test]
    fn the_greeting_follows_the_local_hour() {
        let cases = [
            (0, DayPart::Night),
            (4, DayPart::Night),
            (5, DayPart::Morning),
            (11, DayPart::Morning),
            (12, DayPart::Afternoon),
            (17, DayPart::Afternoon),
            (18, DayPart::Evening),
            (21, DayPart::Evening),
            (22, DayPart::Evening),
            (23, DayPart::Evening),
        ];
        for (hour, part) in cases {
            assert_eq!(day_part(hour), part, "hour {hour}");
        }
        assert_eq!(greeting_for(DayPart::Afternoon), "GOOD AFTERNOON.");
        assert_eq!(greeting_for(DayPart::Evening), "GOOD EVENING.");
        assert_eq!(greeting_for(DayPart::Night), "STILL UP?");
    }

    #[test]
    fn the_clock_line_reads_time_then_weekday_day_and_month() {
        let clock = LocalClock {
            hour: 21,
            minute: 4,
            weekday: 4,
            day: 9,
            month: 10,
        };
        assert_eq!(
            clock_line(clock, false, UiLanguage::English),
            "21:04 · THURSDAY 09 OCT"
        );
        assert_eq!(
            clock_line(clock, false, UiLanguage::Spanish),
            "21:04 · JUEVES 09 OCT"
        );
    }

    #[test]
    fn the_clock_reads_twelve_hours_with_a_period_only_where_it_is_asked() {
        let at = |hour| LocalClock {
            hour,
            minute: 5,
            weekday: 4,
            day: 9,
            month: 10,
        };
        assert_eq!(
            clock_line(at(21), true, UiLanguage::English),
            "9:05 PM · THURSDAY 09 OCT"
        );
        assert_eq!(
            clock_line(at(0), true, UiLanguage::English),
            "12:05 AM · THURSDAY 09 OCT"
        );
        assert_eq!(
            clock_line(at(12), true, UiLanguage::English),
            "12:05 PM · THURSDAY 09 OCT"
        );
        assert_eq!(
            clock_line(at(0), false, UiLanguage::English),
            "00:05 · THURSDAY 09 OCT"
        );
        assert!(uses_twelve_hour_clock(UiLanguage::English));
        assert!(!uses_twelve_hour_clock(UiLanguage::Spanish));
    }

    #[test]
    fn the_status_stream_reports_only_changed_figures() {
        let reading = |mib, cpu_percent| ProcessReading { mib, cpu_percent };
        let mut shown = None;
        assert!(changed(&mut shown, reading(70, None)));
        assert!(!changed(&mut shown, reading(70, None)));
        assert!(changed(&mut shown, reading(70, Some(0))));
        assert!(!changed(&mut shown, reading(70, Some(0))));
        assert!(changed(&mut shown, reading(71, Some(0))));
    }

    #[test]
    fn chips_keep_order_and_stop_at_the_first_one_that_does_not_fit() {
        let searches: Vec<String> = ["daft punk", "lofi", "a rather long query here", "x"]
            .map(String::from)
            .to_vec();
        // "daft punk" (9 chars) and "lofi" fit in 200 px; the long query does not.
        assert_eq!(
            chip_queries(&searches, 200.0, false),
            vec!["daft punk".to_string(), "lofi".to_string()]
        );
        // Phones scroll the row, so every chip counts, up to the limit.
        assert_eq!(chip_queries(&searches, 200.0, true).len(), 4);
        let many: Vec<String> = (0..9).map(|i| format!("q{i}")).collect();
        assert_eq!(chip_queries(&many, 2_000.0, false).len(), MAX_CHIPS);
        assert!(chip_queries(&[], 800.0, false).is_empty());
    }
}
