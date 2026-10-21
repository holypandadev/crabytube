//! The Library page, a row for each of the user's lists, and the page of one list: its cover, name
//! and buttons over a strict table of its songs. Rows of both can be dragged to another place.

use super::drag::{drag_area, drag_row};
use super::home::RowContext;
use super::messages::{DragList, ListId, PlaylistEditTarget, RowMenu};
use super::shell::{display_title, fit};
use super::theme::{
    self, DIM, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_DISPLAY, TEXT_TITLE, hrule, icon,
    label,
};
use super::{App, Confirmation, Message, Page, format_duration, layout};
use crate::music::Track;
use crate::preferences::UiDensity;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Size};
use std::sync::Arc;

const INDEX_W: f32 = 48.0;
const DURATION_W: f32 = 96.0;
const HEART_W: f32 = 44.0;
/// Width of the ••• button beside the heart.
const MORE_W: f32 = 30.0;
const CELL_GAP: f32 = 16.0;

/// The cover of a list on the Library page, and the fixed pitch of its row: the cover, 8 px above
/// and below, and a 1 px rule.
pub(super) const LIST_COVER: f32 = 56.0;
pub(super) const LIST_ROW_PITCH: f32 = LIST_COVER + 2.0 * 8.0 + 1.0;
/// The most song covers a playlist offers to choose from as its own.
const COVER_CHOICES: usize = 24;
const CHOICE_COVER: f32 = 56.0;

pub(super) fn song_cover(density: UiDensity) -> f32 {
    density.collection_cover()
}

/// Fixed row pitch of a list's songs: cover, 10 px padding above and below, 1 px rule.
pub(super) fn song_row_pitch(density: UiDensity) -> f32 {
    density.collection_row_pitch()
}

/// The size of a list's cover at the top of its page.
pub(super) fn header_cover(width: f32) -> f32 {
    if width < 700.0 { 112.0 } else { 168.0 }
}

/// The covers a playlist's songs offer as its own: each one once, the first ones first.
pub(super) fn cover_choices(songs: &[Track]) -> Vec<&str> {
    let mut choices: Vec<&str> = Vec::new();
    for url in songs
        .iter()
        .filter_map(|track| track.thumbnail_url.as_deref())
    {
        if !choices.contains(&url) {
            choices.push(url);
            if choices.len() == COVER_CHOICES {
                break;
            }
        }
    }
    choices
}

impl App {
    /// The Library page: its title and New playlist over a row for Favorite songs and one for each
    /// playlist, in the user's order.
    pub(super) fn library_page(&self, area: Size) -> Element<'_, Message> {
        let width = area.width.max(240.0);
        let narrow = width < 700.0;
        let naming = self
            .playlist_editor
            .as_ref()
            .is_some_and(|editor| editor.target == PlaylistEditTarget::New(None));
        let new: Element<'_, Message> = if naming {
            self.playlist_name_field(if narrow { width } else { 620.0 }, 360.0, "Create")
        } else {
            button(
                row![
                    icon("plus", 14.0, INK),
                    label(self.tr("New playlist").into_owned(), TEXT_BODY)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .on_press(Message::NewPlaylist(None))
            .padding([12, 24])
            .style(theme::outline_button)
            .into()
        };
        let title = text(self.tr("Library"))
            .size(if narrow { TEXT_TITLE } else { TEXT_DISPLAY })
            .font(MONO);
        let mut header = if narrow {
            column![title, new].spacing(16)
        } else {
            column![row![container(title).width(Length::Fill), new].align_y(Alignment::Center)]
        };
        if self.save_dialog.is_none()
            && let Some(notice) = &self.playlist_notice
        {
            header = header.push(label(self.playlist_notice_text(notice), TEXT_CAPTION).color(DIM));
        }

        let lists = DragList::Lists;
        let dragged = self.dragged_row(lists);
        let mut rows = column![].width(Length::Fill);
        for shown in 0..=self.playlists.len() {
            // Favorite songs is the first row and stays there; a dragged playlist lands after it.
            let (list, name, count) = match shown.checked_sub(1) {
                None => (
                    ListId::Favorites,
                    self.tr("Favorite songs").into_owned(),
                    self.favorites.len(),
                ),
                Some(place) => {
                    let playlist = &self.playlists[self.dragged_item(lists, place)];
                    (
                        ListId::Playlist(playlist.id),
                        playlist.name.clone(),
                        playlist.tracks.len(),
                    )
                }
            };
            let menu = RowMenu {
                list: "lists",
                index: shown,
            };
            let open = self.row_menu.as_ref() == Some(&menu);
            let lifted = shown > 0 && dragged == Some(shown - 1);
            let text_width = width - LIST_COVER - 16.0 - MORE_W - 40.0;
            let mut cells = row![
                self.list_cover(list, LIST_COVER),
                column![
                    label(fit(&name, text_width, TEXT_BODY), TEXT_BODY),
                    label(self.song_count(count, false), TEXT_CAPTION).color(DIM),
                ]
                .spacing(6)
                .width(Length::Fill),
            ]
            .spacing(16)
            .align_y(Alignment::Center);
            if shown > 0 {
                cells = cells.push(
                    button(icon("more", 18.0, INK))
                        .on_press(Message::ToggleRowMenu(menu.clone()))
                        .padding(6)
                        .style(theme::row_icon_button(false, open)),
                );
            }
            let line = button(cells)
                .on_press(self.row_click(lists, Message::Navigate(Page::List(list))))
                .width(Length::Fill)
                .padding([8, 4])
                .style(if lifted || self.row_is_cursor("lists", shown) {
                    theme::row_button_cursor
                } else {
                    theme::row_button
                });
            let block = if open {
                column![line, self.list_actions(shown - 1), hrule(RULE)]
            } else {
                column![line, hrule(RULE)].height(LIST_ROW_PITCH)
            };
            let place = shown.saturating_sub(1);
            rows = rows.push(super::keyboard::mark_row(
                "lists",
                shown,
                drag_row(
                    block,
                    (shown > 0).then_some(Message::DragPressed {
                        list: lists,
                        index: place,
                    }),
                    self.drag_enter(lists, place),
                ),
            ));
        }
        column![
            Space::new().height(8),
            header,
            Space::new().height(28),
            hrule(INK),
            drag_area(
                scrollable(rows)
                    .id("library-lists")
                    .on_scroll(|viewport| Message::LibraryListsScrolled(viewport.absolute_offset()))
                    .direction(theme::vbar())
                    .style(theme::scroll_style)
                    .height(Length::Fill),
                self.drag_edges(lists),
            ),
        ]
        .into()
    }

    /// The ••• actions of a playlist's row on the Library page: moving it, for the keyboard.
    fn list_actions(&self, place: usize) -> Element<'_, Message> {
        let action = |title: &'static str, message: Option<Message>| {
            button(label(self.tr(title).into_owned(), TEXT_CAPTION))
                .on_press_maybe(message)
                .padding([6, 12])
                .style(theme::outline_button)
        };
        container(
            row![
                action(
                    "Move up",
                    (place > 0).then_some(Message::MovePlaylist {
                        index: place,
                        up: true
                    })
                ),
                action(
                    "Move down",
                    (place + 1 < self.playlists.len()).then_some(Message::MovePlaylist {
                        index: place,
                        up: false
                    })
                ),
            ]
            .spacing(10),
        )
        .padding([4, 76])
        .into()
    }

    /// The page of one list. A list that is gone shows the Library.
    pub(super) fn list_page(&self, list: ListId, area: Size) -> Element<'_, Message> {
        let Some(songs) = self.list_songs(list) else {
            return self.library_page(area);
        };
        let width = area.width.max(240.0);
        let header = self.list_header(list, songs, width);
        self.song_table(area, header, songs, list)
    }

    /// A list's cover, name and count, its Play all and Shuffle, and for a playlist Edit, which
    /// opens its name and cover, and Delete, which asks first.
    fn list_header<'a>(
        &'a self,
        list: ListId,
        songs: &'a [Track],
        width: f32,
    ) -> Element<'a, Message> {
        let count = songs.len();
        let narrow = width < 700.0;
        let cover_size = header_cover(width);
        let (name, play, shuffle) = match list {
            ListId::Favorites => {
                let shared = Arc::clone(&self.favorites_context);
                (
                    self.tr("Favorite songs").into_owned(),
                    (count > 0).then(|| Message::PlayContext(Arc::clone(&shared), 0)),
                    (count > 0).then(|| Message::ShuffleContext(shared)),
                )
            }
            ListId::Playlist(id) => (
                self.playlist(id)
                    .map(|playlist| playlist.name.clone())
                    .unwrap_or_default(),
                (count > 0).then_some(Message::PlayPlaylist { id, index: 0 }),
                (count > 0).then_some(Message::ShufflePlaylist(id)),
            ),
        };
        let editing = match (list, self.playlist_editor.as_ref()) {
            (ListId::Playlist(id), Some(editor)) => editor.target == PlaylistEditTarget::Edit(id),
            _ => false,
        };
        let mut actions = row![
            self.list_button("play", "Play all", play),
            self.list_button("shuffle", "Shuffle", shuffle),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        if let ListId::Playlist(id) = list {
            actions = actions
                .push(
                    self.text_list_button("Edit", (!editing).then_some(Message::EditPlaylist(id))),
                )
                .push(self.text_list_button("Delete", Some(Message::AskDeletePlaylist(id))));
        }
        let title_size = if narrow { TEXT_TITLE } else { TEXT_DISPLAY };
        let text_width = if narrow {
            width - 32.0
        } else {
            width - cover_size - 32.0
        };
        let titles = column![
            text(fit(&name, text_width, title_size))
                .size(title_size)
                .font(MONO),
            label(
                if list == ListId::Favorites {
                    self.tr_string(format!("{count} saved tracks"))
                } else {
                    self.song_count(count, false)
                },
                TEXT_BODY
            ),
            Space::new().height(8),
            actions.wrap().vertical_spacing(12),
        ]
        .spacing(6)
        .width(Length::Fill);
        let mut header = if narrow {
            column![self.list_cover(list, cover_size), titles].spacing(16)
        } else {
            column![
                row![self.list_cover(list, cover_size), titles]
                    .spacing(28)
                    .align_y(Alignment::End)
            ]
        };
        if editing {
            header = header.push(self.edit_panel(songs, width));
        }
        if let ListId::Playlist(id) = list {
            let asking = matches!(
                self.confirmation.as_ref().map(|pending| &pending.kind),
                Some(Confirmation::DeletePlaylist(pending)) if *pending == id
            );
            if asking {
                header = header.push(
                    row![
                        label(self.tr("Delete this playlist?").into_owned(), TEXT_BODY),
                        Space::new().width(Length::Fill),
                        self.confirm_buttons("Delete", Message::DeletePlaylist(id)),
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                );
            }
        }
        if self.save_dialog.is_none()
            && let Some(notice) = &self.playlist_notice
        {
            header = header.push(label(self.playlist_notice_text(notice), TEXT_CAPTION).color(DIM));
        }
        header.spacing(20).into()
    }

    /// A playlist's name field and the covers it can take: its first songs' together, or one of
    /// its songs' covers. Save keeps both.
    fn edit_panel<'a>(&'a self, songs: &'a [Track], width: f32) -> Element<'a, Message> {
        let chosen = self
            .playlist_editor
            .as_ref()
            .and_then(|editor| editor.cover.as_deref());
        let choice = |content: Element<'a, Message>, cover: Option<&str>| {
            button(content)
                .on_press(Message::PlaylistCoverChosen(cover.map(str::to_owned)))
                .padding(4)
                .style(if chosen == cover {
                    theme::selected_outline_button
                } else {
                    theme::outline_button
                })
        };
        // The automatic cover, as it is now, says what it is under it.
        let automatic = column![
            choice(
                self.cover_of(super::playlists::automatic_cover(songs), CHOICE_COVER),
                None
            ),
            label(self.tr("Automatic").into_owned(), TEXT_CAPTION).color(DIM),
        ]
        .spacing(6)
        .align_x(Alignment::Center);
        let mut covers = row![automatic].spacing(10).align_y(Alignment::Start);
        for url in cover_choices(songs) {
            covers = covers.push(choice(
                self.cover(Some(url), CHOICE_COVER, crate::artwork::THUMB_MAX),
                Some(url),
            ));
        }
        column![
            self.playlist_name_field(width, 480.0, "Save"),
            label(self.tr("Cover").into_owned(), TEXT_CAPTION).color(DIM),
            covers.wrap().vertical_spacing(10),
        ]
        .spacing(12)
        .into()
    }

    /// An outlined button with an icon, as Play all and Shuffle. Without a message it is off.
    fn list_button(
        &self,
        glyph: &'static str,
        title: &'static str,
        message: Option<Message>,
    ) -> Element<'_, Message> {
        button(
            row![
                icon(glyph, 16.0, INK),
                label(self.tr(title).into_owned(), TEXT_BODY)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press_maybe(message)
        .padding([12, 24])
        .style(theme::outline_button)
        .into()
    }

    fn text_list_button(
        &self,
        title: &'static str,
        message: Option<Message>,
    ) -> Element<'_, Message> {
        button(label(self.tr(title).into_owned(), TEXT_BODY))
            .on_press_maybe(message)
            .padding([12, 24])
            .style(theme::outline_button)
            .into()
    }

    /// The table of a list: the header, its head, then its songs as a virtual list whose rows can
    /// be dragged.
    fn song_table<'a>(
        &'a self,
        area: Size,
        header: Element<'a, Message>,
        songs: &'a [Track],
        list: ListId,
    ) -> Element<'a, Message> {
        let width = area.width.max(240.0);
        let count = songs.len();
        let rows_name = App::song_rows(list);
        let dragging = DragList::Songs(list);
        // Narrow windows drop the album and then the duration column.
        let show_album = width >= 900.0;
        let show_duration = width >= 560.0;
        let fixed = INDEX_W
            + HEART_W
            + MORE_W
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
            container(label(self.tr(title).into_owned(), TEXT_CAPTION).color(DIM)).width(w)
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
        let table_head = table_head.push(Space::new().width(MORE_W + HEART_W));

        let mut page = column![
            Space::new().height(8),
            header,
            Space::new().height(28),
            table_head,
            Space::new().height(10),
            hrule(INK),
        ];
        if count == 0 {
            let hint = match list {
                ListId::Favorites => "Tap the heart on any song to save it here.",
                ListId::Playlist(_) => "Tap the heart on any song and choose this playlist.",
            };
            page = page.push(
                container(label(self.tr(hint).into_owned(), TEXT_BODY).color(DIM)).padding([20, 4]),
            );
            return page.into();
        }
        let density = self.preferences.ui_density;
        let pitch = song_row_pitch(density);
        let cover = song_cover(density);
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
        let lifted = self.dragged_row(dragging);
        for shown in range.clone() {
            // While a row is dragged, the songs show in the order they would land in.
            let index = self.dragged_item(dragging, shown);
            let track = &songs[index];
            let current = playing == Some(track.video_id.as_str());
            // The row under the keyboard cursor is outlined, as the dragged one; the playing row
            // stays inverted.
            let cursor = self.row_is_cursor(rows_name, index) || lifted == Some(shown);
            let ink = theme::row_ink(current);
            let menu = RowMenu {
                list: rows_name,
                index,
            };
            let open = self.row_menu.as_ref() == Some(&menu);
            let duration = self
                .duration_of(track)
                .map(format_duration)
                .unwrap_or_else(|| "—".to_owned());
            let album = track.album.as_deref().unwrap_or("—");
            let cells = row![
                container(
                    label(
                        if current {
                            "▶".to_owned()
                        } else {
                            (shown + 1).to_string()
                        },
                        TEXT_BODY
                    )
                    .color(ink.title)
                )
                .width(INDEX_W),
                row![
                    self.cover(
                        track.thumbnail_url.as_deref(),
                        cover,
                        crate::artwork::THUMB_MAX
                    ),
                    if show_duration {
                        Element::from(
                            label(
                                fit(
                                    display_title(&track.title),
                                    title_w - cover - 16.0,
                                    TEXT_BODY,
                                ),
                                TEXT_BODY,
                            )
                            .color(ink.title),
                        )
                    } else {
                        column![
                            label(
                                fit(
                                    display_title(&track.title),
                                    title_w - cover - 16.0,
                                    TEXT_BODY
                                ),
                                TEXT_BODY
                            )
                            .color(ink.title),
                            label(
                                fit(&track.artist, title_w - cover - 16.0, TEXT_CAPTION),
                                TEXT_CAPTION
                            )
                            .color(ink.secondary),
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
                    container(
                        label(fit(&track.artist, artist_w, TEXT_BODY), TEXT_BODY).color(ink.title),
                    )
                    .width(artist_w),
                )
            } else {
                cells
            };
            let cells = if show_album {
                cells.push(
                    container(
                        label(fit(album, album_w, TEXT_BODY), TEXT_BODY).color(ink.secondary),
                    )
                    .width(album_w),
                )
            } else {
                cells
            };
            let cells = if show_duration {
                cells.push(
                    container(label(duration, TEXT_BODY).color(ink.secondary)).width(DURATION_W),
                )
            } else {
                cells
            };
            // The heart sits inside the row, so the hover fill runs under it
            // instead of stopping at a black box. It opens the Save panel.
            let saved = self.is_saved(&track.video_id);
            let heart = container(
                button(icon(
                    if saved { "like-filled" } else { "like" },
                    18.0,
                    ink.icon,
                ))
                .on_press(self.row_click(dragging, Message::OpenSaveDialog(track.clone())))
                .padding(6)
                .style(theme::row_icon_button(current, false)),
            )
            .width(HEART_W)
            .center_x(HEART_W);
            let more = button(icon("more", 18.0, ink.icon))
                .on_press(self.row_click(dragging, Message::ToggleRowMenu(menu)))
                .padding(6)
                .style(theme::row_icon_button(current, open));
            let play = match list {
                ListId::Favorites => {
                    Message::PlayContext(Arc::clone(&self.favorites_context), index)
                }
                ListId::Playlist(id) => Message::PlayPlaylist { id, index },
            };
            let line = button(
                row![cells, Space::new().width(Length::Fill), more, heart]
                    .align_y(Alignment::Center),
            )
            .on_press(self.row_click(dragging, play))
            .width(Length::Fill)
            .padding([10, 4])
            .style(if current {
                theme::row_button_current
            } else if cursor {
                theme::row_button_cursor
            } else {
                theme::row_button
            });
            // An open menu adds its actions under the row; closed rows keep the fixed pitch the
            // virtual list counts with.
            let block = if open {
                column![
                    line,
                    self.row_actions_in(track, RowContext::List { list, index, count }),
                    hrule(RULE)
                ]
            } else {
                column![line, hrule(RULE)].height(pitch)
            };
            rows = rows.push(super::keyboard::mark_row(
                rows_name,
                index,
                drag_row(
                    block,
                    Some(Message::DragPressed {
                        list: dragging,
                        index,
                    }),
                    self.drag_enter(dragging, shown),
                ),
            ));
        }
        let remaining = count.saturating_sub(range.end);
        if remaining > 0 {
            rows = rows.push(Space::new().height(layout::virtual_spacer_height(remaining, pitch)));
        }
        page.push(drag_area(
            scrollable(rows)
                .id("library-content")
                .on_scroll(|viewport| Message::LibraryScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill),
            self.drag_edges(dragging),
        ))
        .into()
    }
}
