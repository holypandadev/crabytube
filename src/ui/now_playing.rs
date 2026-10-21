//! Now Playing: large artwork with transport on the left, Up Next and Lyrics
//! on the right.

use super::drag::{drag_area, drag_row};
use super::messages::DragList;
use super::playlists::HEART_W;
use super::shell::{display_title, fit};
use super::theme::{
    self, BLACK, DIM, DIM_ON_LIGHT, FADED, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_SECTION,
    TEXT_TITLE, UNSUNG_ON_LIGHT, hrule, icon, label,
};
use super::{App, Message, NowPlayingTab, format_duration, layout};
use crate::preferences::UiDensity;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Size};
use unicode_width::UnicodeWidthStr;
const PANEL_PADDING: f32 = 16.0;
const COLUMN_GAP: f32 = 40.0;
const STACK_BELOW: f32 = 760.0;
/// Room beside the clear button's label for its padding (12 px each side), its border and the gap
/// before it, so a queue notice never runs under the button.
const CLEAR_BUTTON_EXTRA: f32 = 30.0;
/// The row under the tabs with the queue count and the clear button.
const QUEUE_HEADER_H: f32 = 40.0;

/// Height of everything in the artwork column but the artwork: the title block, the seek row and
/// the transport, with their gaps.
const ART_CHROME_H: f32 = 190.0;
/// Icon size of the Now Playing transport; play is a little larger.
const TRANSPORT_ICON: f32 = 22.0;
/// Width of that transport: five buttons with 7 px padding (icons of 18, 22, 28, 22 and 18 px)
/// and four 14 px gaps.
const TRANSPORT_W: f32 = 234.0;
/// The favorite and volume buttons on either side of the transport, and the gap to it.
const END_BUTTON_W: f32 = 32.0;
const GROUP_GAP: f32 = 14.0;
const END_ICON: f32 = 18.0;
/// Gap between the volume rail and its button.
const RAIL_GAP: f32 = 8.0;
/// A rail shorter than this opens on a row of its own under the controls.
const RAIL_MIN: f32 = 40.0;
/// How long the volume rail takes to open or close, and how long it stays open untouched.
const RAIL_EASE_MS: f32 = 160.0;
pub(super) const RAIL_IDLE: std::time::Duration = std::time::Duration::from_secs(2);

/// The volume rail of Now Playing. It opens from the volume button, eases in and out, and closes
/// once it has not been touched for `RAIL_IDLE`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct VolumeRail {
    /// How far it is open: 0 closed, 1 open.
    pub(super) amount: f32,
    from: f32,
    to: f32,
    started: Option<std::time::Instant>,
    /// When it was last opened or moved.
    touched: Option<std::time::Instant>,
    /// A check for closing it is on its way.
    pub(super) check_pending: bool,
}

impl VolumeRail {
    pub(super) fn is_open(&self) -> bool {
        self.to > 0.5
    }

    pub(super) fn animating(&self) -> bool {
        self.started.is_some()
    }

    fn ease_to(&mut self, to: f32, now: std::time::Instant, reduced_motion: bool) {
        self.to = to;
        if reduced_motion {
            self.amount = to;
            self.started = None;
        } else {
            self.from = self.amount;
            self.started = Some(now);
        }
    }

    pub(super) fn open(&mut self, now: std::time::Instant, reduced_motion: bool) {
        self.touched = Some(now);
        self.ease_to(1.0, now, reduced_motion);
    }

    pub(super) fn close(&mut self, now: std::time::Instant, reduced_motion: bool) {
        self.touched = None;
        self.ease_to(0.0, now, reduced_motion);
    }

    /// Moving the rail or pressing its button keeps it open for another `RAIL_IDLE`.
    pub(super) fn touch(&mut self, now: std::time::Instant) {
        if self.is_open() {
            self.touched = Some(now);
        }
    }

    /// Time left before the rail closes, or `None` when it should close now.
    pub(super) fn idle_left(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        let touched = self.touched?;
        RAIL_IDLE
            .checked_sub(now.saturating_duration_since(touched))
            .filter(|left| !left.is_zero())
    }

    pub(super) fn step(&mut self, now: std::time::Instant) {
        let Some(started) = self.started else {
            return;
        };
        let elapsed_ms = now.saturating_duration_since(started).as_secs_f32() * 1000.0;
        let t = (elapsed_ms / RAIL_EASE_MS).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.amount = self.from + (self.to - self.from) * eased;
        if t >= 1.0 {
            self.amount = self.to;
            self.started = None;
        }
    }
}

/// Edge of the artwork and width of the side panel, or `None` when the page is stacked. The
/// artwork starts at the page margin and the panel takes the rest after the gap.
fn now_playing_geometry(area: Size) -> Option<(f32, f32)> {
    let content = area.width.max(320.0);
    if content < STACK_BELOW {
        return None;
    }
    let widest = ((content - COLUMN_GAP) * 0.48).floor();
    let height = (area.height - 24.0).max(240.0);
    let art = widest.min(height - ART_CHROME_H).max(160.0).floor();
    Some((art, content - COLUMN_GAP - art))
}

/// A short message about the queue, shown above it until the queue changes again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum QueueNotice {
    Limited(usize),
}

impl App {
    pub(super) fn now_playing_panel_width(&self) -> f32 {
        now_playing_geometry(self.area).map_or(self.area.width.max(320.0), |(_, right)| right)
    }

    /// Width available to one lyric line inside the panel.
    pub(super) fn lyrics_text_width(&self) -> f32 {
        (self.now_playing_panel_width() - 2.0 * PANEL_PADDING - 24.0 - 10.0).max(160.0)
    }

    /// Height of the lists, from the measured page area.
    pub(super) fn page_height(&self) -> f32 {
        (self.area.height - 24.0).max(240.0)
    }

    pub(super) fn now_playing_page(&self, area: Size) -> Element<'_, Message> {
        let Some(track) = self.shown_track() else {
            return column![
                label(self.tr("[ NOTHING PLAYING ]").into_owned(), TEXT_BODY),
                label(
                    self.tr("Choose a song from Home or Favorite songs.")
                        .into_owned(),
                    TEXT_CAPTION
                )
                .color(DIM),
            ]
            .spacing(8)
            .into();
        };
        match now_playing_geometry(area) {
            Some((art, right_width)) => {
                let mut left = column![
                    self.cover(
                        track.thumbnail_url.as_deref(),
                        art,
                        crate::artwork::LARGE_MAX
                    ),
                    Space::new().height(16),
                    self.track_header(track, art),
                    Space::new().height(14),
                    container(self.seek_row(TEXT_CAPTION)).width(art),
                ]
                .width(art);
                left = left.push(self.controls_row(track, art));
                row![
                    container(left).width(art).height(Length::Fill),
                    container(self.side_panel(right_width))
                        .width(right_width)
                        .height(Length::Fill),
                ]
                .spacing(COLUMN_GAP)
                .padding(iced::Padding::default().bottom(24.0))
                .into()
            }
            None => {
                let width = area.width.max(320.0);
                column![
                    row![
                        self.cover(
                            track.thumbnail_url.as_deref(),
                            112.0,
                            crate::artwork::CARD_MID_MAX
                        ),
                        container(self.track_header(track, width - 132.0)).width(Length::Fill),
                    ]
                    .spacing(20)
                    .align_y(Alignment::Center),
                    self.seek_row(TEXT_CAPTION),
                    self.controls_row(track, width),
                    container(self.side_panel(width)).height(Length::Fill),
                ]
                .spacing(12)
                .padding(iced::Padding::default().bottom(24.0))
                .into()
            }
        }
    }

    fn track_header<'a>(
        &'a self,
        track: &'a crate::music::Track,
        width: f32,
    ) -> Element<'a, Message> {
        let text_width = width;
        let mut meta = Vec::new();
        if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
            meta.push(album.to_owned());
        }
        // Only an album or playlist the queue was started from; search and
        // radio queues have no meaningful source to show.
        if let Some(source) = &self.queue_source {
            meta.push(source.title.clone());
            meta.push(self.song_count(source.track_count, source.more));
        }
        // A failed track says why here too: this page has no player bar to show it.
        let details = match self.playback_failure() {
            Some(failure) => {
                label(fit(&failure, text_width, TEXT_CAPTION), TEXT_CAPTION).color(theme::ALERT)
            }
            None => label(
                fit(&meta.join(" · "), text_width, TEXT_CAPTION),
                TEXT_CAPTION,
            )
            .color(DIM),
        };
        column![
            column![
                text(fit(display_title(&track.title), text_width, TEXT_TITLE))
                    .size(TEXT_TITLE)
                    .font(MONO),
                self.artist_link(track, text_width, TEXT_SECTION, INK),
                details,
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .width(width)
        .into()
    }

    /// One centred group: favorite, the transport and the volume button, each next to the other.
    /// The volume rail opens to the right of its button, into the free side of the row, so the
    /// group does not move; where that side has no room the rail opens on a row under it.
    fn controls_row<'a>(
        &'a self,
        track: &'a crate::music::Track,
        width: f32,
    ) -> Element<'a, Message> {
        let group_w = TRANSPORT_W + 2.0 * (END_BUTTON_W + GROUP_GAP);
        let room = ((width - group_w) / 2.0 - RAIL_GAP)
            .floor()
            .min(super::shell::VOLUME_W);
        let beside = room >= RAIL_MIN;
        let open = self.volume_rail.amount > 0.01;
        let muted = self.preferences.muted;
        let volume_button = button(icon(
            if muted { "volume-off" } else { "volume" },
            END_ICON,
            INK,
        ))
        .on_press(Message::ToggleVolumeRail)
        .padding(7)
        .style(if self.volume_rail.is_open() {
            theme::icon_button_active
        } else {
            theme::icon_button
        });
        let group = row![
            self.like_button(Some(track), END_ICON),
            self.transport_controls(TRANSPORT_ICON, true),
            super::keyboard::wheel_volume(volume_button),
        ]
        .spacing(GROUP_GAP)
        .align_y(Alignment::Center);
        let mut right = row![Space::new().width(RAIL_GAP)].align_y(Alignment::Center);
        if beside && open {
            right = right.push(self.volume_rail_slider(room * self.volume_rail.amount));
        }
        let line = row![
            Space::new().width(Length::Fill),
            group,
            container(right).width(Length::Fill),
        ]
        .width(width)
        .align_y(Alignment::Center);
        if beside || !open {
            return line.into();
        }
        column![
            line,
            container(self.volume_rail_slider(super::shell::VOLUME_W * self.volume_rail.amount))
                .center_x(width),
        ]
        .spacing(6)
        .into()
    }

    fn volume_rail_slider(&self, width: f32) -> Element<'_, Message> {
        super::keyboard::wheel_volume(
            iced::widget::slider(0.0..=1.0, self.preferences.volume, Message::SetVolume)
                .on_release(Message::PersistState)
                .step(0.01_f32)
                .width(width.max(1.0))
                .height(theme::SLIDER_H)
                .style(theme::seek_style),
        )
    }

    fn side_panel(&self, width: f32) -> Element<'_, Message> {
        let tab = |title: &'static str, which: NowPlayingTab| {
            button(
                container(label(self.tr(title).into_owned(), TEXT_BODY))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(Message::SetNowPlayingTab(which))
            .width(Length::Fill)
            .height(38)
            .style(theme::tab_style(self.now_playing_tab == which))
        };
        // One frame: the panel's outline. The tabs have none of their own; the active one is
        // filled.
        let tabs = row![
            tab("Up next", NowPlayingTab::Queue),
            tab("Lyrics", NowPlayingTab::Lyrics),
        ]
        .align_y(Alignment::Center);
        let body = match self.now_playing_tab {
            NowPlayingTab::Queue => self.up_next(width - 2.0 * PANEL_PADDING),
            NowPlayingTab::Lyrics => self.lyrics_view(),
        };
        container(
            column![
                container(tabs).padding(PANEL_PADDING),
                container(body)
                    .padding([0.0, PANEL_PADDING])
                    .height(Length::Fill),
            ]
            .height(Length::Fill),
        )
        .width(width)
        .height(Length::Fill)
        .padding(theme::hairline())
        .style(theme::boxed)
        .into()
    }

    /// The notice in the current language.
    fn queue_notice_text(&self, notice: QueueNotice) -> String {
        match notice {
            QueueNotice::Limited(entries) => self
                .tr("Queue limited to {} entries; the selected track stays first.")
                .replacen("{}", &entries.to_string(), 1),
        }
    }

    fn up_next(&self, width: f32) -> Element<'_, Message> {
        if self.queue.is_empty() {
            return container(
                label(self.tr("The queue is empty.").into_owned(), TEXT_BODY).color(DIM),
            )
            .padding([16, 0])
            .into();
        }
        let density = self.preferences.ui_density;
        let pitch = density.queue_row_pitch();
        let cover = density.queue_cover();
        let padding = UiDensity::row_padding(cover, pitch);
        let range = layout::virtual_range(
            self.queue.len(),
            self.queue_scroll.y,
            self.page_height(),
            pitch,
            6,
        );
        let mut rows = column![].spacing(0).width(Length::Fill);
        if range.start > 0 {
            rows =
                rows.push(Space::new().height(layout::virtual_spacer_height(range.start, pitch)));
        }
        // On a phone the rows leave out the length and sit closer, so the title keeps its room
        // beside the buttons a finger needs.
        let mobile = self.mobile();
        let gap = if mobile { 12.0 } else { 20.0 };
        let text_width = if mobile {
            width - cover - 3.0 * gap - self.row_more_width() - HEART_W - 24.0
        } else {
            width - cover - 20.0 - 64.0 - 44.0 - 24.0 - HEART_W - 20.0
        };
        let dragged = self.dragged_row(DragList::Queue);
        for index in range.clone() {
            let entry = &self.queue[index];
            let current = self.queue_index == Some(index);
            let duration = self
                .duration_of(entry)
                .map(format_duration)
                .unwrap_or_default();
            let menu = super::messages::RowMenu {
                list: "queue",
                index,
            };
            let open = self.row_menu.as_ref() == Some(&menu);
            // The row under the keyboard cursor is outlined; the playing row stays inverted.
            let cursor = self.row_is_cursor("queue", index);
            let mut ink = theme::row_ink(current);
            // The row being dragged stays in its place, faded, while a label follows the pointer.
            if dragged == Some(index) {
                ink.title = theme::FADED;
                ink.secondary = theme::FADED;
            }
            let title = display_title(&entry.title);
            let title = if current {
                format!("▶ {title}")
            } else {
                title.to_owned()
            };
            let more = button(super::theme::icon("more", 18.0, ink.icon))
                .on_press(self.row_click(DragList::Queue, Message::ToggleRowMenu(menu)))
                .padding(self.row_icon_padding(cover))
                .style(theme::row_icon_button(current, open));
            let mut names = column![
                text(fit(&title, text_width, TEXT_BODY))
                    .size(TEXT_BODY)
                    .font(MONO)
                    .color(ink.title),
            ];
            // A track without an artist (an album's, before it plays) keeps its title alone, centred,
            // rather than over an empty line.
            if !entry.artist.trim().is_empty() {
                names = names.push(
                    label(fit(&entry.artist, text_width, TEXT_CAPTION), TEXT_CAPTION)
                        .color(ink.secondary),
                );
            }
            let mut cells = row![
                self.cover(
                    entry.thumbnail_url.as_deref(),
                    cover,
                    crate::artwork::THUMB_MAX
                ),
                names.spacing(6).width(Length::Fill),
            ]
            .spacing(gap)
            .align_y(Alignment::Center);
            if !mobile {
                cells = cells.push(label(duration, TEXT_CAPTION).color(ink.secondary));
            }
            let cells = cells.push(more).push(self.row_heart(
                entry,
                ink.icon,
                current,
                cover,
                self.row_click(DragList::Queue, Message::OpenSaveDialog(entry.clone())),
            ));
            let main = button(cells)
                .on_press(self.row_click(DragList::Queue, Message::PlayQueueIndex(index)))
                .width(Length::Fill)
                .padding([padding, 8.0])
                .style(if current {
                    theme::row_button_current
                } else if cursor {
                    theme::row_button_cursor
                } else {
                    theme::row_button
                });
            rows = rows.push(super::keyboard::mark_row(
                "queue",
                index,
                drag_row(
                    column![main, hrule(RULE)],
                    Some(Message::DragPressed {
                        list: DragList::Queue,
                        index,
                    }),
                    Some(Message::DragHeld {
                        list: DragList::Queue,
                        index,
                    }),
                ),
            ));
        }
        let remaining = self.queue.len().saturating_sub(range.end);
        if remaining > 0 {
            rows = rows.push(Space::new().height(layout::virtual_spacer_height(remaining, pitch)));
        }
        if self.radio_loading {
            rows = rows.push(
                container(
                    label(self.tr("[ LOADING RADIO ]").into_owned(), TEXT_CAPTION).color(DIM),
                )
                .padding([12, 0]),
            );
        }
        if let Some(error) = &self.radio_error {
            rows = rows.push(
                row![
                    label(
                        fit(&self.tr(error), width - 120.0, TEXT_CAPTION),
                        TEXT_CAPTION
                    )
                    .color(DIM),
                    button(label(self.tr("Retry").into_owned(), TEXT_CAPTION))
                        .on_press(Message::RetryRadio)
                        .padding([6, 12])
                        .style(theme::outline_button),
                ]
                .spacing(12)
                .padding([12, 0])
                .align_y(Alignment::Center),
            );
        }
        let upcoming = self
            .queue
            .len()
            .saturating_sub(self.queue_index.map_or(0, |current| current + 1));
        // The notice takes the room left of the button and is cut short, never wrapped, so the list
        // below keeps its place. The room follows the translated label's width.
        let clear_label = self.tr("Clear up next").into_owned();
        let save_label = self.tr("Save as playlist").into_owned();
        let clear_width = (clear_label.width() + save_label.width()) as f32 * TEXT_CAPTION * 0.6
            + 2.0 * CLEAR_BUTTON_EXTRA;
        // The count of what comes next, or a notice about the last change to the queue while one
        // is set. Either is cut short, never wrapped, so the list keeps its place.
        let status = match self.queue_notice {
            Some(notice) => self.queue_notice_text(notice),
            None => self
                .tr_static("{} up next")
                .replacen("{}", &upcoming.to_string(), 1),
        };
        let header = container(
            row![
                container(
                    label(
                        fit(&status, width - clear_width - 16.0, TEXT_CAPTION),
                        TEXT_CAPTION
                    )
                    .color(DIM)
                    .wrapping(iced::widget::text::Wrapping::None)
                )
                .width(Length::Fill)
                .clip(true),
                button(label(save_label, TEXT_CAPTION))
                    .on_press_maybe((!self.queue.is_empty()).then_some(Message::SaveQueue))
                    .padding([6, 12])
                    .style(theme::outline_button),
                button(label(clear_label, TEXT_CAPTION))
                    .on_press_maybe((upcoming > 0).then_some(Message::ClearUpcomingQueue))
                    .padding([6, 12])
                    .style(theme::outline_button),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(QUEUE_HEADER_H)
        .center_y(QUEUE_HEADER_H);
        let mut list = iced::widget::stack![
            scrollable(rows)
                .id("queue-content")
                .on_scroll(|viewport| Message::QueueScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill)
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        if let Some(overlay) = self.drag_overlay(DragList::Queue, width) {
            list = list.push(overlay);
        }
        let list = drag_area(list, self.drag_moves(DragList::Queue));
        column![header, list].spacing(8).height(Length::Fill).into()
    }

    /// The provider name, marked when the text has no timing so it is not taken for synced lyrics.
    fn lyrics_source_label(&self, lyrics: &crate::lyrics::Lyrics) -> String {
        if lyrics.lines.is_empty() {
            format!("{} · {}", lyrics.source, self.tr("Not synced"))
        } else {
            lyrics.source.clone()
        }
    }

    /// Lyrics list; the active line is drawn as an inverted bar.
    fn lyrics_view(&self) -> Element<'_, Message> {
        if self.lyrics_loading && self.lyrics.is_none() {
            return container(
                label(self.tr("[ LOADING LYRICS ]").into_owned(), TEXT_BODY).color(DIM),
            )
            .padding([16, 0])
            .into();
        }
        let Some(lyrics) = &self.lyrics else {
            // A restored track whose length is not known has no lyrics until playback starts;
            // nothing is fetched at startup.
            if self.playback_status.track.is_none() && self.lyrics_error.is_none() {
                return label(
                    self.tr("Press play to load the lyrics.").into_owned(),
                    TEXT_BODY,
                )
                .color(DIM)
                .into();
            }
            return column![
                label(
                    self.lyrics_error
                        .clone()
                        .map(|error| self.tr_string(error))
                        .unwrap_or_else(|| self.tr("No lyrics for this song.").into_owned()),
                    TEXT_BODY
                )
                .color(DIM),
                button(label(self.tr("Retry").into_owned(), TEXT_CAPTION))
                    .on_press(Message::RetryLyrics)
                    .padding([6, 12])
                    .style(theme::outline_button),
            ]
            .spacing(12)
            .padding([16, 0])
            .into();
        };
        let size = self.preferences.lyrics_font_size;
        let mut lines = column![].spacing(2);
        let (range, top_spacer, bottom_spacer) = crate::lyrics::virtual_window(
            &self.lyrics_heights,
            self.lyrics_scroll.y,
            self.page_height(),
            8,
        );
        if top_spacer > 0.0 {
            lines = lines.push(Space::new().height(top_spacer));
        }
        let seconds = self.effective_lyrics_position();
        for index in range {
            let line = &lyrics.lines[index];
            let active = self.lyric_active == Some(index);
            let content: Element<'_, Message> = if active && !line.words.is_empty() {
                iced::widget::rich_text::<(), Message, iced::Theme, iced::Renderer>(
                    line.words
                        .iter()
                        .map(|word| {
                            iced::widget::span(word.text.clone()).font(MONO).color(
                                if seconds >= word.start {
                                    BLACK
                                } else {
                                    UNSUNG_ON_LIGHT
                                },
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .size(size)
                .into()
            } else {
                text(line.text.clone())
                    .size(size)
                    .font(MONO)
                    .color(if active { BLACK } else { INK })
                    .into()
            };
            // Romanization and translation sit under the line with the translation switch;
            // background vocals stay apart from the main text, always in the dim style.
            let show_translation = self.preferences.lyrics_show_translation;
            let sub_color = if active { DIM_ON_LIGHT } else { DIM };
            let mut body = column![content].spacing(2);
            if show_translation && let Some(romanization) = line.romanization.as_deref() {
                body = body.push(
                    text(romanization.to_owned())
                        .size(TEXT_CAPTION)
                        .font(MONO)
                        .color(sub_color),
                );
            }
            if show_translation
                && let Some(translation) = line
                    .translation
                    .as_deref()
                    .filter(|translation| !translation.trim().is_empty())
            {
                body = body.push(
                    text(translation.to_owned())
                        .size(TEXT_CAPTION)
                        .font(MONO)
                        .color(sub_color),
                );
            }
            if let Some(background) = line.background.as_deref() {
                body = body.push(
                    text(background.to_owned())
                        .size(TEXT_CAPTION)
                        .font(MONO)
                        .color(sub_color),
                );
            }
            let body: Element<'_, Message> = body.into();
            lines = lines.push(
                button(body)
                    .on_press(Message::SeekTo(line.start))
                    .width(Length::Fill)
                    .padding([4, 12])
                    .style(if active {
                        theme::inverted_button
                    } else {
                        theme::row_button
                    }),
            );
        }
        if bottom_spacer > 0.0 {
            lines = lines.push(Space::new().height(bottom_spacer));
        }
        // Plain text is shown only when there is no synced text, so the two never mix. Each line takes the
        // row padding and spacing of a synced line, so unsynced text keeps the same reading rhythm.
        if let Some(plain) = lyrics.plain.as_ref().filter(|_| lyrics.lines.is_empty()) {
            for plain_line in plain.lines() {
                // A blank line keeps its height, so the gaps between stanzas stay.
                let shown = if plain_line.trim().is_empty() {
                    " "
                } else {
                    plain_line
                };
                lines = lines
                    .push(container(text(shown.to_owned()).size(size).font(MONO)).padding([4, 12]));
            }
        }
        // The source, and for timed lyrics the controls that move this song's lyrics by half a second.
        let source = self.lyrics_source_label(lyrics);
        let earlier = self.tr("Earlier").into_owned();
        let later = self.tr("Later").into_owned();
        let offset = format!("{:+.1} s", self.current_lyrics_offset() as f64 / 1000.0);
        // In a narrow panel the controls go under the source: on one line a long source such as
        // "QQ Music via Better Lyrics" pushed "Later" out of sight.
        let one_line = {
            use unicode_width::UnicodeWidthStr;
            let characters = source.width() + earlier.width() + later.width() + offset.width();
            let needed = characters as f32 * TEXT_CAPTION * 0.6 + 2.0 * 26.0 + 4.0 * 12.0 + 24.0;
            needed <= self.lyrics_text_width()
        };
        let source = label(source, TEXT_CAPTION).color(FADED);
        let header: Element<'_, Message> = if lyrics.lines.is_empty() {
            source.into()
        } else {
            let controls = row![
                button(label(earlier, TEXT_CAPTION))
                    .on_press(Message::AdjustLyricsOffset(-500))
                    .padding([6, 12])
                    .style(theme::outline_button),
                label(offset, TEXT_CAPTION).color(FADED),
                button(label(later, TEXT_CAPTION))
                    .on_press(Message::AdjustLyricsOffset(500))
                    .padding([6, 12])
                    .style(theme::outline_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            if one_line {
                row![source, Space::new().width(Length::Fill), controls]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into()
            } else {
                column![source, controls].spacing(8).into()
            }
        };
        let mut heading = column![container(header).padding([10, 12])];
        if lyrics.length_mismatch {
            heading = heading.push(
                container(
                    label(
                        self.tr("Timed lyrics don't match this version")
                            .into_owned(),
                        TEXT_CAPTION,
                    )
                    .color(FADED),
                )
                .padding([0, 12]),
            );
        }
        let mut content = column![
            heading,
            scrollable(lines)
                .id("lyrics")
                .on_scroll(|viewport| Message::LyricsScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill),
        ]
        .height(Length::Fill);
        if self.lyrics_follow_paused && self.preferences.lyrics_follow {
            content = content.push(
                container(
                    button(label(
                        self.tr("Return to current line").into_owned(),
                        TEXT_CAPTION,
                    ))
                    .on_press(Message::ResumeLyricsFollow)
                    .padding([6, 12])
                    .style(theme::outline_button),
                )
                .padding([10, 0]),
            );
        }
        content.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_artwork_starts_at_the_margin_and_the_panel_takes_the_rest() {
        let area = Size::new(1354.0, 669.0);
        let (art, panel) = now_playing_geometry(area).unwrap();
        assert_eq!(art + COLUMN_GAP + panel, area.width);
        // The artwork leaves room for the title block, the seek row and the transport.
        assert!(art + ART_CHROME_H <= area.height - 24.0 + 1.0, "{art}");
    }

    #[test]
    fn the_volume_rail_opens_eases_and_closes_after_two_untouched_seconds() {
        let start = std::time::Instant::now();
        let mut rail = VolumeRail::default();
        assert!(!rail.is_open());
        rail.open(start, false);
        assert!(rail.is_open() && rail.animating());
        rail.step(start + std::time::Duration::from_millis(80));
        assert!(rail.amount > 0.0 && rail.amount < 1.0, "{}", rail.amount);
        rail.step(start + std::time::Duration::from_millis(200));
        assert_eq!(rail.amount, 1.0);
        assert!(!rail.animating());
        // Moving it a second later keeps it open two more seconds.
        let moved = start + std::time::Duration::from_secs(1);
        rail.touch(moved);
        assert!(
            rail.idle_left(start + std::time::Duration::from_secs(2))
                .is_some()
        );
        assert_eq!(rail.idle_left(moved + RAIL_IDLE), None);
        rail.close(moved + RAIL_IDLE, true);
        assert_eq!(rail.amount, 0.0);
        assert!(!rail.is_open());
    }

    #[test]
    fn narrow_pages_stack() {
        assert!(now_playing_geometry(Size::new(600.0, 800.0)).is_none());
    }
}
