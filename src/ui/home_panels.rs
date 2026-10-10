//! The Home panels around the carousel: the greeting, recent searches, "Jump back in" and the
//! status line. `HomeLayout` decides which of them fit; this module draws them.

use super::home::{
    CHIPS_H, GREETING_GAP, GREETING_SUBLINE_GAP, GreetingSize, HomeLayout, JUMP_HEADING_GAP,
    JUMP_ROW_GAP, MAX_CHIPS, STATUS_H,
};
use super::shell::{fit, section_heading};
use super::theme::{self, DIM, FADED, MONO, MONO_BOLD, RULE, hrule, label};
use super::{App, Message, format_duration};
use crate::local_clock::LocalClock;
use crate::preferences::RecentPlay;
use crate::process_stats::{self, ProcessReading};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding};
use std::sync::Arc;
use std::time::Duration;

/// Edge of the thumbnails in "Jump back in".
pub(super) const JUMP_THUMB: f32 = 36.0;
/// Width of the duration or track count at the end of a "Jump back in" entry.
const JUMP_META_W: f32 = 96.0;
/// Gap between the two columns of "Jump back in".
const JUMP_COLUMN_GAP: f32 = 16.0;
const CHIP_TEXT_SIZE: f32 = 12.0;
/// Estimated advance of one monospace character at chip size, and the chip's padding and border.
const CHIP_CHAR_W: f32 = 8.0;
const CHIP_PADDING: f32 = 26.0;
const CHIP_GAP: f32 = 8.0;

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

/// The part of the day the greeting names. Night runs from 22:00 to 04:59.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DayPart {
    Morning,
    Afternoon,
    Evening,
    Night,
}

pub(super) fn day_part(hour: u8) -> DayPart {
    match hour {
        5..=11 => DayPart::Morning,
        12..=17 => DayPart::Afternoon,
        18..=21 => DayPart::Evening,
        _ => DayPart::Night,
    }
}

fn greeting_for(part: DayPart) -> &'static str {
    match part {
        DayPart::Morning => "GOOD MORNING.",
        DayPart::Afternoon => "GOOD AFTERNOON.",
        DayPart::Evening => "GOOD EVENING.",
        DayPart::Night => "GOOD NIGHT.",
    }
}

/// "21:14 — THURSDAY 09 OCT", with the weekday and month already translated.
fn clock_line(clock: LocalClock, weekday: &str, month: &str) -> String {
    format!(
        "{:02}:{:02} — {} {:02} {}",
        clock.hour, clock.minute, weekday, clock.day, month
    )
}

/// The recent searches that get a chip, in order and at most `MAX_CHIPS`. On a desktop width
/// only the ones that fit on one line; on a phone the row scrolls, so all of them count.
pub(super) fn chip_queries(searches: &[String], width: f32, narrow: bool) -> Vec<String> {
    let mut chips: Vec<String> = Vec::new();
    let mut used = 0.0;
    for query in searches.iter().take(MAX_CHIPS) {
        let chip = query.chars().count() as f32 * CHIP_CHAR_W + CHIP_PADDING;
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
        let weekday = self.tr_static(WEEKDAYS[usize::from(clock.weekday % 7)]);
        let month = self.tr_static(MONTHS[usize::from(clock.month.clamp(1, 12) - 1)]);
        let stamp = clock_line(clock, weekday, month);
        let greeting = self.tr_static(greeting_for(day_part(clock.hour)));
        let content: Element<'_, Message> = match layout.greeting {
            GreetingSize::Full => column![
                text(greeting).size(layout.greeting_font).font(MONO_BOLD),
                Space::new().height(GREETING_SUBLINE_GAP),
                label(stamp, 13.0).color(DIM),
            ]
            .into(),
            GreetingSize::Small => row![
                text(greeting).size(14.0).font(MONO_BOLD),
                label(stamp, 12.0).color(DIM),
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

    /// The recent searches as outlined chips. A click searches for that query again.
    pub(super) fn recent_chips_view(&self, layout: &HomeLayout) -> Element<'_, Message> {
        let chips: Vec<Element<'_, Message>> =
            chip_queries(&self.search_history, layout.search_width, layout.narrow)
                .into_iter()
                .take(layout.chips)
                .map(|query| {
                    let shown = fit(&query, layout.search_width * 0.8, CHIP_TEXT_SIZE);
                    button(text(shown).size(CHIP_TEXT_SIZE).font(MONO))
                        .on_press(Message::SearchFor(query))
                        .padding([6, 12])
                        .style(theme::outline_button)
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
        for chunk in entries.chunks(per_row) {
            let cells: Vec<Element<'_, Message>> = chunk
                .iter()
                .map(|play| self.jump_entry(play, cell_width))
                .collect();
            grid = grid.push(row(cells).spacing(JUMP_COLUMN_GAP));
        }
        let heading = section_heading(self.tr_static("JUMP BACK IN").to_owned(), None);
        container(
            column![heading, Space::new().height(JUMP_HEADING_GAP), grid]
                .width(layout.search_width),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    }

    fn jump_entry(&self, play: &RecentPlay, width: f32) -> Element<'_, Message> {
        let variant = crate::artwork::variant_for_pixels(JUMP_THUMB * self.pixel_scale());
        let (url, title, detail, meta, action) = match play {
            RecentPlay::Track(track) => (
                track.thumbnail_url.as_deref(),
                track.title.clone(),
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
                if collection.track_count == 1 {
                    self.tr_static("1 song").to_owned()
                } else {
                    self.tr_string(format!("{} songs", collection.track_count))
                },
                Message::OpenCollection(collection.browse.clone()),
            ),
        };
        let line = if detail.trim().is_empty() {
            title
        } else {
            format!("{title} — {detail}")
        };
        let text_width = (width - JUMP_THUMB - JUMP_META_W - 36.0).max(40.0);
        button(
            row![
                self.cover(url, JUMP_THUMB, variant),
                container(label(fit(&line, text_width, 13.0), 13.0)).width(Length::Fill),
                label(meta, 12.0).color(DIM),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(action)
        .width(Length::Fill)
        .padding([2, 6])
        .style(theme::row_button)
        .into()
    }

    /// The status line at the foot of Home: memory and CPU of this process, and the number of
    /// favorites on the left; the version on the right.
    pub(super) fn status_line(&self) -> Element<'_, Message> {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        column![
            hrule(RULE),
            Space::new().height(8.0),
            row![
                label(self.status_text(), 11.0).color(FADED),
                Space::new().width(Length::Fill),
                label(version, 11.0).color(FADED),
            ]
            .align_y(Alignment::Center),
        ]
        .width(Length::Fill)
        .height(STATUS_H)
        .into()
    }

    fn status_text(&self) -> String {
        let favorites = format!("{} {}", self.favorites.len(), self.tr_static("FAVORITES"));
        match self.process_reading {
            Some(reading) => {
                let cpu = match reading.cpu_percent {
                    Some(cpu) => format!("CPU {cpu:.1}%"),
                    None => "CPU --".to_owned(),
                };
                format!("RAM {} MB · {cpu} · {favorites}", reading.pss_kib / 1024)
            }
            None => favorites,
        }
    }

    /// Opens Home: the clock is read now, and the process baseline restarts so the first CPU
    /// figure covers only time spent on Home.
    pub(super) fn enter_home(&mut self) {
        self.clock = LocalClock::now();
        self.process_sample = None;
        self.refresh_process_status();
    }

    /// Reads the process memory and CPU for the status line. The CPU figure covers the time
    /// since the previous reading, so the first reading after `enter_home` shows memory only.
    pub(super) fn refresh_process_status(&mut self) {
        let Some(sample) = process_stats::sample() else {
            self.process_reading = None;
            self.process_sample = None;
            return;
        };
        let cpu_percent = self
            .process_sample
            .and_then(|previous| process_stats::cpu_percent(&previous, &sample));
        self.process_reading = Some(ProcessReading {
            pss_kib: sample.pss_kib,
            cpu_percent,
        });
        self.process_sample = Some(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            (22, DayPart::Night),
            (23, DayPart::Night),
        ];
        for (hour, part) in cases {
            assert_eq!(day_part(hour), part, "hour {hour}");
        }
        assert_eq!(greeting_for(DayPart::Afternoon), "GOOD AFTERNOON.");
        assert_eq!(greeting_for(DayPart::Night), "GOOD NIGHT.");
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
            clock_line(clock, "THURSDAY", "OCT"),
            "21:04 — THURSDAY 09 OCT"
        );
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
