//! The Library page, a row for each of the user's lists, and the page of one list: its cover, name
//! and buttons over a strict table of its songs. Rows of both can be dragged to another place.

use super::drag::{drag_area, drag_row};
use super::messages::{CoverChoice, DragList, ListId, PlaylistEditTarget, RowMenu};
use super::playlists::HEART_W;
use super::shell::{display_title, fit};
use super::theme::{
    self, DIM, FADED, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_DISPLAY, TEXT_TITLE, hrule,
    icon, label,
};
use super::{App, Message, Page, Preference, format_duration, layout};
use crate::music::Track;
use crate::preferences::{LibrarySort, UiDensity};
use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, stack, text, text_input,
};
use iced::{Alignment, Element, Length, Size};
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

const INDEX_W: f32 = 48.0;
const DURATION_W: f32 = 96.0;
const CELL_GAP: f32 = 16.0;

/// The cover of a list on the Library page, and the fixed pitch of its row: the cover, 8 px above
/// and below, and a 1 px rule.
pub(super) const LIST_COVER: f32 = 56.0;
pub(super) const LIST_ROW_PITCH: f32 = LIST_COVER + 2.0 * 8.0 + 1.0;
/// The id of the Library's filter field, and the widths of the field and of the order's list.
pub(super) const LIBRARY_FILTER: &str = "library-filter";
const FILTER_W: f32 = 360.0;
const SORT_W: f32 = 220.0;
/// The gap between the buttons under a list's name.
const BUTTON_GAP: f32 = 12.0;
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
                .spacing(20)
        };
        if self.save_dialog.is_none()
            && let Some(notice) = &self.playlist_notice
        {
            header = header.push(label(self.playlist_notice_text(notice), TEXT_CAPTION).color(DIM));
        }
        // With playlists to find, a filter and the order to show them in.
        if !self.playlists.is_empty() {
            header = header.push(self.library_controls(width));
        }

        let lists = DragList::Lists;
        let dragged = self.dragged_row(lists);
        // The user's order can be changed by dragging while the page shows it whole.
        let reorderable = self.library_reorderable();
        let mut rows = column![].width(Length::Fill);
        let shown_rows = self.library_rows();
        if shown_rows.is_empty() {
            rows = rows.push(
                container(label(self.tr("No playlists match.").into_owned(), TEXT_BODY).color(DIM))
                    .padding([20, 4]),
            );
        }
        for shown in shown_rows {
            // Favorite songs is the first row and stays there.
            let place = shown.checked_sub(1);
            let (list, name, count) = match place {
                None => (
                    ListId::Favorites,
                    self.tr("Favorite songs").into_owned(),
                    self.favorites.len(),
                ),
                Some(place) => {
                    let playlist = &self.playlists[place];
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
            // The row being dragged stays in its place, faded, while a label follows the pointer.
            let lifted = place.is_some() && place == dragged;
            let ink = if lifted { FADED } else { INK };
            let text_width = width - LIST_COVER - 16.0 - self.row_more_width() - 40.0;
            let mut cells = row![
                self.list_cover(list, LIST_COVER),
                column![
                    label(fit(&name, text_width, TEXT_BODY), TEXT_BODY).color(ink),
                    label(self.song_count(count, false), TEXT_CAPTION).color(DIM),
                ]
                .spacing(6)
                .width(Length::Fill),
            ]
            .spacing(16)
            .align_y(Alignment::Center);
            if place.is_some() {
                cells = cells.push(
                    button(icon("more", 18.0, ink))
                        .on_press(self.row_click(lists, Message::ToggleRowMenu(menu)))
                        .padding(self.row_icon_padding(LIST_COVER))
                        .style(theme::row_icon_button(false, open)),
                );
            }
            let line = button(cells)
                .on_press(self.row_click(lists, Message::Navigate(Page::List(list))))
                .width(Length::Fill)
                .padding([8, 4])
                .style(if self.row_is_cursor("lists", shown) {
                    theme::row_button_cursor
                } else {
                    theme::row_button
                });
            rows = rows.push(super::keyboard::mark_row(
                "lists",
                shown,
                drag_row(
                    column![line, hrule(RULE)].height(LIST_ROW_PITCH),
                    place
                        .filter(|_| reorderable)
                        .map(|index| Message::DragPressed { list: lists, index }),
                    place
                        .filter(|_| reorderable)
                        .map(|index| Message::DragHeld { list: lists, index }),
                ),
            ));
        }
        let mut area = stack![
            scrollable(rows)
                .id("library-lists")
                .on_scroll(|viewport| Message::LibraryListsScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill)
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        if let Some(overlay) = self.drag_overlay(lists, width) {
            area = area.push(overlay);
        }
        column![
            Space::new().height(8),
            header,
            Space::new().height(28),
            hrule(INK),
            drag_area(area, self.drag_moves(lists)),
        ]
        .into()
    }

    /// The rows of the Library page in the order shown, by their place: 0 is Favorite songs and
    /// each playlist follows at its place in the user's order plus one. The filter keeps the
    /// lists whose name holds it, and Favorite songs stays first in every order.
    pub(super) fn library_rows(&self) -> Vec<usize> {
        let filter = self.library_filter.trim().to_lowercase();
        let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
        let mut places: Vec<usize> = (0..self.playlists.len())
            .filter(|&place| matches(&self.playlists[place].name))
            .collect();
        match self.preferences.library_sort {
            LibrarySort::Custom => {}
            // Ids only grow, so the newest playlist has the largest.
            LibrarySort::Recent => {
                places.sort_by_key(|&place| std::cmp::Reverse(self.playlists[place].id));
            }
            LibrarySort::Name => {
                places.sort_by_cached_key(|&place| self.playlists[place].name.to_lowercase());
            }
        }
        let favorites = matches(&self.tr("Favorite songs")).then_some(0);
        favorites
            .into_iter()
            .chain(places.into_iter().map(|place| place + 1))
            .collect()
    }

    /// Whether the Library shows the user's order whole, which dragging and Move up and down
    /// change.
    pub(super) fn library_reorderable(&self) -> bool {
        self.preferences.library_sort == LibrarySort::Custom
            && self.library_filter.trim().is_empty()
    }

    /// The filter field and the order of the Library page, side by side, or one under the other on
    /// a phone.
    fn library_controls(&self, width: f32) -> Element<'_, Message> {
        let narrow = width < 560.0;
        let placeholder = self.tr("Filter playlists").into_owned();
        let mut field = row![
            icon("search", 16.0, INK),
            text_input(&placeholder, &self.library_filter)
                .id(LIBRARY_FILTER)
                .on_input(Message::LibraryFilterChanged)
                .font(MONO)
                .size(TEXT_BODY)
                .padding([8, 4])
                .style(theme::input_style),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        if !self.library_filter.is_empty() {
            field = field.push(
                button(icon("close", 14.0, INK))
                    .on_press(Message::LibraryFilterChanged(String::new()))
                    .padding(4)
                    .style(theme::icon_button),
            );
        }
        let field = container(field)
            .width(if narrow {
                Length::Fill
            } else {
                Length::Fixed(FILTER_W.min(width * 0.5))
            })
            .padding([0, 14])
            .style(theme::boxed);
        let sort = pick_list(
            LibrarySort::ALL,
            Some(self.preferences.library_sort),
            |sort| Message::Preference(Preference::LibrarySort(sort)),
        )
        .font(MONO)
        .text_size(TEXT_BODY)
        .padding([9, 12])
        .width(if narrow {
            Length::Fill
        } else {
            Length::Fixed(SORT_W)
        })
        .style(theme::pick_style)
        .menu_style(theme::menu_style);
        if narrow {
            column![field, sort].spacing(10).into()
        } else {
            row![field, Space::new().width(Length::Fill), sort]
                .align_y(Alignment::Center)
                .into()
        }
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
    /// opens its name and cover, and Delete, which asks in a panel first.
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
        let mut buttons: Vec<(Option<&'static str>, &'static str, Option<Message>)> = vec![
            (Some("play"), "Play all", play),
            (Some("shuffle"), "Shuffle", shuffle),
        ];
        if let ListId::Playlist(id) = list {
            buttons.push((
                None,
                "Edit",
                (!editing).then_some(Message::EditPlaylist(id)),
            ));
            buttons.push((None, "Delete", Some(Message::AskDeletePlaylist(id))));
        }
        let room = if narrow {
            width
        } else {
            width - cover_size - 28.0
        };
        let title_size = if narrow { TEXT_TITLE } else { TEXT_DISPLAY };
        let titles = column![
            text(fit(&name, room, title_size))
                .size(title_size)
                .font(MONO),
            label(
                fit(&self.list_summary(list, songs), room, TEXT_BODY),
                TEXT_BODY
            ),
            Space::new().height(8),
            self.even_buttons(buttons, room, narrow),
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
        if self.save_dialog.is_none()
            && let Some(notice) = &self.playlist_notice
        {
            header = header.push(label(self.playlist_notice_text(notice), TEXT_CAPTION).color(DIM));
        }
        header.spacing(20).into()
    }

    /// How many songs a list has and how long they play together, as far as their lengths are
    /// known.
    fn list_summary(&self, list: ListId, songs: &[Track]) -> String {
        let count = songs.len();
        let songs_line = if list == ListId::Favorites {
            self.tr_string(format!("{count} saved tracks"))
        } else {
            self.song_count(count, false)
        };
        let total: u64 = songs
            .iter()
            .filter_map(|track| self.duration_of(track))
            .map(|duration| duration.as_secs())
            .sum();
        if total == 0 {
            return songs_line;
        }
        format!("{songs_line} · {}", self.total_length(total))
    }

    /// A length of `seconds` in minutes, or in hours and minutes from an hour up.
    pub(super) fn total_length(&self, seconds: u64) -> String {
        let minutes = seconds.div_ceil(60);
        if minutes < 60 {
            self.tr_static("{} min")
                .replacen("{}", &minutes.to_string(), 1)
        } else {
            self.tr_static("{} h {} min")
                .replacen("{}", &(minutes / 60).to_string(), 1)
                .replacen("{}", &(minutes % 60).to_string(), 1)
        }
    }

    /// Buttons of one size, the size of the widest label among them, in as many rows as `room`
    /// needs. With `fill`, as on a phone, or where two do not fit side by side, they share the room
    /// in two even columns, edge to edge, and a label too long is cut.
    fn even_buttons<'a>(
        &self,
        buttons: Vec<(Option<&'static str>, &'static str, Option<Message>)>,
        room: f32,
        fill: bool,
    ) -> Element<'a, Message> {
        const PADDING: f32 = 20.0;
        const ICON: f32 = 16.0;
        const ICON_GAP: f32 = 10.0;
        let titles: Vec<String> = buttons
            .iter()
            .map(|(_, title, _)| self.tr(title).into_owned())
            .collect();
        let widest = buttons
            .iter()
            .zip(&titles)
            .map(|((glyph, _, _), title)| {
                title.width() as f32 * TEXT_BODY * 0.6
                    + if glyph.is_some() {
                        ICON + ICON_GAP
                    } else {
                        0.0
                    }
            })
            .fold(0.0_f32, f32::max);
        // A few pixels to spare, so rounding never cuts the widest label.
        let mut width = (widest + 2.0 * PADDING + 6.0).ceil();
        if fill || 2.0 * width + BUTTON_GAP > room {
            width = ((room - BUTTON_GAP) / 2.0).floor().max(80.0);
        }
        let mut line = row![].spacing(BUTTON_GAP);
        for ((glyph, _, message), title) in buttons.into_iter().zip(titles) {
            let text_room = width
                - 2.0 * PADDING
                - if glyph.is_some() {
                    ICON + ICON_GAP
                } else {
                    0.0
                };
            let mut content = row![].spacing(ICON_GAP).align_y(Alignment::Center);
            if let Some(glyph) = glyph {
                content = content.push(icon(glyph, ICON, INK));
            }
            content = content.push(label(
                fit(&title, text_room.max(16.0), TEXT_BODY),
                TEXT_BODY,
            ));
            line = line.push(
                button(
                    container(content)
                        .width(Length::Fill)
                        .center_x(Length::Fill),
                )
                .on_press_maybe(message)
                .width(width)
                .padding([12, 0])
                .style(theme::outline_button),
            );
        }
        line.wrap().vertical_spacing(BUTTON_GAP).into()
    }

    /// A playlist's name field and the covers it can take: its first songs' together, one of its
    /// songs' covers, or a picture from the computer. Save keeps both.
    fn edit_panel<'a>(&'a self, songs: &'a [Track], width: f32) -> Element<'a, Message> {
        let Some(editor) = self.playlist_editor.as_ref() else {
            return Space::new().into();
        };
        let chosen = &editor.cover;
        let choice = |content: Element<'a, Message>, cover: CoverChoice| {
            let selected = *chosen == cover;
            button(content)
                .on_press(Message::PlaylistCoverChosen(cover))
                .padding(4)
                .style(if selected {
                    theme::selected_outline_button
                } else {
                    theme::outline_button
                })
        };
        // A choice with a word under it: the automatic cover, and the computer's.
        let captioned = |tile: Element<'a, Message>, caption: &'static str| {
            column![
                tile,
                // A caption too long for one line goes on to a second.
                text(self.tr(caption).into_owned())
                    .size(TEXT_CAPTION)
                    .font(MONO)
                    .color(DIM)
                    .width(CHOICE_COVER + 24.0)
                    .align_x(iced::alignment::Horizontal::Center),
            ]
            .spacing(6)
            .width(CHOICE_COVER + 24.0)
            .align_x(Alignment::Center)
        };
        let mut covers = row![captioned(
            choice(
                self.cover_of(super::playlists::automatic_cover(songs), CHOICE_COVER),
                CoverChoice::Automatic
            )
            .into(),
            "Automatic"
        )]
        .spacing(10)
        .align_y(Alignment::Start);
        // The picture chosen from the computer, while it is the one chosen.
        if let CoverChoice::File(name) = chosen {
            covers = covers.push(choice(
                super::playlists::file_cover(name, CHOICE_COVER),
                CoverChoice::File(name.clone()),
            ));
        }
        for url in cover_choices(songs) {
            covers = covers.push(choice(
                self.cover(Some(url), CHOICE_COVER, crate::artwork::THUMB_MAX),
                CoverChoice::Song(url.to_owned()),
            ));
        }
        let computer = button(
            container(icon("image", 22.0, INK))
                .width(CHOICE_COVER)
                .height(CHOICE_COVER)
                .center_x(CHOICE_COVER)
                .center_y(CHOICE_COVER),
        )
        .on_press_maybe((!editor.picking).then_some(Message::ChooseCoverFile))
        .padding(4)
        .style(theme::outline_button);
        covers = covers.push(captioned(computer.into(), "From your computer"));
        let mut panel = column![
            self.playlist_name_field(width, 480.0, "Save"),
            label(self.tr("Cover").into_owned(), TEXT_CAPTION).color(DIM),
            covers.wrap().vertical_spacing(10),
        ]
        .spacing(12);
        if editor.cover_failed {
            panel = panel.push(
                label(
                    self.tr("That picture could not be used. Choose a PNG or JPEG file.")
                        .into_owned(),
                    TEXT_CAPTION,
                )
                .color(theme::ALERT),
            );
        }
        panel.into()
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
        // Narrow windows drop the album and then the artist and duration columns, which go under
        // the title. A column no song has anything for is left out too.
        let columns = width >= 560.0;
        let show_album = width >= 900.0
            && songs.iter().any(|track| {
                track
                    .album
                    .as_deref()
                    .is_some_and(|album| !album.trim().is_empty())
            });
        let show_duration = columns && songs.iter().any(|track| self.duration_of(track).is_some());
        let more_w = self.row_more_width();
        let fixed = INDEX_W
            + HEART_W
            + more_w
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
        } else if columns {
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
        if columns {
            table_head = table_head.push(column_head("ARTIST", artist_w));
        }
        if show_album {
            table_head = table_head.push(column_head("ALBUM", album_w));
        }
        if show_duration {
            table_head = table_head.push(column_head("DURATION", DURATION_W));
        }
        let table_head = table_head.push(Space::new().width(more_w + HEART_W));

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
        let dragged = self.dragged_row(dragging);
        for index in range.clone() {
            let track = &songs[index];
            let current = playing == Some(track.video_id.as_str());
            // The row under the keyboard cursor is outlined; the playing row stays inverted.
            let cursor = self.row_is_cursor(rows_name, index);
            let mut ink = theme::row_ink(current);
            // The row being dragged stays in its place, faded, while a label follows the pointer.
            if dragged == Some(index) {
                ink.title = FADED;
                ink.secondary = FADED;
            }
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
                            (index + 1).to_string()
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
                    if columns {
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
            let cells = if columns {
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
            let heart = self.row_heart(
                track,
                ink.icon,
                current,
                cover,
                self.row_click(dragging, Message::OpenSaveDialog(track.clone())),
            );
            let more = button(icon("more", 18.0, ink.icon))
                .on_press(self.row_click(dragging, Message::ToggleRowMenu(menu)))
                .padding(self.row_icon_padding(cover))
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
            // Rows keep the fixed pitch the virtual list counts with; a ••• menu opens over the
            // window.
            rows = rows.push(super::keyboard::mark_row(
                rows_name,
                index,
                drag_row(
                    column![line, hrule(RULE)].height(pitch),
                    Some(Message::DragPressed {
                        list: dragging,
                        index,
                    }),
                    Some(Message::DragHeld {
                        list: dragging,
                        index,
                    }),
                ),
            ));
        }
        let remaining = count.saturating_sub(range.end);
        if remaining > 0 {
            rows = rows.push(Space::new().height(layout::virtual_spacer_height(remaining, pitch)));
        }
        let mut area = stack![
            scrollable(rows)
                .id("library-content")
                .on_scroll(|viewport| Message::LibraryScrolled(viewport.absolute_offset()))
                .direction(theme::vbar())
                .style(theme::scroll_style)
                .height(Length::Fill)
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        if let Some(overlay) = self.drag_overlay(dragging, width) {
            area = area.push(overlay);
        }
        page.push(drag_area(area, self.drag_moves(dragging))).into()
    }
}
