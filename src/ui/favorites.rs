//! Favorite Songs: a strict table of the locally saved tracks.

use super::shell::fit;
use super::theme::{self, DIM, INK, MONO, RULE, hrule, icon, label};
use super::{App, Message, format_duration, layout};
use crate::preferences::UiDensity;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Size};
use std::sync::Arc;

const INDEX_W: f32 = 48.0;
const DURATION_W: f32 = 96.0;
const HEART_W: f32 = 44.0;
const CELL_GAP: f32 = 16.0;

pub(super) fn favorites_cover(density: UiDensity) -> f32 {
    density.collection_cover()
}

/// Fixed row pitch: cover, 10 px padding above and below, 1 px rule.
pub(super) fn favorites_row_pitch(density: UiDensity) -> f32 {
    density.collection_row_pitch()
}

impl App {
    pub(super) fn favorites_page(&self, area: Size) -> Element<'_, Message> {
        let width = area.width.max(240.0);
        let count = self.favorites.len();
        let shared = Arc::clone(&self.favorites_context);
        let actions = row![
            button(
                row![
                    icon("play", 16.0, INK),
                    label(self.tr("Play All").into_owned(), 14.0)
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            )
            .on_press_maybe((count > 0).then(|| Message::PlayContext(Arc::clone(&shared), 0)))
            .padding([12, 24])
            .style(theme::outline_button),
            button(
                row![
                    icon("shuffle", 16.0, INK),
                    label(self.tr("Shuffle").into_owned(), 14.0)
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            )
            .on_press_maybe((count > 0).then(|| Message::ShuffleContext(Arc::clone(&shared))))
            .padding([12, 24])
            .style(theme::outline_button),
        ]
        .spacing(16);
        let narrow = width < 700.0;
        let titles = column![
            text(self.tr("Favorite Songs"))
                .size(if narrow { 28 } else { 40 })
                .font(MONO),
            label(self.tr_string(format!("{count} saved tracks")), 16.0),
        ]
        .spacing(6)
        .width(Length::Fill);
        let header: Element<'_, Message> = if narrow {
            column![titles, actions].spacing(16).into()
        } else {
            row![titles, actions].align_y(Alignment::Center).into()
        };

        // Narrow windows drop the album and then the duration column.
        let show_album = width >= 900.0;
        let show_duration = width >= 560.0;
        let fixed = INDEX_W
            + HEART_W
            + if show_duration {
                DURATION_W + CELL_GAP
            } else {
                0.0
            }
            + if show_album { CELL_GAP } else { 0.0 }
            + 2.0 * CELL_GAP;
        let flexible = (width - fixed).max(160.0);
        let (title_w, artist_w, album_w) = if show_album {
            let title = (flexible * 0.4).floor();
            let artist = (flexible * 0.3).floor();
            (title, artist, flexible - title - artist)
        } else if show_duration {
            let title = (flexible * 0.6).floor();
            (title, flexible - title, 0.0)
        } else {
            // Phone width: the artist goes under the title instead of in a column.
            (flexible - CELL_GAP, 0.0, 0.0)
        };
        let column_head = |title: &'static str, w: f32| {
            container(label(self.tr(title).into_owned(), 11.0).color(DIM)).width(w)
        };
        let mut table_head = row![column_head("#", INDEX_W), column_head("TITLE", title_w)]
            .spacing(CELL_GAP)
            .padding([0, 4]);
        if show_duration {
            table_head = table_head.push(column_head("ARTIST", artist_w));
        }
        if show_album {
            table_head = table_head.push(column_head("ALBUM", album_w));
        }
        if show_duration {
            table_head = table_head.push(column_head("DURATION", DURATION_W));
        }
        let table_head = table_head.push(Space::new().width(HEART_W));

        let mut page = column![
            Space::new().height(8),
            header,
            Space::new().height(28),
            table_head,
            Space::new().height(10),
            hrule(INK),
        ];
        if count == 0 {
            page = page.push(
                container(
                    label(
                        self.tr("Tap the heart on any song to save it here.")
                            .into_owned(),
                        13.0,
                    )
                    .color(DIM),
                )
                .padding([20, 4]),
            );
            return page.into();
        }
        if let Some(notice) = &self.library_notice {
            page = page.push(label(self.tr_string(notice.clone()), 12.0).color(DIM));
        }

        let density = self.preferences.ui_density;
        let pitch = favorites_row_pitch(density);
        let cover = favorites_cover(density);
        let range = layout::virtual_range(count, self.library_scroll.y, area.height, pitch, 6);
        let mut rows = column![].spacing(0).width(Length::Fill);
        if range.start > 0 {
            rows =
                rows.push(Space::new().height(layout::virtual_spacer_height(range.start, pitch)));
        }
        let playing = self
            .playback_status
            .track
            .as_ref()
            .map(|track| track.video_id.as_str());
        for index in range.clone() {
            let track = &self.favorites[index];
            let current = playing == Some(track.video_id.as_str());
            let duration = self
                .track_duration(&track.video_id)
                .or_else(|| {
                    track
                        .duration_secs
                        .map(|secs| std::time::Duration::from_secs(u64::from(secs)))
                })
                .map(format_duration)
                .unwrap_or_else(|| "—".to_owned());
            let album = track.album.as_deref().unwrap_or("—");
            let cells = row![
                container(label(
                    if current {
                        "▶".to_owned()
                    } else {
                        (index + 1).to_string()
                    },
                    14.0
                ))
                .width(INDEX_W),
                row![
                    self.cover(
                        track.thumbnail_url.as_deref(),
                        cover,
                        crate::artwork::THUMB_MAX
                    ),
                    if show_duration {
                        Element::from(label(fit(&track.title, title_w - cover - 16.0, 14.0), 14.0))
                    } else {
                        column![
                            label(fit(&track.title, title_w - cover - 16.0, 14.0), 14.0),
                            label(fit(&track.artist, title_w - cover - 16.0, 12.0), 12.0)
                                .color(DIM),
                        ]
                        .spacing(4)
                        .into()
                    },
                ]
                .spacing(16)
                .align_y(Alignment::Center)
                .width(title_w),
            ]
            .spacing(CELL_GAP)
            .align_y(Alignment::Center);
            let cells = if show_duration {
                cells.push(
                    container(label(fit(&track.artist, artist_w, 14.0), 14.0)).width(artist_w),
                )
            } else {
                cells
            };
            let cells = if show_album {
                cells.push(container(label(fit(album, album_w, 14.0), 14.0)).width(album_w))
            } else {
                cells
            };
            let cells = if show_duration {
                cells.push(container(label(duration, 14.0)).width(DURATION_W))
            } else {
                cells
            };
            // The heart sits inside the row, so the hover fill runs under it
            // instead of stopping at a black box.
            let heart = container(
                button(icon("like-filled", 18.0, INK))
                    .on_press(Message::ToggleLike(track.video_id.clone()))
                    .padding(6)
                    .style(theme::icon_button),
            )
            .width(HEART_W)
            .center_x(HEART_W);
            let line = button(
                row![cells, Space::new().width(Length::Fill), heart].align_y(Alignment::Center),
            )
            .on_press(Message::PlayContext(Arc::clone(&shared), index))
            .width(Length::Fill)
            .padding([10, 4])
            .style(if current {
                theme::row_button_current
            } else {
                theme::row_button
            });
            rows = rows.push(column![line, hrule(RULE)].height(pitch));
        }
        let remaining = count.saturating_sub(range.end);
        if remaining > 0 {
            rows = rows.push(Space::new().height(layout::virtual_spacer_height(remaining, pitch)));
        }
        page.push(
            scrollable(rows)
                .id("library-content")
                .on_scroll(|viewport| Message::LibraryScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill),
        )
        .into()
    }
}
