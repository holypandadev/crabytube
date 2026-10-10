//! Now Playing: large artwork with transport on the left, Up Next and Lyrics
//! on the right.

use super::shell::fit;
use super::theme::{self, BLACK, DIM, FADED, INK, MONO, RULE, hrule, label, vrule};
use super::{App, Message, NowPlayingTab, format_duration, layout};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Length, Size};

/// Row pitch of the Up Next list: 64 px cover, 8 px padding above and below, 1 px rule.
pub(super) const QUEUE_ROW_PITCH: f32 = 81.0;
const QUEUE_COVER: f32 = 64.0;
const PANEL_PADDING: f32 = 16.0;
const COLUMN_GAP: f32 = 40.0;
const STACK_BELOW: f32 = 760.0;

/// Widths of the artwork column and the side panel, or `None` when stacked.
fn now_playing_columns(content: f32) -> Option<(f32, f32)> {
    if content < STACK_BELOW {
        return None;
    }
    let left = ((content - COLUMN_GAP) * 0.48).floor();
    Some((left, content - COLUMN_GAP - left))
}

impl App {
    pub(super) fn now_playing_panel_width(&self) -> f32 {
        let width = self.area.width.max(320.0);
        now_playing_columns(width).map_or(width, |(_, right)| right)
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
        let Some(track) = self.playback_status.track.as_ref() else {
            return column![
                label(self.tr("[ NOTHING PLAYING ]").into_owned(), 14.0),
                label(
                    self.tr("Choose a song from Home or Favorite Songs.")
                        .into_owned(),
                    12.0
                )
                .color(DIM),
            ]
            .spacing(8)
            .into();
        };
        let height = (area.height - 24.0).max(240.0);
        match now_playing_columns(area.width.max(320.0)) {
            Some((left_width, right_width)) => {
                let art = left_width.min(height - 190.0).max(160.0).floor();
                let left = column![
                    self.cover(
                        track.thumbnail_url.as_deref(),
                        art,
                        crate::artwork::LARGE_MAX
                    ),
                    Space::new().height(16),
                    self.track_header(track, art),
                    Space::new().height(14),
                    container(self.seek_row(12.0)).width(art),
                    container(self.transport(22.0)).width(art).center_x(art),
                ]
                .width(left_width)
                .align_x(Alignment::Center);
                row![
                    container(left)
                        .width(left_width)
                        .height(Length::Fill)
                        .center_x(left_width),
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
                    self.seek_row(12.0),
                    container(self.transport(20.0)).center_x(Length::Fill),
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
        let text_width = width - 44.0;
        let mut meta = Vec::new();
        if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
            meta.push(album.to_owned());
        }
        // Only an album or playlist the queue was started from; search and
        // radio queues have no meaningful source to show.
        if let Some(source) = &self.queue_source {
            meta.push(source.title.clone());
            meta.push(self.tr_string(format!("{} songs", source.track_count)));
        }
        row![
            column![
                text(fit(&track.title, text_width, 28.0))
                    .size(28)
                    .font(MONO),
                self.artist_link(&track.artist, text_width, 18.0, INK),
                label(fit(&meta.join("  •  "), text_width, 12.0), 12.0).color(DIM),
            ]
            .spacing(4)
            .width(Length::Fill),
            self.like_button(Some(track.video_id.as_str()), 24.0),
        ]
        .width(width)
        .align_y(Alignment::Start)
        .into()
    }

    fn side_panel(&self, width: f32) -> Element<'_, Message> {
        let tab = |title: &'static str, which: NowPlayingTab| {
            button(
                container(label(self.tr(title).into_owned(), 14.0))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(Message::SetNowPlayingTab(which))
            .width(Length::Fill)
            .height(38)
            .style(theme::tab_style(self.now_playing_tab == which))
        };
        let tabs = container(
            row![
                tab("Up Next", NowPlayingTab::Queue),
                vrule(38.0, INK),
                tab("Lyrics", NowPlayingTab::Lyrics),
            ]
            .align_y(Alignment::Center),
        )
        .padding(theme::hairline())
        .style(theme::boxed);
        let body = match self.now_playing_tab {
            NowPlayingTab::Queue => self.up_next(width - 2.0 * PANEL_PADDING),
            NowPlayingTab::Lyrics => self.lyrics_view(),
        };
        container(
            column![
                container(tabs).padding(PANEL_PADDING),
                hrule(INK),
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

    fn up_next(&self, width: f32) -> Element<'_, Message> {
        if self.queue.is_empty() {
            return container(label(self.tr("The queue is empty.").into_owned(), 13.0).color(DIM))
                .padding([16, 0])
                .into();
        }
        let range = layout::virtual_range(
            self.queue.len(),
            self.queue_scroll.y,
            self.page_height(),
            QUEUE_ROW_PITCH,
            6,
        );
        let mut rows = column![].spacing(0).width(Length::Fill);
        if range.start > 0 {
            rows = rows.push(
                Space::new().height(layout::virtual_spacer_height(range.start, QUEUE_ROW_PITCH)),
            );
        }
        let text_width = width - QUEUE_COVER - 20.0 - 64.0 - 44.0 - 24.0;
        for index in range.clone() {
            let entry = &self.queue[index];
            let current = self.queue_index == Some(index);
            let duration = self
                .track_duration(&entry.video_id)
                .map(format_duration)
                .unwrap_or_default();
            let menu = super::messages::RowMenu {
                list: "queue",
                index,
            };
            let open = self.row_menu.as_ref() == Some(&menu);
            let marker: Element<'_, Message> = if current {
                label("▶", 12.0).into()
            } else {
                Space::new().width(0).into()
            };
            let more = button(super::theme::icon("more", 18.0, INK))
                .on_press(Message::ToggleRowMenu(menu))
                .padding(6)
                .style(if open {
                    theme::icon_button_active
                } else {
                    theme::icon_button
                });
            let main = button(
                row![
                    self.cover(
                        entry.thumbnail_url.as_deref(),
                        QUEUE_COVER,
                        crate::artwork::THUMB_MAX
                    ),
                    column![
                        row![
                            marker,
                            text(fit(&entry.title, text_width, 15.0))
                                .size(15)
                                .font(MONO)
                        ]
                        .spacing(8),
                        label(fit(&entry.artist, text_width, 12.0), 12.0).color(DIM),
                    ]
                    .spacing(6)
                    .width(Length::Fill),
                    label(duration, 13.0),
                    more,
                ]
                .spacing(20)
                .align_y(Alignment::Center),
            )
            .on_press(Message::PlayQueueIndex(index))
            .width(Length::Fill)
            .padding([8, 8])
            .style(if current {
                theme::row_button_current
            } else {
                theme::row_button
            });
            let mut block = column![main];
            if open {
                block = block.push(self.row_actions_with(entry, (!current).then_some(index)));
            }
            rows = rows.push(block.push(hrule(RULE)));
        }
        let remaining = self.queue.len().saturating_sub(range.end);
        if remaining > 0 {
            rows = rows.push(
                Space::new().height(layout::virtual_spacer_height(remaining, QUEUE_ROW_PITCH)),
            );
        }
        if self.radio_loading {
            rows = rows.push(
                container(label(self.tr("[ LOADING RADIO ]").into_owned(), 12.0).color(DIM))
                    .padding([12, 0]),
            );
        }
        if let Some(error) = &self.radio_error {
            rows = rows.push(
                row![
                    label(fit(error, width - 120.0, 12.0), 12.0).color(DIM),
                    button(label(self.tr("Retry").into_owned(), 12.0))
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
        let header = row![
            Space::new().width(Length::Fill),
            button(label(self.tr("Clear queue").into_owned(), 12.0))
                .on_press_maybe((upcoming > 0).then_some(Message::ClearUpcomingQueue))
                .padding([6, 12])
                .style(theme::outline_button),
        ]
        .align_y(Alignment::Center);
        let list = scrollable(rows)
            .id("queue-content")
            .on_scroll(|viewport| Message::QueueScrolled(viewport.absolute_offset()))
            .direction(theme::vbar())
            .style(theme::scroll_style)
            .height(Length::Fill);
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
            return container(label(self.tr("[ LOADING LYRICS ]").into_owned(), 13.0).color(DIM))
                .padding([16, 0])
                .into();
        }
        let Some(lyrics) = &self.lyrics else {
            return column![
                label(
                    self.lyrics_error
                        .clone()
                        .unwrap_or_else(|| self.tr("No lyrics for this song.").into_owned()),
                    13.0
                )
                .color(DIM),
                button(label(self.tr("Retry").into_owned(), 12.0))
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
                                    Color::from_rgb8(120, 120, 120)
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
            let sub_color = if active {
                Color::from_rgb8(70, 70, 70)
            } else {
                DIM
            };
            let mut body = column![content].spacing(2);
            if show_translation && let Some(romanization) = line.romanization.as_deref() {
                body = body.push(
                    text(romanization.to_owned())
                        .size(12)
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
                        .size(12)
                        .font(MONO)
                        .color(sub_color),
                );
            }
            if let Some(background) = line.background.as_deref() {
                body = body.push(
                    text(background.to_owned())
                        .size(12)
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
        // Plain text is shown only when there is no synced text, so the two never mix.
        if let Some(plain) = lyrics.plain.as_ref().filter(|_| lyrics.lines.is_empty()) {
            lines =
                lines.push(container(text(plain.clone()).size(size).font(MONO)).padding([4, 12]));
        }
        let mut content = column![
            container(label(self.lyrics_source_label(lyrics), 11.0).color(FADED)).padding([10, 12]),
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
                    button(label(self.tr("Return to current line").into_owned(), 12.0))
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
