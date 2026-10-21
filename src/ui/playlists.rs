//! The user's lists: Favorite songs, which is always there, and the playlists they make. A heart
//! opens the Save panel, which puts its song in any of them or in a new one. The Library page shows
//! the lists (`library.rs`); each list can be played, edited, reordered by dragging, and deleted,
//! but Favorite songs stays.

use super::drag::Edge;
use super::shell::{display_title, fit};
use super::theme::{
    self, DIM, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_SECTION, hrule, icon, label,
};
use super::*;
use crate::preferences::{
    MAX_LOCAL_FAVORITES, MAX_PLAYLIST_NAME_CHARS, MAX_PLAYLIST_TRACKS, MAX_PLAYLIST_TRACKS_TOTAL,
    MAX_PLAYLISTS, clean_playlist_name, next_playlist_id,
};
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

/// The id of the field a playlist's name is typed in. Only one is open at a time.
pub(super) const PLAYLIST_NAME_FIELD: &str = "playlist-name";

/// Room for the two buttons beside the name field, with the longest of their translations.
const NAME_BUTTONS_W: f32 = 250.0;
/// The narrowest the name field gets beside its buttons; with less room they go under it.
const NAME_FIELD_MIN_W: f32 = 140.0;

/// How often a list scrolls a row on while a dragged row is held at its edge.
const DRAG_SCROLL_EVERY: std::time::Duration = std::time::Duration::from_millis(110);

/// The Save panel: its widest, and the cover of each list in it.
const SAVE_PANEL_W: f32 = 440.0;
const SAVE_COVER: f32 = 40.0;

impl App {
    pub(super) fn playlist(&self, id: u64) -> Option<&Playlist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }

    fn playlist_position(&self, id: u64) -> Option<usize> {
        self.playlists.iter().position(|playlist| playlist.id == id)
    }

    /// The songs of a list, while it exists.
    pub(super) fn list_songs(&self, list: ListId) -> Option<&[Track]> {
        match list {
            ListId::Favorites => Some(&self.favorites),
            ListId::Playlist(id) => self.playlist(id).map(|playlist| playlist.tracks.as_slice()),
        }
    }

    /// The list the page shows, while it exists.
    pub(super) fn shown_list(&self) -> Option<ListId> {
        match self.page {
            Page::List(list) if self.list_songs(list).is_some() => Some(list),
            _ => None,
        }
    }

    fn list_contains(&self, list: ListId, video_id: &str) -> bool {
        match list {
            ListId::Favorites => self.liked.contains(video_id),
            ListId::Playlist(id) => self
                .playlist(id)
                .is_some_and(|playlist| playlist.contains(video_id)),
        }
    }

    /// Whether the song is in any of the user's lists, which fills its heart.
    pub(super) fn is_saved(&self, video_id: &str) -> bool {
        self.liked.contains(video_id) || self.in_playlists.contains(video_id)
    }

    /// Notes again which songs are in a playlist, after the playlists changed.
    pub(super) fn index_playlist_songs(&mut self) {
        self.in_playlists = self
            .playlists
            .iter()
            .flat_map(|playlist| &playlist.tracks)
            .map(|track| track.video_id.clone())
            .collect();
    }

    /// The name of a list's rows, for ••• menus and the keyboard cursor.
    pub(super) fn song_rows(list: ListId) -> &'static str {
        match list {
            ListId::Favorites => "favorites",
            ListId::Playlist(_) => "playlist",
        }
    }

    /// The covers that make a list's cover: the one chosen, or the automatic one. Favorite songs
    /// has a heart instead.
    pub(super) fn list_cover_urls(&self, list: ListId) -> Vec<&str> {
        let ListId::Playlist(id) = list else {
            return Vec::new();
        };
        let Some(playlist) = self.playlist(id) else {
            return Vec::new();
        };
        match &playlist.cover {
            Some(cover) => vec![cover.as_str()],
            None => automatic_cover(&playlist.tracks),
        }
    }

    /// The artwork size a list cover of `size` asks for: the whole cover, or a quarter of it.
    pub(super) fn list_cover_variant(&self, list: ListId, size: f32) -> u32 {
        cover_variant(self.list_cover_urls(list).len(), size, self.pixel_scale())
    }

    /// A list's cover: a heart for Favorite songs, the cover chosen, four song covers, or one.
    pub(super) fn list_cover(&self, list: ListId, size: f32) -> Element<'_, Message> {
        if list == ListId::Favorites {
            return container(icon("like-filled", (size * 0.4).max(14.0), INK))
                .width(size)
                .height(size)
                .center_x(size)
                .center_y(size)
                .style(theme::boxed)
                .into();
        }
        self.cover_of(self.list_cover_urls(list), size)
    }

    /// A cover made of `urls`: four make a square of four, fewer show the first, and none, a list
    /// without covers yet, shows the queue's icon.
    pub(super) fn cover_of<'a>(&self, urls: Vec<&str>, size: f32) -> Element<'a, Message> {
        if urls.is_empty() {
            return container(icon("queue", (size * 0.36).max(14.0), theme::FADED))
                .width(size)
                .height(size)
                .center_x(size)
                .center_y(size)
                .style(theme::boxed)
                .into();
        }
        let variant = cover_variant(urls.len(), size, self.pixel_scale());
        if urls.len() == 4 {
            let half = size / 2.0;
            return column![
                row![
                    self.cover(Some(urls[0]), half, variant),
                    self.cover(Some(urls[1]), half, variant)
                ],
                row![
                    self.cover(Some(urls[2]), half, variant),
                    self.cover(Some(urls[3]), half, variant)
                ],
            ]
            .into();
        }
        self.cover(urls.first().copied(), size, variant)
    }

    /// Handles the messages of the lists.
    pub(super) fn update_playlists(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenSaveDialog(track) => {
                self.close_save_dialog();
                // Chosen from a ••• menu, the menu has done its part.
                self.row_menu = None;
                self.save_dialog = Some(track);
                return self.preload_current_page_artwork();
            }
            Message::CloseSaveDialog => self.close_save_dialog(),
            Message::ToggleSaved { list, track } => self.toggle_saved(list, track),
            Message::NewPlaylist(track) => {
                if self.playlists.len() >= MAX_PLAYLISTS {
                    self.playlist_notice = Some(PlaylistNotice::TooManyPlaylists);
                    return Task::none();
                }
                self.playlist_notice = None;
                self.playlist_editor = Some(PlaylistEditor {
                    target: PlaylistEditTarget::New(track.map(Box::new)),
                    name: String::new(),
                    cover: None,
                });
                self.playlist_name_typed = true;
                return operation::focus(PLAYLIST_NAME_FIELD);
            }
            Message::EditPlaylist(id) => {
                let Some(playlist) = self.playlist(id) else {
                    return Task::none();
                };
                self.playlist_editor = Some(PlaylistEditor {
                    target: PlaylistEditTarget::Edit(id),
                    name: playlist.name.clone(),
                    cover: playlist.cover.clone(),
                });
                self.playlist_name_typed = true;
                return Task::batch([
                    operation::focus(PLAYLIST_NAME_FIELD),
                    operation::move_cursor_to_end(PLAYLIST_NAME_FIELD),
                    self.preload_current_page_artwork(),
                ]);
            }
            Message::PlaylistNameChanged(name) => {
                if let Some(editor) = &mut self.playlist_editor {
                    editor.name = name.chars().take(MAX_PLAYLIST_NAME_CHARS).collect();
                    self.playlist_name_typed = true;
                }
            }
            Message::PlaylistCoverChosen(cover) => {
                if let Some(editor) = &mut self.playlist_editor
                    && matches!(editor.target, PlaylistEditTarget::Edit(_))
                {
                    editor.cover = cover;
                }
            }
            Message::SubmitPlaylistEditor => return self.submit_playlist_editor(),
            Message::CancelPlaylistEditor => self.playlist_editor = None,
            Message::RemoveFromList { list, index } => {
                if self
                    .list_songs(list)
                    .is_some_and(|songs| index < songs.len())
                {
                    self.with_list_songs(list, |songs| {
                        songs.remove(index);
                    });
                    self.row_menu = None;
                    self.row_cursor = None;
                    self.persist();
                }
            }
            Message::MoveInList { list, index, up } => {
                let count = self.list_songs(list).map_or(0, <[Track]>::len);
                let Some(to) = step(index, up, count) else {
                    return Task::none();
                };
                self.with_list_songs(list, |songs| songs.swap(index, to));
                self.follow_row(App::song_rows(list), index, to);
                self.persist();
            }
            Message::MovePlaylist { index, up } => {
                let Some(to) = step(index, up, self.playlists.len()) else {
                    return Task::none();
                };
                Arc::make_mut(&mut self.playlists).swap(index, to);
                // Favorite songs is the first row of the Library page.
                self.follow_row("lists", index + 1, to + 1);
                self.persist();
            }
            Message::PlayPlaylist { id, index } => {
                let Some(playlist) = self.playlist(id) else {
                    return Task::none();
                };
                if index >= playlist.tracks.len() {
                    return Task::none();
                }
                let source = QueueSource {
                    title: playlist.name.clone(),
                    track_count: playlist.tracks.len(),
                    more: false,
                    browse: None,
                    thumbnail_url: self
                        .list_cover_urls(ListId::Playlist(id))
                        .first()
                        .map(|url| (*url).to_owned()),
                    album: false,
                };
                let tracks: Arc<[Track]> = Arc::from(playlist.tracks.as_slice());
                return self.update(Message::PlayCollection(tracks, index, source));
            }
            Message::ShufflePlaylist(id) => {
                let Some(playlist) = self.playlist(id) else {
                    return Task::none();
                };
                if playlist.tracks.is_empty() {
                    return Task::none();
                }
                let tracks: Arc<[Track]> = Arc::from(playlist.tracks.as_slice());
                return self.update(Message::ShuffleContext(tracks));
            }
            Message::AskDeletePlaylist(id) => {
                if self.playlist(id).is_some() {
                    return self.ask_confirmation(Confirmation::DeletePlaylist(id));
                }
            }
            Message::DeletePlaylist(id) => return self.delete_playlist(id),
            Message::DragPressed { list, index } => {
                self.row_drag = Some(RowDrag {
                    list,
                    from: index,
                    to: index,
                    moved: false,
                });
                self.drag_edge = Edge::None;
            }
            Message::DragEntered(index) => {
                let count = self.drag_count();
                if let Some(drag) = &mut self.row_drag
                    && count > 0
                {
                    drag.to = index.min(count - 1);
                    if drag.to != drag.from && !drag.moved {
                        drag.moved = true;
                        // Rows keep one height while they move, so the list's places hold.
                        self.row_menu = None;
                    }
                }
            }
            Message::DragEdge(edge, height) => {
                self.drag_list_height = height;
                if self.row_drag.is_none() {
                    return Task::none();
                }
                self.drag_edge = edge;
                self.drag_tick = self.drag_tick.wrapping_add(1);
                if edge != Edge::None {
                    return self.next_drag_tick();
                }
            }
            Message::DragTick(tick) => {
                if tick == self.drag_tick {
                    return self.drag_scroll();
                }
            }
            Message::DragReleased => return self.finish_row_drag(),
            _ => {}
        }
        Task::none()
    }

    fn close_save_dialog(&mut self) {
        self.save_dialog = None;
        self.playlist_notice = None;
        if self
            .playlist_editor
            .as_ref()
            .is_some_and(|editor| matches!(editor.target, PlaylistEditTarget::New(Some(_))))
        {
            self.playlist_editor = None;
        }
    }

    /// The covers the Save panel draws: its song's, and its lists'.
    pub(super) fn save_dialog_artwork(&self) -> Vec<(String, u32)> {
        let Some(track) = &self.save_dialog else {
            return Vec::new();
        };
        let mut requests: Vec<(String, u32)> = track
            .thumbnail_url
            .iter()
            .map(|url| (url.clone(), crate::artwork::THUMB_MAX))
            .collect();
        for playlist in self.playlists.iter() {
            let list = ListId::Playlist(playlist.id);
            let variant = self.list_cover_variant(list, SAVE_COVER);
            requests.extend(
                self.list_cover_urls(list)
                    .into_iter()
                    .map(|url| (url.to_owned(), variant)),
            );
        }
        requests
    }

    /// Puts a song in a list, or takes it out when it is there. A full list says so in the panel.
    fn toggle_saved(&mut self, list: ListId, mut track: Track) {
        self.playlist_notice = None;
        if self.list_contains(list, &track.video_id) {
            let video_id = track.video_id;
            self.with_list_songs(list, |songs| songs.retain(|song| song.video_id != video_id));
            self.persist();
            return;
        }
        let total: usize = self
            .playlists
            .iter()
            .map(|playlist| playlist.tracks.len())
            .sum();
        let full = match list {
            ListId::Favorites => (self.favorites.len() >= MAX_LOCAL_FAVORITES)
                .then_some(PlaylistNotice::FavoritesFull),
            ListId::Playlist(id) => match self.playlist(id) {
                None => return,
                Some(playlist) if playlist.tracks.len() >= MAX_PLAYLIST_TRACKS => {
                    Some(PlaylistNotice::PlaylistFull)
                }
                Some(_) if total >= MAX_PLAYLIST_TRACKS_TOTAL => Some(PlaylistNotice::AllFull),
                Some(_) => None,
            },
        };
        if full.is_some() {
            self.playlist_notice = full;
            return;
        }
        if track.duration_secs.is_none() {
            track.duration_secs = self.stored_duration_secs(&track.video_id);
        }
        self.with_list_songs(list, |songs| songs.push(track));
        self.persist();
    }

    /// Changes a list's songs, and keeps what is worked out from them in step.
    fn with_list_songs(&mut self, list: ListId, change: impl FnOnce(&mut Vec<Track>)) {
        match list {
            ListId::Favorites => {
                change(Arc::make_mut(&mut self.favorites));
                self.favorites_context = Arc::from(self.favorites.as_slice());
                self.liked = self
                    .favorites
                    .iter()
                    .map(|track| track.video_id.clone())
                    .collect();
            }
            ListId::Playlist(id) => {
                if let Some(position) = self.playlist_position(id) {
                    change(&mut Arc::make_mut(&mut self.playlists)[position].tracks);
                }
            }
        }
    }

    /// The open menu and the keyboard cursor go with a row that moved.
    fn follow_row(&mut self, list: &str, from: usize, to: usize) {
        for row in [&mut self.row_menu, &mut self.row_cursor]
            .into_iter()
            .flatten()
        {
            if row.list == list && row.index == from {
                row.index = to;
            }
        }
    }

    /// Makes the playlist, or saves its new name and cover. A name of spaces alone does nothing
    /// and leaves the field open.
    fn submit_playlist_editor(&mut self) -> Task<Message> {
        let Some(editor) = self.playlist_editor.take() else {
            return Task::none();
        };
        let Some(name) = clean_playlist_name(&editor.name) else {
            self.playlist_editor = Some(editor);
            return Task::none();
        };
        match editor.target {
            PlaylistEditTarget::Edit(id) => {
                if let Some(position) = self.playlist_position(id) {
                    let playlist = &mut Arc::make_mut(&mut self.playlists)[position];
                    playlist.name = name;
                    playlist.cover = editor.cover;
                    self.persist();
                }
                self.preload_current_page_artwork()
            }
            PlaylistEditTarget::New(track) => {
                if self.playlists.len() >= MAX_PLAYLISTS {
                    self.playlist_notice = Some(PlaylistNotice::TooManyPlaylists);
                    return Task::none();
                }
                let id = next_playlist_id(&self.playlists);
                Arc::make_mut(&mut self.playlists).push(Playlist {
                    id,
                    name,
                    tracks: Vec::new(),
                    cover: None,
                });
                match track {
                    // Made in the Save panel: the song goes in, and the panel shows it ticked.
                    Some(track) => {
                        self.toggle_saved(ListId::Playlist(id), *track);
                        Task::none()
                    }
                    None => {
                        self.persist();
                        self.navigate(Page::List(ListId::Playlist(id)))
                    }
                }
            }
        }
    }

    /// The second press on Delete. The pages of the playlist are dropped from Back and Forward,
    /// and its own page gives way to the Library.
    fn delete_playlist(&mut self, id: u64) -> Task<Message> {
        if !matches!(
            self.confirmation.take(),
            Some(PendingConfirmation {
                kind: Confirmation::DeletePlaylist(pending),
                ..
            }) if pending == id
        ) {
            return Task::none();
        }
        let Some(position) = self.playlist_position(id) else {
            return Task::none();
        };
        Arc::make_mut(&mut self.playlists).remove(position);
        if self
            .playlist_editor
            .as_ref()
            .is_some_and(|editor| editor.target == PlaylistEditTarget::Edit(id))
        {
            self.playlist_editor = None;
        }
        self.persist();
        let page = Page::List(ListId::Playlist(id));
        self.history.retain(|seen| *seen != page);
        self.forward.retain(|seen| *seen != page);
        if self.page != page {
            return Task::none();
        }
        if self.history.last() == Some(&Page::Library) {
            self.history.pop();
        }
        self.navigate_internal(Page::Library, false)
    }

    /// How many rows the dragged row's list has.
    fn drag_count(&self) -> usize {
        self.row_drag
            .map_or(0, |drag| self.drag_list_count(drag.list))
    }

    /// How many rows a list with draggable rows has.
    fn drag_list_count(&self, list: DragList) -> usize {
        match list {
            DragList::Songs(list) => self.list_songs(list).map_or(0, <[Track]>::len),
            DragList::Lists => self.playlists.len(),
        }
    }

    fn next_drag_tick(&self) -> Task<Message> {
        let tick = self.drag_tick;
        Task::perform(
            async move { tokio::time::sleep(DRAG_SCROLL_EVERY).await },
            move |()| Message::DragTick(tick),
        )
    }

    /// Scrolls the dragged row's list a row on, toward the edge the pointer is at. The pointer
    /// stays where it is, so the row under it, where the dragged row lands, is the next one.
    fn drag_scroll(&mut self) -> Task<Message> {
        let Some(drag) = self.row_drag else {
            return Task::none();
        };
        let direction = match self.drag_edge {
            Edge::Up => -1.0,
            Edge::Down => 1.0,
            Edge::None => return Task::none(),
        };
        let count = self.drag_count();
        let density = self.preferences.ui_density;
        let (id, pitch, offset) = match drag.list {
            DragList::Songs(_) => (
                "library-content",
                super::library::song_row_pitch(density),
                &mut self.library_scroll,
            ),
            DragList::Lists => (
                "library-lists",
                super::library::LIST_ROW_PITCH,
                &mut self.lists_scroll,
            ),
        };
        let rows = match drag.list {
            DragList::Songs(_) => count,
            // Favorite songs comes before the playlists.
            DragList::Lists => count + 1,
        };
        let furthest = (rows as f32 * pitch - self.drag_list_height).max(0.0);
        let y = (offset.y + direction * pitch).clamp(0.0, furthest);
        if (y - offset.y).abs() < 0.5 || count == 0 {
            return Task::none();
        }
        offset.y = y;
        let offset = *offset;
        if let Some(drag) = &mut self.row_drag {
            drag.to = if direction > 0.0 {
                (drag.to + 1).min(count - 1)
            } else {
                drag.to.saturating_sub(1)
            };
            if drag.to != drag.from && !drag.moved {
                drag.moved = true;
                self.row_menu = None;
            }
        }
        Task::batch([
            operation::scroll_to(id, offset),
            self.next_drag_tick(),
            self.scroll_preload_task(),
        ])
    }

    /// Ends a drag: the row moves to where it was let go.
    pub(super) fn finish_row_drag(&mut self) -> Task<Message> {
        let Some(drag) = self.row_drag.take() else {
            return Task::none();
        };
        self.drag_edge = Edge::None;
        self.drag_tick = self.drag_tick.wrapping_add(1);
        let count = self.drag_list_count(drag.list);
        if drag.from == drag.to || drag.from >= count {
            return Task::none();
        }
        let (from, to) = (drag.from, drag.to.min(count - 1));
        match drag.list {
            DragList::Songs(list) => self.with_list_songs(list, |songs| {
                let song = songs.remove(from);
                songs.insert(to, song);
            }),
            DragList::Lists => {
                let playlists = Arc::make_mut(&mut self.playlists);
                let playlist = playlists.remove(from);
                playlists.insert(to, playlist);
            }
        }
        self.row_menu = None;
        self.row_cursor = None;
        self.persist();
        Task::none()
    }

    /// Where the row shown at `shown` in `list` comes from while one of its rows is dragged.
    pub(super) fn dragged_item(&self, list: DragList, shown: usize) -> usize {
        match self.row_drag {
            Some(drag) if drag.list == list => super::drag::shown_item(shown, drag.from, drag.to),
            _ => shown,
        }
    }

    /// The row of `list` that is being dragged, where it shows now.
    pub(super) fn dragged_row(&self, list: DragList) -> Option<usize> {
        self.row_drag
            .filter(|drag| drag.list == list && drag.moved)
            .map(|drag| drag.to)
    }

    /// What a press on a row of `list` does: `click`, unless a row of it was dragged, which ends
    /// the drag instead.
    pub(super) fn row_click(&self, list: DragList, click: Message) -> Message {
        if self
            .row_drag
            .is_some_and(|drag| drag.list == list && drag.moved)
        {
            Message::DragReleased
        } else {
            click
        }
    }

    /// The message for the pointer coming onto row `index` of `list`, only while one of its rows
    /// is dragged.
    pub(super) fn drag_enter(&self, list: DragList, index: usize) -> Option<Message> {
        self.row_drag
            .is_some_and(|drag| drag.list == list)
            .then_some(Message::DragEntered(index))
    }

    /// The edge reports of a list's area, only while one of its rows is dragged.
    pub(super) fn drag_edges(&self, list: DragList) -> Option<fn(Edge, f32) -> Message> {
        self.row_drag
            .is_some_and(|drag| drag.list == list)
            .then_some(Message::DragEdge as fn(Edge, f32) -> Message)
    }

    /// A notice about the lists in the interface's language.
    pub(super) fn playlist_notice_text(&self, notice: &PlaylistNotice) -> String {
        let fill = |template: &'static str, part: usize| {
            self.tr_static(template)
                .replacen("{}", &part.to_string(), 1)
        };
        match notice {
            PlaylistNotice::TooManyPlaylists => fill("Playlists are limited to {}.", MAX_PLAYLISTS),
            PlaylistNotice::PlaylistFull => {
                fill("A playlist holds up to {} songs.", MAX_PLAYLIST_TRACKS)
            }
            PlaylistNotice::AllFull => fill(
                "Playlists hold up to {} songs in all.",
                MAX_PLAYLIST_TRACKS_TOTAL,
            ),
            PlaylistNotice::FavoritesFull => self.tr_string(format!(
                "Local favorites are limited to {MAX_LOCAL_FAVORITES} tracks."
            )),
        }
    }

    /// The field for a playlist's name, at most `widest`, with its button and Cancel beside it in
    /// `room`, or under it when the room is too small. Enter does what the button does; Escape
    /// cancels.
    pub(super) fn playlist_name_field(
        &self,
        room: f32,
        widest: f32,
        submit: &'static str,
    ) -> Element<'_, Message> {
        let name = self
            .playlist_editor
            .as_ref()
            .map_or("", |editor| editor.name.as_str());
        let ready = clean_playlist_name(name).is_some();
        let beside = room - NAME_BUTTONS_W >= NAME_FIELD_MIN_W;
        let field_width = if beside {
            (room - NAME_BUTTONS_W).min(widest)
        } else {
            room.min(widest)
        };
        let field = container(
            text_input(self.tr_static("Playlist name"), name)
                .id(PLAYLIST_NAME_FIELD)
                .on_input(Message::PlaylistNameChanged)
                .on_submit(Message::SubmitPlaylistEditor)
                .font(MONO)
                .size(TEXT_BODY)
                .padding([8, 12])
                .style(theme::input_style),
        )
        .width(field_width.max(1.0))
        .style(theme::boxed);
        let buttons = row![
            button(label(self.tr(submit).into_owned(), TEXT_BODY))
                .on_press_maybe(ready.then_some(Message::SubmitPlaylistEditor))
                .padding([8, 16])
                .style(theme::outline_button),
            button(label(self.tr("Cancel").into_owned(), TEXT_BODY))
                .on_press(Message::CancelPlaylistEditor)
                .padding([8, 16])
                .style(theme::outline_button),
        ]
        .spacing(10);
        if beside {
            row![field, buttons]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
        } else {
            column![field, buttons].spacing(10).into()
        }
    }

    /// How far down the window the Save panel starts.
    pub(super) fn save_panel_top(&self) -> f32 {
        (self.window_height * 0.14).clamp(24.0, 140.0)
    }

    /// The Save panel over the window: the song, then each list with a box that is filled when the
    /// song is in it, then New playlist. A press on a list puts the song in or takes it out.
    pub(super) fn save_dialog_view<'a>(&'a self, track: &'a Track) -> Element<'a, Message> {
        let width = SAVE_PANEL_W.min(self.window_width - 32.0).max(200.0);
        let inner = width - 48.0;
        let song = row![
            self.cover(
                track.thumbnail_url.as_deref(),
                SAVE_COVER,
                crate::artwork::THUMB_MAX
            ),
            column![
                label(
                    fit(
                        display_title(&track.title),
                        inner - SAVE_COVER - 16.0,
                        TEXT_BODY
                    ),
                    TEXT_BODY
                ),
                label(
                    fit(&track.artist, inner - SAVE_COVER - 16.0, TEXT_CAPTION),
                    TEXT_CAPTION
                )
                .color(DIM),
            ]
            .spacing(4),
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let entry = |list: ListId, name: String, count: usize| {
            let inside = self.list_contains(list, &track.video_id);
            let tick = container(Space::new().width(10).height(10))
                .padding(3)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(if inside {
                        INK
                    } else {
                        theme::BLACK
                    })),
                    border: iced::Border {
                        color: INK,
                        width: theme::hairline(),
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                });
            let text_width = inner - SAVE_COVER - 16.0 - 16.0 - 32.0;
            button(
                row![
                    self.list_cover(list, SAVE_COVER),
                    column![
                        label(fit(&name, text_width, TEXT_BODY), TEXT_BODY),
                        label(self.song_count(count, false), TEXT_CAPTION).color(DIM),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    container(tick).padding([0, 8]),
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            )
            .on_press(Message::ToggleSaved {
                list,
                track: track.clone(),
            })
            .width(Length::Fill)
            .padding([6, 4])
            .style(theme::row_button)
        };
        let mut lists = column![entry(
            ListId::Favorites,
            self.tr("Favorite songs").into_owned(),
            self.favorites.len()
        )];
        for playlist in self.playlists.iter() {
            lists = lists.push(entry(
                ListId::Playlist(playlist.id),
                playlist.name.clone(),
                playlist.tracks.len(),
            ));
        }
        let naming = self.playlist_editor.as_ref().is_some_and(
            |editor| matches!(&editor.target, PlaylistEditTarget::New(Some(new)) if new.video_id == track.video_id),
        );
        let new: Element<'_, Message> = if naming {
            container(self.playlist_name_field(inner, inner, "Create"))
                .padding([8, 4])
                .into()
        } else {
            button(
                row![
                    container(icon("plus", 16.0, INK))
                        .width(SAVE_COVER)
                        .height(SAVE_COVER)
                        .center_x(SAVE_COVER)
                        .center_y(SAVE_COVER)
                        .style(theme::boxed),
                    label(self.tr("New playlist").into_owned(), TEXT_BODY),
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            )
            .on_press(Message::NewPlaylist(Some(track.clone())))
            .width(Length::Fill)
            .padding([6, 4])
            .style(theme::row_button)
            .into()
        };
        // The lists scroll on their own when there are many; the rest of the panel stays.
        let lists = container(
            scrollable(lists.spacing(2))
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(4).scroller_width(4),
                ))
                .style(theme::scroll_style),
        )
        .max_height((self.window_height - self.save_panel_top() - 340.0).clamp(100.0, 420.0));
        let mut panel = column![
            row![
                text(self.tr("Save to"))
                    .size(TEXT_SECTION)
                    .font(MONO)
                    .width(Length::Fill),
                button(icon("close", 14.0, INK))
                    .on_press(Message::CloseSaveDialog)
                    .padding(6)
                    .style(theme::icon_button),
            ]
            .align_y(Alignment::Center),
            song,
            hrule(RULE),
            lists,
            new,
        ]
        .spacing(14);
        if let Some(notice) = &self.playlist_notice {
            panel = panel.push(label(self.playlist_notice_text(notice), TEXT_CAPTION).color(DIM));
        }
        panel = panel.push(
            row![
                Space::new().width(Length::Fill),
                button(label(self.tr("Done").into_owned(), TEXT_BODY))
                    .on_press(Message::CloseSaveDialog)
                    .padding([8, 20])
                    .style(theme::outline_button),
            ]
            .width(Length::Fill),
        );
        container(panel)
            .width(width)
            .padding(24)
            .style(theme::boxed)
            .into()
    }
}

/// The cover a playlist gets without one chosen: the covers of its first four songs with one, or
/// that of its first song when fewer have one.
pub(super) fn automatic_cover(songs: &[Track]) -> Vec<&str> {
    let mut urls: Vec<&str> = Vec::new();
    for url in songs
        .iter()
        .filter_map(|track| track.thumbnail_url.as_deref())
    {
        if !urls.contains(&url) {
            urls.push(url);
            if urls.len() == 4 {
                return urls;
            }
        }
    }
    urls.truncate(1);
    urls
}

/// The artwork size a cover of `size` made of `count` covers asks for.
fn cover_variant(count: usize, size: f32, scale: f32) -> u32 {
    let shown = if count == 4 { size / 2.0 } else { size };
    crate::artwork::variant_for_pixels(shown * scale)
}

/// The place one step up or down from `index` in a list of `count`, when there is one.
fn step(index: usize, up: bool, count: usize) -> Option<usize> {
    let to = if up { index.checked_sub(1)? } else { index + 1 };
    (index < count && to < count).then_some(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Track {
        Track {
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            artist_page: None,
            album_page: None,
            thumbnail_url: Some(format!("https://covers.test/{id}.jpg")),
            duration_secs: None,
        }
    }

    fn library() -> App {
        App {
            page: Page::Library,
            ..App::default()
        }
    }

    /// Makes a playlist from the Library page and returns its id.
    fn make(app: &mut App, name: &str) -> u64 {
        drop(app.update(Message::NewPlaylist(None)));
        drop(app.update(Message::PlaylistNameChanged(name.into())));
        drop(app.update(Message::SubmitPlaylistEditor));
        app.playlists.last().expect("a playlist").id
    }

    fn songs_of(app: &App, list: ListId) -> Vec<&str> {
        app.list_songs(list)
            .expect("the list")
            .iter()
            .map(|track| track.video_id.as_str())
            .collect()
    }

    fn save(app: &mut App, list: ListId, id: &str) {
        drop(app.update(Message::ToggleSaved {
            list,
            track: song(id),
        }));
    }

    #[test]
    fn a_playlist_made_from_the_library_is_named_and_opened() {
        let mut app = library();
        let id = make(&mut app, "  Road   trip ");
        assert_eq!(app.playlists.len(), 1);
        assert_eq!(app.playlist(id).unwrap().name, "Road trip");
        assert_eq!(app.page, Page::List(ListId::Playlist(id)));
        assert!(app.playlist_editor.is_none());
        // Back returns to the Library, and a second playlist gets an id of its own.
        drop(app.update(Message::Back));
        assert_eq!(app.page, Page::Library);
        let other = make(&mut app, "Gym");
        assert_ne!(other, id);
    }

    #[test]
    fn a_blank_name_makes_nothing_and_keeps_the_field_open() {
        let mut app = library();
        drop(app.update(Message::NewPlaylist(None)));
        drop(app.update(Message::PlaylistNameChanged("   ".into())));
        drop(app.update(Message::SubmitPlaylistEditor));
        assert!(app.playlists.is_empty());
        assert!(app.playlist_editor.is_some());
        drop(app.update(Message::CancelPlaylistEditor));
        assert!(app.playlist_editor.is_none());
        assert!(app.playlists.is_empty());
    }

    #[test]
    fn a_name_is_cut_at_its_longest() {
        let mut app = library();
        drop(app.update(Message::NewPlaylist(None)));
        drop(app.update(Message::PlaylistNameChanged("ñ".repeat(200))));
        let typed = &app.playlist_editor.as_ref().unwrap().name;
        assert_eq!(typed.chars().count(), MAX_PLAYLIST_NAME_CHARS);
    }

    #[test]
    fn the_save_panel_puts_a_song_in_a_list_and_takes_it_out() {
        let mut app = library();
        let mix = make(&mut app, "Mix");
        drop(app.update(Message::OpenSaveDialog(song("a"))));
        assert!(app.save_dialog.is_some());
        assert!(!app.is_saved("a"));
        save(&mut app, ListId::Playlist(mix), "a");
        save(&mut app, ListId::Favorites, "a");
        assert_eq!(songs_of(&app, ListId::Playlist(mix)), ["a"]);
        assert_eq!(songs_of(&app, ListId::Favorites), ["a"]);
        assert!(app.is_saved("a"));
        assert_eq!(app.favorites_context.len(), 1);
        // A second press takes it out again; the heart empties once no list has it.
        save(&mut app, ListId::Favorites, "a");
        assert!(app.is_saved("a"));
        save(&mut app, ListId::Playlist(mix), "a");
        assert!(!app.is_saved("a"));
        assert!(songs_of(&app, ListId::Favorites).is_empty());
        drop(app.update(Message::CloseSaveDialog));
        assert!(app.save_dialog.is_none());
    }

    #[test]
    fn a_playlist_made_in_the_save_panel_takes_the_song() {
        let mut app = library();
        drop(app.update(Message::OpenSaveDialog(song("a"))));
        drop(app.update(Message::NewPlaylist(Some(song("a")))));
        drop(app.update(Message::PlaylistNameChanged("Mix".into())));
        drop(app.update(Message::SubmitPlaylistEditor));
        let id = app.playlists[0].id;
        assert_eq!(songs_of(&app, ListId::Playlist(id)), ["a"]);
        // The panel stays open over the page it was opened on.
        assert!(app.save_dialog.is_some());
        assert_eq!(app.page, Page::Library);
        // Closing the panel closes a name field it had open.
        drop(app.update(Message::NewPlaylist(Some(song("a")))));
        drop(app.update(Message::CloseSaveDialog));
        assert!(app.playlist_editor.is_none());
    }

    #[test]
    fn a_full_list_takes_no_more_songs_and_says_so() {
        let mut app = library();
        let id = make(&mut app, "Full");
        Arc::make_mut(&mut app.playlists)[0].tracks = (0..MAX_PLAYLIST_TRACKS)
            .map(|index| song(&format!("s{index}")))
            .collect();
        save(&mut app, ListId::Playlist(id), "new");
        assert_eq!(app.playlist_notice, Some(PlaylistNotice::PlaylistFull));
        assert_eq!(app.playlist(id).unwrap().tracks.len(), MAX_PLAYLIST_TRACKS);
        // Together the playlists have a limit of their own.
        for number in 1..MAX_PLAYLIST_TRACKS_TOTAL / MAX_PLAYLIST_TRACKS {
            let other = make(&mut app, &format!("Full {number}"));
            let position = app.playlist_position(other).unwrap();
            Arc::make_mut(&mut app.playlists)[position].tracks = (0..MAX_PLAYLIST_TRACKS)
                .map(|index| song(&format!("p{number}-{index}")))
                .collect();
        }
        let room = make(&mut app, "Empty");
        save(&mut app, ListId::Playlist(room), "new");
        assert_eq!(app.playlist_notice, Some(PlaylistNotice::AllFull));
        assert!(app.playlist(room).unwrap().tracks.is_empty());
        // Favorite songs has its own.
        Arc::make_mut(&mut app.favorites)
            .extend((0..MAX_LOCAL_FAVORITES).map(|index| song(&format!("f{index}"))));
        save(&mut app, ListId::Favorites, "new");
        assert_eq!(app.playlist_notice, Some(PlaylistNotice::FavoritesFull));
    }

    #[test]
    fn no_playlist_is_made_past_the_limit() {
        let mut app = library();
        for number in 0..MAX_PLAYLISTS {
            make(&mut app, &format!("P{number}"));
            drop(app.update(Message::Navigate(Page::Library)));
        }
        drop(app.update(Message::NewPlaylist(None)));
        assert!(app.playlist_editor.is_none());
        assert_eq!(app.playlist_notice, Some(PlaylistNotice::TooManyPlaylists));
        assert_eq!(app.playlists.len(), MAX_PLAYLISTS);
    }

    #[test]
    fn songs_move_and_go_and_the_menu_follows_its_song() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Mix"));
        for id in ["a", "b", "c"] {
            save(&mut app, list, id);
        }
        app.row_menu = Some(RowMenu {
            list: "playlist",
            index: 2,
        });
        drop(app.update(Message::MoveInList {
            list,
            index: 2,
            up: true,
        }));
        assert_eq!(songs_of(&app, list), ["a", "c", "b"]);
        assert_eq!(app.row_menu.as_ref().map(|menu| menu.index), Some(1));
        // The ends stay put.
        drop(app.update(Message::MoveInList {
            list,
            index: 0,
            up: true,
        }));
        drop(app.update(Message::MoveInList {
            list,
            index: 2,
            up: false,
        }));
        assert_eq!(songs_of(&app, list), ["a", "c", "b"]);
        drop(app.update(Message::RemoveFromList { list, index: 1 }));
        assert_eq!(songs_of(&app, list), ["a", "b"]);
        assert!(app.row_menu.is_none());
        // A place past the end does nothing.
        drop(app.update(Message::RemoveFromList { list, index: 5 }));
        assert_eq!(songs_of(&app, list), ["a", "b"]);
        // Favorite songs moves and loses songs the same way.
        for id in ["x", "y"] {
            save(&mut app, ListId::Favorites, id);
        }
        drop(app.update(Message::MoveInList {
            list: ListId::Favorites,
            index: 0,
            up: false,
        }));
        assert_eq!(songs_of(&app, ListId::Favorites), ["y", "x"]);
        assert_eq!(app.favorites_context[0].video_id, "y");
        drop(app.update(Message::RemoveFromList {
            list: ListId::Favorites,
            index: 0,
        }));
        assert!(!app.liked.contains("y"));
    }

    #[test]
    fn playlists_move_on_the_library_page() {
        let mut app = library();
        let first = make(&mut app, "One");
        let second = make(&mut app, "Two");
        drop(app.update(Message::MovePlaylist { index: 1, up: true }));
        let order: Vec<u64> = app.playlists.iter().map(|playlist| playlist.id).collect();
        assert_eq!(order, [second, first]);
    }

    #[test]
    fn a_dragged_song_lands_where_it_is_let_go() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Mix"));
        for id in ["a", "b", "c", "d"] {
            save(&mut app, list, id);
        }
        let songs = DragList::Songs(list);
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 0,
        }));
        // Before it moves, a row still plays on a click.
        assert!(matches!(
            app.row_click(songs, Message::PlayPlaylist { id: 0, index: 0 }),
            Message::PlayPlaylist { .. }
        ));
        drop(app.update(Message::DragEntered(2)));
        assert_eq!(app.dragged_item(songs, 2), 0);
        assert_eq!(app.dragged_item(songs, 0), 1);
        assert!(matches!(
            app.row_click(songs, Message::PlayPlaylist { id: 0, index: 0 }),
            Message::DragReleased
        ));
        drop(app.update(Message::PointerHeld(false)));
        assert_eq!(songs_of(&app, list), ["b", "c", "a", "d"]);
        assert!(app.row_drag.is_none());
        // Let go where it started, nothing moves.
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 1,
        }));
        drop(app.update(Message::DragEntered(3)));
        drop(app.update(Message::DragEntered(1)));
        drop(app.update(Message::DragReleased));
        assert_eq!(songs_of(&app, list), ["b", "c", "a", "d"]);
    }

    #[test]
    fn a_dragged_playlist_lands_where_it_is_let_go_and_favorites_stays_first() {
        let mut app = library();
        let ids: Vec<u64> = ["One", "Two", "Three"]
            .into_iter()
            .map(|name| {
                let id = make(&mut app, name);
                drop(app.update(Message::Navigate(Page::Library)));
                id
            })
            .collect();
        drop(app.update(Message::DragPressed {
            list: DragList::Lists,
            index: 2,
        }));
        drop(app.update(Message::DragEntered(0)));
        drop(app.update(Message::PointerHeld(false)));
        let order: Vec<u64> = app.playlists.iter().map(|playlist| playlist.id).collect();
        assert_eq!(order, [ids[2], ids[0], ids[1]]);
    }

    #[test]
    fn a_drag_at_the_edge_scrolls_the_list_and_carries_the_row_on() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Long"));
        for index in 0..40 {
            save(&mut app, list, &format!("s{index}"));
        }
        let songs = DragList::Songs(list);
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 3,
        }));
        drop(app.update(Message::DragEntered(5)));
        drop(app.update(Message::DragEdge(Edge::Down, 300.0)));
        let tick = app.drag_tick;
        drop(app.update(Message::DragTick(tick)));
        assert_eq!(app.row_drag.map(|drag| drag.to), Some(6));
        assert!(app.library_scroll.y > 0.0);
        // A tick from before the pointer left the edge does nothing.
        drop(app.update(Message::DragEdge(Edge::None, 300.0)));
        drop(app.update(Message::DragTick(tick)));
        assert_eq!(app.row_drag.map(|drag| drag.to), Some(6));
    }

    #[test]
    fn a_playlist_is_deleted_only_on_the_second_press_and_its_page_goes() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        assert_eq!(app.page, Page::List(ListId::Playlist(id)));
        drop(app.update(Message::DeletePlaylist(id)));
        assert_eq!(app.playlists.len(), 1);
        drop(app.update(Message::AskDeletePlaylist(id)));
        // Another press in between drops the question.
        drop(app.update(Message::EditPlaylist(id)));
        drop(app.update(Message::DeletePlaylist(id)));
        assert_eq!(app.playlists.len(), 1);
        drop(app.update(Message::AskDeletePlaylist(id)));
        drop(app.update(Message::DeletePlaylist(id)));
        assert!(app.playlists.is_empty());
        assert_eq!(app.page, Page::Library);
        assert!(
            !app.history
                .iter()
                .chain(&app.forward)
                .any(|page| *page == Page::List(ListId::Playlist(id)))
        );
    }

    #[test]
    fn an_edited_playlist_keeps_its_songs_and_takes_the_cover_chosen() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        let list = ListId::Playlist(id);
        for song_id in ["a", "b"] {
            save(&mut app, list, song_id);
        }
        assert_eq!(app.list_cover_urls(list), ["https://covers.test/a.jpg"]);
        drop(app.update(Message::EditPlaylist(id)));
        assert_eq!(app.playlist_editor.as_ref().unwrap().name, "Mix");
        drop(app.update(Message::PlaylistNameChanged("Night drive".into())));
        drop(app.update(Message::PlaylistCoverChosen(Some(
            "https://covers.test/b.jpg".into(),
        ))));
        drop(app.update(Message::SubmitPlaylistEditor));
        let playlist = app.playlist(id).unwrap();
        assert_eq!(playlist.name, "Night drive");
        assert_eq!(songs_of(&app, list), ["a", "b"]);
        assert_eq!(app.list_cover_urls(list), ["https://covers.test/b.jpg"]);
        // Cancel leaves both as they were.
        drop(app.update(Message::EditPlaylist(id)));
        drop(app.update(Message::PlaylistCoverChosen(None)));
        drop(app.update(Message::CancelPlaylistEditor));
        assert_eq!(app.list_cover_urls(list), ["https://covers.test/b.jpg"]);
    }

    #[test]
    fn four_songs_with_covers_make_a_mosaic() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Mix"));
        for id in ["a", "b", "c"] {
            save(&mut app, list, id);
        }
        assert_eq!(app.list_cover_urls(list).len(), 1);
        save(&mut app, list, "d");
        assert_eq!(app.list_cover_urls(list).len(), 4);
        assert!(app.list_cover_urls(ListId::Favorites).is_empty());
    }

    #[test]
    fn a_playlist_plays_from_the_song_chosen_and_names_itself() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        for song_id in ["a", "b", "c"] {
            save(&mut app, ListId::Playlist(id), song_id);
        }
        drop(app.update(Message::PlayPlaylist { id, index: 1 }));
        assert_eq!(app.queue.len(), 3);
        assert_eq!(app.queue_index, Some(1));
        let source = app.queue_source.as_ref().expect("a source");
        assert_eq!(source.title, "Mix");
        assert_eq!(source.track_count, 3);
        assert!(source.browse.is_none());
        // A place past the end, or a playlist that is gone, plays nothing.
        drop(app.update(Message::PlayPlaylist { id, index: 9 }));
        drop(app.update(Message::PlayPlaylist { id: 77, index: 0 }));
        assert_eq!(app.queue_index, Some(1));
    }

    #[test]
    fn escape_closes_the_name_field_then_the_save_panel() {
        let mut app = App {
            search_query: "typed".into(),
            ..App::default()
        };
        drop(app.update(Message::OpenSaveDialog(song("a"))));
        drop(app.update(Message::NewPlaylist(Some(song("a")))));
        drop(app.update(Message::PlaylistNameChanged("Mi".into())));
        // The field takes the Escape that unfocuses it.
        drop(app.update(Message::SearchFieldEscape { repeat: false }));
        assert!(app.playlist_editor.is_none());
        assert!(app.save_dialog.is_some());
        assert_eq!(app.search_query, "typed");
        drop(app.escape_outside_search());
        assert!(app.save_dialog.is_none());
    }

    #[test]
    fn leaving_the_page_closes_the_name_field_and_drops_a_drag() {
        let mut app = library();
        drop(app.update(Message::NewPlaylist(None)));
        drop(app.update(Message::DragPressed {
            list: DragList::Lists,
            index: 0,
        }));
        drop(app.update(Message::Navigate(Page::Home)));
        assert!(app.playlist_editor.is_none());
        assert!(app.row_drag.is_none());
    }

    #[test]
    fn the_playlists_are_saved_and_brought_back_by_an_import() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        save(&mut app, ListId::Playlist(id), "a");
        let saved = app.snapshot();
        assert_eq!(saved.playlists.len(), 1);
        let json = serde_json::to_string(&saved).unwrap();
        let read: Snapshot = serde_json::from_str(&json).unwrap();
        let mut other = library();
        drop(other.apply_snapshot(read));
        assert_eq!(other.playlists, app.playlists);
        assert!(other.is_saved("a"));
        // A state without playlists writes none and reads as none.
        let empty = serde_json::to_string(&App::default().snapshot()).unwrap();
        assert!(!empty.contains("playlists"));
    }

    #[test]
    fn notices_read_in_the_interface_language() {
        let mut app = App::default();
        app.preferences.ui_language = crate::preferences::UiLanguage::Spanish;
        assert_eq!(
            app.playlist_notice_text(&PlaylistNotice::TooManyPlaylists),
            format!("Las listas están limitadas a {MAX_PLAYLISTS}.")
        );
        assert!(
            app.playlist_notice_text(&PlaylistNotice::FavoritesFull)
                .contains(&MAX_LOCAL_FAVORITES.to_string())
        );
    }
}
