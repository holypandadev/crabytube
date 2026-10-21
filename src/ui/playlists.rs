//! The user's lists: Favorite songs, which is always there, and the playlists they make. A heart
//! opens the Save panel, which puts its song in any of them or in a new one. The Library page shows
//! the lists (`library.rs`); each list can be played, edited, reordered by dragging, and deleted,
//! but Favorite songs stays.

use super::drag::{DRAG_START, Edge, landing, slot_at};
use super::messages::CoverChoice;
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

/// How often a list scrolls on while a dragged row is held at its edge: about 30 times a second,
/// so the list glides.
const DRAG_SCROLL_EVERY: std::time::Duration = std::time::Duration::from_millis(33);

/// The room the heart of a song row takes.
pub(super) const HEART_W: f32 = 44.0;

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

    /// The heart at the end of a song row, filled when the song is in one of the user's lists. A
    /// press sends `message`: the Save panel, or what a row being dragged turns it into.
    pub(super) fn row_heart<'a>(
        &self,
        track: &Track,
        color: iced::Color,
        current: bool,
        cover: f32,
        message: Message,
    ) -> Element<'a, Message> {
        let saved = self.is_saved(&track.video_id);
        container(
            button(icon(
                if saved { "like-filled" } else { "like" },
                18.0,
                color,
            ))
            .on_press(message)
            .padding(self.row_icon_padding(cover))
            .style(theme::row_icon_button(current, false)),
        )
        .width(HEART_W)
        .center_x(HEART_W)
        .into()
    }

    /// The padding of a row's ••• and heart. On a phone they are 44 px wide, and as tall as the
    /// row's cover lets them be, up to 44, so a finger finds them.
    pub(super) fn row_icon_padding(&self, cover: f32) -> iced::Padding {
        if self.mobile() {
            let vertical = ((cover - 18.0) / 2.0).clamp(6.0, 13.0);
            iced::Padding {
                top: vertical,
                bottom: vertical,
                left: 13.0,
                right: 13.0,
            }
        } else {
            iced::Padding::new(6.0)
        }
    }

    /// The room a row's ••• button takes.
    pub(super) fn row_more_width(&self) -> f32 {
        if self.mobile() { 44.0 } else { 30.0 }
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
        match (&playlist.cover_file, &playlist.cover) {
            // A picture from the computer is drawn from its file.
            (Some(_), _) => Vec::new(),
            (None, Some(cover)) => vec![cover.as_str()],
            (None, None) => automatic_cover(&playlist.tracks),
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
        if let ListId::Playlist(id) = list
            && let Some(name) = self
                .playlist(id)
                .and_then(|playlist| playlist.cover_file.as_deref())
        {
            return file_cover(name, size);
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
                    cover: CoverChoice::Automatic,
                    picking: false,
                    cover_failed: false,
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
                    cover: match (&playlist.cover_file, &playlist.cover) {
                        (Some(name), _) => CoverChoice::File(name.clone()),
                        (None, Some(url)) => CoverChoice::Song(url.clone()),
                        (None, None) => CoverChoice::Automatic,
                    },
                    picking: false,
                    cover_failed: false,
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
            Message::PlaylistCoverChosen(cover) => self.choose_cover(cover),
            Message::ChooseCoverFile => {
                if let Some(editor) = &mut self.playlist_editor
                    && matches!(editor.target, PlaylistEditTarget::Edit(_))
                    && !editor.picking
                {
                    editor.picking = true;
                    editor.cover_failed = false;
                    let title = self.tr_static("Choose a cover");
                    return Task::perform(
                        crate::file_picker::pick_image(title),
                        Message::CoverFilePicked,
                    );
                }
            }
            Message::CoverFilePicked(picked) => {
                let Some(editor) = self.editing_cover() else {
                    return Task::none();
                };
                editor.picking = false;
                match picked {
                    Ok(Some(path)) => {
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    crate::playlist_covers::import(&path)
                                })
                                .await
                                .map_err(|error| error.to_string())?
                            },
                            Message::CoverFileImported,
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "the Open dialog for a cover failed");
                        editor.cover_failed = true;
                    }
                }
            }
            Message::CoverFileImported(imported) => match imported {
                Ok(name) => {
                    if self.editing_cover().is_some() {
                        self.choose_cover(CoverChoice::File(name));
                    } else {
                        // The fields closed while the picture was read: nothing will name it.
                        crate::playlist_covers::remove(&name);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "a picture could not be made a cover");
                    if let Some(editor) = self.editing_cover() {
                        editor.cover_failed = true;
                    }
                }
            },
            Message::SubmitPlaylistEditor => return self.submit_playlist_editor(),
            Message::CancelPlaylistEditor => {
                if let Some(editor) = self.playlist_editor.take() {
                    self.discard_unsaved_cover(&editor.cover);
                }
            }
            Message::RemoveFromList { list, index } => {
                let Some(track) = self
                    .list_songs(list)
                    .and_then(|songs| songs.get(index))
                    .cloned()
                else {
                    return Task::none();
                };
                self.with_list_songs(list, |songs| {
                    songs.remove(index);
                });
                self.row_menu = None;
                self.row_cursor = None;
                self.persist();
                let name = match list {
                    ListId::Favorites => String::new(),
                    ListId::Playlist(id) => self
                        .playlist(id)
                        .map(|playlist| playlist.name.clone())
                        .unwrap_or_default(),
                };
                return self.show_toast(
                    super::toast::Note::RemovedFrom { list, name },
                    Some(super::toast::ToastAction::RestoreListSong {
                        list,
                        index,
                        track: Box::new(track),
                    }),
                );
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
                    self.row_menu = None;
                    self.delete_prompt = Some(id);
                }
            }
            Message::DeletePlaylist(id) => return self.delete_playlist(id),
            Message::CancelDeletePlaylist => self.delete_prompt = None,
            Message::CloseOverlay => {
                self.close_save_dialog();
                self.delete_prompt = None;
                self.row_menu = None;
            }
            Message::DragPressed { list, index } => {
                self.row_drag = Some(RowDrag {
                    list,
                    from: index,
                    to: index,
                    moved: false,
                    pointer: None,
                    start: None,
                });
                self.drag_edge = Edge::None;
            }
            Message::DragHeld { list, index } => {
                // A finger lifts the row at once; the row follows it from its first move.
                self.row_drag = Some(RowDrag {
                    list,
                    from: index,
                    to: index,
                    moved: true,
                    pointer: None,
                    start: None,
                });
                self.drag_edge = Edge::None;
                self.row_menu = None;
            }
            Message::DragMoved(point, height) => return self.drag_moved(point, height),
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

    /// The fields of a playlist being edited, which a cover can be chosen for.
    fn editing_cover(&mut self) -> Option<&mut PlaylistEditor> {
        self.playlist_editor
            .as_mut()
            .filter(|editor| matches!(editor.target, PlaylistEditTarget::Edit(_)))
    }

    /// Chooses a cover in the playlist's fields. A picture from the computer chosen there before,
    /// and not saved, is removed.
    fn choose_cover(&mut self, cover: CoverChoice) {
        let Some(editor) = self.editing_cover() else {
            return;
        };
        let before = std::mem::replace(&mut editor.cover, cover);
        editor.cover_failed = false;
        if editor.cover != before {
            self.discard_unsaved_cover(&before);
        }
    }

    /// Removes a picture from the computer that was chosen in the fields but is not the cover the
    /// playlist keeps.
    fn discard_unsaved_cover(&self, cover: &CoverChoice) {
        let CoverChoice::File(name) = cover else {
            return;
        };
        let kept = self
            .playlists
            .iter()
            .any(|playlist| playlist.cover_file.as_ref() == Some(name));
        if !kept {
            crate::playlist_covers::remove(name);
        }
    }

    /// Forgets covers from the computer whose file is gone, as after an import from another
    /// computer, and removes the files no playlist names.
    pub(super) fn tidy_playlist_covers(&mut self) {
        let missing: Vec<usize> = self
            .playlists
            .iter()
            .enumerate()
            .filter(|(_, playlist)| {
                playlist
                    .cover_file
                    .as_deref()
                    .is_some_and(|name| !crate::playlist_covers::exists(name))
            })
            .map(|(position, _)| position)
            .collect();
        if !missing.is_empty() {
            let playlists = Arc::make_mut(&mut self.playlists);
            for position in missing {
                playlists[position].cover_file = None;
            }
        }
        let kept: HashSet<&str> = self
            .playlists
            .iter()
            .filter_map(|playlist| playlist.cover_file.as_deref())
            .collect();
        crate::playlist_covers::remove_unused(&kept);
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

    /// Makes a playlist named `name` of `tracks`, with `cover` as its cover: a playlist or album of
    /// YouTube Music, or the queue. A song in it twice goes in once, and what the limits leave no
    /// room for is left out; the note says how much went in, with a button that opens it.
    pub(super) fn save_as_playlist(
        &mut self,
        name: &str,
        tracks: &[Track],
        cover: Option<String>,
    ) -> Task<Message> {
        use super::toast::{Note, ToastAction};
        if self.playlists.len() >= MAX_PLAYLISTS {
            return self.show_toast(Note::Limit(PlaylistNotice::TooManyPlaylists), None);
        }
        let total: usize = self
            .playlists
            .iter()
            .map(|playlist| playlist.tracks.len())
            .sum();
        let room = MAX_PLAYLIST_TRACKS.min(MAX_PLAYLIST_TRACKS_TOTAL.saturating_sub(total));
        if room == 0 {
            return self.show_toast(Note::Limit(PlaylistNotice::AllFull), None);
        }
        let mut seen = HashSet::new();
        let unique: Vec<&Track> = tracks
            .iter()
            .filter(|track| seen.insert(track.video_id.as_str()))
            .collect();
        let of = unique.len();
        let songs: Vec<Track> = unique
            .into_iter()
            .take(room)
            .map(|track| {
                let mut track = track.clone();
                if track.duration_secs.is_none() {
                    track.duration_secs = self.stored_duration_secs(&track.video_id);
                }
                track
            })
            .collect();
        let saved = songs.len();
        let name =
            clean_playlist_name(name).unwrap_or_else(|| self.tr("New playlist").into_owned());
        let id = next_playlist_id(&self.playlists);
        Arc::make_mut(&mut self.playlists).push(Playlist {
            id,
            name: name.clone(),
            tracks: songs,
            cover: cover.filter(|url| url.len() <= crate::preferences::MAX_COVER_URL_BYTES),
            cover_file: None,
        });
        self.persist();
        self.show_toast(
            Note::PlaylistSaved { name, saved, of },
            Some(ToastAction::Open(ListId::Playlist(id))),
        )
    }

    /// Puts a song taken out of a list back at its place: Undo on the note about it. A list that
    /// is gone, has the song again, or is full meanwhile is left as it is.
    pub(super) fn restore_list_song(&mut self, list: ListId, index: usize, track: Track) {
        let Some(count) = self.list_songs(list).map(<[Track]>::len) else {
            return;
        };
        if self.list_contains(list, &track.video_id) {
            return;
        }
        let total: usize = self
            .playlists
            .iter()
            .map(|playlist| playlist.tracks.len())
            .sum();
        let full = match list {
            ListId::Favorites => count >= MAX_LOCAL_FAVORITES,
            ListId::Playlist(_) => {
                count >= MAX_PLAYLIST_TRACKS || total >= MAX_PLAYLIST_TRACKS_TOTAL
            }
        };
        if full {
            return;
        }
        self.with_list_songs(list, |songs| songs.insert(index.min(songs.len()), track));
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
                    let replaced = playlist.cover_file.take();
                    (playlist.cover, playlist.cover_file) = match editor.cover {
                        CoverChoice::Automatic => (None, None),
                        CoverChoice::Song(url) => (Some(url), None),
                        CoverChoice::File(name) => (None, Some(name)),
                    };
                    // The picture the playlist had before goes when nothing names it any more.
                    if let Some(replaced) = replaced
                        && playlist.cover_file.as_ref() != Some(&replaced)
                    {
                        crate::playlist_covers::remove(&replaced);
                    }
                    self.persist();
                } else {
                    self.discard_unsaved_cover(&editor.cover);
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
                    cover_file: None,
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

    /// Delete in the panel that asked. The pages of the playlist are dropped from Back and Forward,
    /// and its own page gives way to the Library.
    fn delete_playlist(&mut self, id: u64) -> Task<Message> {
        if self.delete_prompt.take() != Some(id) {
            return Task::none();
        }
        let Some(position) = self.playlist_position(id) else {
            return Task::none();
        };
        let removed = Arc::make_mut(&mut self.playlists).remove(position);
        if let Some(name) = &removed.cover_file {
            crate::playlist_covers::remove(name);
        }
        if let Some(editor) = self.playlist_editor.as_ref()
            && editor.target == PlaylistEditTarget::Edit(id)
        {
            self.discard_unsaved_cover(&editor.cover.clone());
        }
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
            DragList::Queue => self.queue.len(),
        }
    }

    fn next_drag_tick(&self) -> Task<Message> {
        let tick = self.drag_tick;
        Task::perform(
            async move { tokio::time::sleep(DRAG_SCROLL_EVERY).await },
            move |()| Message::DragTick(tick),
        )
    }

    /// The scroll id, row pitch, offset and fixed leading rows of a list with draggable rows.
    fn drag_geometry(&self, list: DragList) -> (&'static str, f32, f32, usize) {
        match list {
            DragList::Songs(_) => (
                "library-content",
                super::library::song_row_pitch(self.preferences.ui_density),
                self.library_scroll.y,
                0,
            ),
            // Favorite songs is the first row and stays there.
            DragList::Lists => (
                "library-lists",
                super::library::LIST_ROW_PITCH,
                self.lists_scroll.y,
                1,
            ),
            DragList::Queue => (
                "queue-content",
                self.preferences.ui_density.queue_row_pitch(),
                self.queue_scroll.y,
                0,
            ),
        }
    }

    /// The insertion line the held row would go to now, before row `slot` of its list.
    pub(super) fn drag_slot(&self) -> Option<usize> {
        let drag = self.row_drag.filter(|drag| drag.moved)?;
        let pointer = drag.pointer?;
        let (_, pitch, offset, lead) = self.drag_geometry(drag.list);
        Some(slot_at(
            pointer.y + offset,
            pitch,
            lead,
            self.drag_list_count(drag.list),
        ))
    }

    /// Works out where the held row lands from the pointer and the list's scroll.
    fn aim_drag(&mut self) {
        let slot = self.drag_slot();
        let count = self.drag_count();
        if let (Some(drag), Some(slot)) = (&mut self.row_drag, slot) {
            drag.to = landing(drag.from, slot, count);
        }
    }

    /// The pointer moved while a row is held: past a few pixels the row is dragged, and at an edge
    /// of the list the list starts scrolling.
    fn drag_moved(&mut self, point: iced::Point, height: f32) -> Task<Message> {
        self.drag_list_height = height;
        let Some(drag) = &mut self.row_drag else {
            return Task::none();
        };
        drag.pointer = Some(point);
        let start = *drag.start.get_or_insert(point);
        if !drag.moved {
            if (point.y - start.y).abs().max((point.x - start.x).abs()) < DRAG_START {
                return Task::none();
            }
            drag.moved = true;
            self.row_menu = None;
        }
        self.aim_drag();
        let edge = Edge::of(point.y, height);
        let was_still = self.drag_edge == Edge::None;
        self.drag_edge = edge;
        match (edge, was_still) {
            (Edge::None, _) => {
                self.drag_tick = self.drag_tick.wrapping_add(1);
                Task::none()
            }
            // Ticks already run; they read the depth as it is now.
            (_, false) => Task::none(),
            (_, true) => {
                self.drag_tick = self.drag_tick.wrapping_add(1);
                self.next_drag_tick()
            }
        }
    }

    /// Scrolls the held row's list a little toward the edge the pointer is at, and aims the row
    /// again, as the rows move under the pointer.
    fn drag_scroll(&mut self) -> Task<Message> {
        let Some(drag) = self.row_drag.filter(|drag| drag.moved) else {
            return Task::none();
        };
        let step = self.drag_edge.step();
        if step == 0.0 {
            return Task::none();
        }
        let count = self.drag_count();
        let (id, pitch, offset, lead) = self.drag_geometry(drag.list);
        let furthest = ((count + lead) as f32 * pitch - self.drag_list_height).max(0.0);
        let y = (offset + step).clamp(0.0, furthest);
        if (y - offset).abs() < 0.5 {
            return Task::none();
        }
        let scrolled = AbsoluteOffset { x: 0.0, y };
        match drag.list {
            DragList::Songs(_) => self.library_scroll = scrolled,
            DragList::Lists => self.lists_scroll = scrolled,
            DragList::Queue => self.queue_scroll = scrolled,
        }
        self.aim_drag();
        Task::batch([
            operation::scroll_to(id, scrolled),
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
        if !drag.moved || drag.from == drag.to || drag.from >= count {
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
            DragList::Queue => self.move_in_queue(from, to),
        }
        self.row_menu = None;
        self.row_cursor = None;
        self.persist();
        Task::none()
    }

    /// The row of `list` that is being dragged, which shows faded where it was.
    pub(super) fn dragged_row(&self, list: DragList) -> Option<usize> {
        self.row_drag
            .filter(|drag| drag.list == list && drag.moved)
            .map(|drag| drag.from)
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

    /// The pointer reports of a list's area, only while one of its rows is held.
    pub(super) fn drag_moves(&self, list: DragList) -> Option<fn(iced::Point, f32) -> Message> {
        self.row_drag
            .is_some_and(|drag| drag.list == list)
            .then_some(Message::DragMoved as fn(iced::Point, f32) -> Message)
    }

    /// What shows over a list while one of its rows is dragged: a line where it would land, and a
    /// label with its name that follows the pointer.
    pub(super) fn drag_overlay<'a>(
        &self,
        list: DragList,
        width: f32,
    ) -> Option<Element<'a, Message>> {
        let drag = self
            .row_drag
            .filter(|drag| drag.list == list && drag.moved)?;
        let pointer = drag.pointer?;
        let slot = self.drag_slot()?;
        let (_, pitch, offset, lead) = self.drag_geometry(list);
        let name = match list {
            DragList::Songs(songs) => self
                .list_songs(songs)
                .and_then(|tracks| tracks.get(drag.from))
                .map(|track| display_title(&track.title).to_owned()),
            DragList::Lists => self
                .playlists
                .get(drag.from)
                .map(|playlist| playlist.name.clone()),
            DragList::Queue => self
                .queue
                .get(drag.from)
                .map(|track| display_title(&track.title).to_owned()),
        }?;
        let height = self.drag_list_height;
        let mut layers = iced::widget::stack![];
        // The line is shown only where the row would move to.
        let line_y = (slot + lead) as f32 * pitch - offset;
        if drag.to != drag.from && (0.0..=height).contains(&line_y) {
            layers = layers.push(column![
                Space::new().height((line_y - 1.0).max(0.0)),
                container(Space::new().height(2))
                    .width(Length::Fill)
                    .style(|_: &iced::Theme| container::Style {
                        background: Some(iced::Background::Color(INK)),
                        ..Default::default()
                    }),
            ]);
        }
        let label_width = (width * 0.5).clamp(140.0, 320.0);
        let chip = container(
            row![
                icon("note", 14.0, INK),
                label(fit(&name, label_width - 50.0, TEXT_BODY), TEXT_BODY)
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .width(label_width)
        .style(theme::boxed);
        let left = (pointer.x + 16.0).clamp(0.0, (width - label_width).max(0.0));
        let top = (pointer.y - 18.0).clamp(0.0, (height - 40.0).max(0.0));
        layers = layers.push(column![
            Space::new().height(top),
            row![Space::new().width(left), chip]
        ]);
        Some(layers.into())
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

    /// The Save panel over the window: the song, then each list with a box that is filled when the
    /// song is in it, then New playlist. A press on a list puts the song in or takes it out.
    pub(super) fn save_dialog_view<'a>(&'a self, track: &'a Track) -> Element<'a, Message> {
        let width = self.panel_width(SAVE_PANEL_W);
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
        .max_height((self.window_height - 400.0).clamp(100.0, 420.0));
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

/// A cover the user chose from their computer, drawn from its file.
pub(super) fn file_cover<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let line = theme::hairline();
    let inner: Element<'a, Message> = match crate::playlist_covers::path(name) {
        Some(path) => iced::widget::image(iced::widget::image::Handle::from_path(path))
            .width(size - 2.0 * line)
            .height(size - 2.0 * line)
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => Space::new().into(),
    };
    container(inner)
        .width(size)
        .height(size)
        .padding(line)
        .style(theme::boxed)
        .into()
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

    #[test]
    fn the_queue_is_saved_as_a_playlist_once_per_song_and_the_note_opens_it() {
        use super::super::toast::{Note, ToastAction};
        let mut app = App {
            queue: Arc::new(vec![song("a"), song("b"), song("a"), song("c")]),
            queue_index: Some(1),
            ..App::default()
        };
        drop(app.update(Message::SaveQueue));
        let playlist = app.playlists.last().expect("a playlist");
        let ids: Vec<&str> = playlist
            .tracks
            .iter()
            .map(|t| t.video_id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(playlist.name, "Saved queue");
        let id = playlist.id;
        let toast = app.toast.clone().expect("a note");
        assert_eq!(
            toast.note,
            Note::PlaylistSaved {
                name: "Saved queue".into(),
                saved: 3,
                of: 3
            }
        );
        assert_eq!(toast.action, Some(ToastAction::Open(ListId::Playlist(id))));
        drop(app.update(Message::ToastPressed(toast.count)));
        assert_eq!(app.page, Page::List(ListId::Playlist(id)));
        assert!(
            app.in_playlists.contains("c"),
            "the hearts follow the new playlist"
        );
    }

    #[test]
    fn a_saved_playlist_takes_only_the_songs_the_limits_leave_room_for() {
        use super::super::toast::Note;
        let mut app = App::default();
        let full: Vec<Track> = (0..MAX_PLAYLIST_TRACKS)
            .map(|n| song(&format!("x{n}")))
            .collect();
        let songs: Vec<Track> = (0..MAX_PLAYLIST_TRACKS + 5)
            .map(|n| song(&format!("s{n}")))
            .collect();
        drop(app.save_as_playlist("Big", &songs, None));
        assert_eq!(app.playlists[0].tracks.len(), MAX_PLAYLIST_TRACKS);
        assert!(matches!(
            app.toast.as_ref().map(|toast| &toast.note),
            Some(Note::PlaylistSaved { saved, of, .. })
                if *saved == MAX_PLAYLIST_TRACKS && *of == MAX_PLAYLIST_TRACKS + 5
        ));
        // Once the playlists hold all they can, nothing is made and the note says why.
        while app.playlists.iter().map(|p| p.tracks.len()).sum::<usize>()
            < MAX_PLAYLIST_TRACKS_TOTAL
        {
            drop(app.save_as_playlist("More", &full, None));
        }
        let count = app.playlists.len();
        drop(app.save_as_playlist("One more", &songs, None));
        assert_eq!(app.playlists.len(), count);
        assert_eq!(
            app.toast.as_ref().map(|toast| &toast.note),
            Some(&Note::Limit(PlaylistNotice::AllFull))
        );
    }

    #[test]
    fn the_library_filters_and_sorts_its_playlists_with_favorite_songs_first() {
        use crate::preferences::LibrarySort;
        let mut app = library();
        for name in ["Rock", "ambient", "Road trip"] {
            make(&mut app, name);
            drop(app.update(Message::Navigate(Page::Library)));
        }
        // Rows by place: Favorite songs is 0, then the playlists in the user's order.
        assert_eq!(app.library_rows(), [0, 1, 2, 3]);
        assert!(app.library_reorderable());
        drop(app.update(Message::Preference(Preference::LibrarySort(
            LibrarySort::Name,
        ))));
        assert_eq!(app.library_rows(), [0, 2, 3, 1]);
        assert!(
            !app.library_reorderable(),
            "dragging is for the user's order"
        );
        drop(app.update(Message::Preference(Preference::LibrarySort(
            LibrarySort::Recent,
        ))));
        assert_eq!(app.library_rows(), [0, 3, 2, 1]);
        // The filter ignores case, and leaves Favorite songs out when its name does not match.
        drop(app.update(Message::LibraryFilterChanged("RO".into())));
        assert_eq!(app.library_rows(), [3, 1]);
        drop(app.update(Message::LibraryFilterChanged("favorite".into())));
        assert_eq!(app.library_rows(), [0]);
        drop(app.update(Message::LibraryFilterChanged("nothing".into())));
        assert!(app.library_rows().is_empty());
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

    /// Moves the pointer over a list's area, 400 px tall, as the drag area reports it.
    fn point(app: &mut App, y: f32) {
        drop(app.update(Message::DragMoved(iced::Point::new(200.0, y), 400.0)));
    }

    #[test]
    fn a_dragged_song_lands_on_the_line_nearest_the_pointer() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Mix"));
        for id in ["a", "b", "c", "d"] {
            save(&mut app, list, id);
        }
        let songs = DragList::Songs(list);
        let pitch = super::super::library::song_row_pitch(app.preferences.ui_density);
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 0,
        }));
        // A small shake is still a click.
        point(&mut app, 10.0);
        point(&mut app, 13.0);
        assert!(matches!(
            app.row_click(songs, Message::PlayPlaylist { id: 0, index: 0 }),
            Message::PlayPlaylist { .. }
        ));
        assert!(app.drag_overlay(songs, 800.0).is_none());
        // Past a few pixels it is a drag: aimed at the line before the fourth row, it lands third.
        point(&mut app, 3.0 * pitch + 4.0);
        assert!(matches!(
            app.row_click(songs, Message::PlayPlaylist { id: 0, index: 0 }),
            Message::DragReleased
        ));
        assert_eq!(app.drag_slot(), Some(3));
        assert_eq!(app.dragged_row(songs), Some(0));
        assert!(app.drag_overlay(songs, 800.0).is_some());
        drop(app.update(Message::PointerHeld(false)));
        assert_eq!(songs_of(&app, list), ["b", "c", "a", "d"]);
        assert!(app.row_drag.is_none());
        // Let go on a line next to where it started, nothing moves.
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 1,
        }));
        point(&mut app, 1.5 * pitch);
        point(&mut app, 3.5 * pitch);
        point(&mut app, 2.0 * pitch);
        drop(app.update(Message::DragReleased));
        assert_eq!(songs_of(&app, list), ["b", "c", "a", "d"]);
        // Escape lets go of a drag without moving the row.
        drop(app.update(Message::DragPressed {
            list: songs,
            index: 0,
        }));
        point(&mut app, 10.0);
        point(&mut app, 4.0 * pitch);
        drop(app.escape_outside_search());
        drop(app.update(Message::PointerHeld(false)));
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
        let pitch = super::super::library::LIST_ROW_PITCH;
        drop(app.update(Message::DragPressed {
            list: DragList::Lists,
            index: 2,
        }));
        point(&mut app, 3.5 * pitch);
        // Aimed above Favorite songs, it lands first among the playlists.
        point(&mut app, 0.0);
        assert_eq!(app.drag_slot(), Some(0));
        drop(app.update(Message::PointerHeld(false)));
        let order: Vec<u64> = app.playlists.iter().map(|playlist| playlist.id).collect();
        assert_eq!(order, [ids[2], ids[0], ids[1]]);
    }

    #[test]
    fn a_dragged_queue_entry_moves_and_the_playing_track_stays_current() {
        let mut app = App {
            page: Page::NowPlaying,
            queue: Arc::new(["a", "b", "c", "d", "e"].map(song).to_vec()),
            queue_index: Some(1),
            ..App::default()
        };
        let pitch = app.preferences.ui_density.queue_row_pitch();
        let order =
            |app: &App| -> Vec<String> { app.queue.iter().map(|t| t.video_id.clone()).collect() };
        // The track that plays, "b", goes to the end.
        drop(app.update(Message::DragPressed {
            list: DragList::Queue,
            index: 1,
        }));
        point(&mut app, 1.5 * pitch);
        point(&mut app, 5.0 * pitch);
        drop(app.update(Message::PointerHeld(false)));
        assert_eq!(order(&app), ["a", "c", "d", "e", "b"]);
        assert_eq!(app.queue_index, Some(4));
        // "e" moves above "a": the track that plays is pushed one place down.
        drop(app.update(Message::DragPressed {
            list: DragList::Queue,
            index: 3,
        }));
        point(&mut app, 3.5 * pitch);
        point(&mut app, 0.0);
        drop(app.update(Message::DragReleased));
        assert_eq!(order(&app), ["e", "a", "c", "d", "b"]);
        assert_eq!(app.queue_index, Some(4));
        // Move up and down from a ••• menu keep it too.
        drop(app.update(Message::MoveInQueue { index: 4, up: true }));
        assert_eq!(order(&app), ["e", "a", "c", "b", "d"]);
        assert_eq!(app.queue_index, Some(3));
        drop(app.update(Message::MoveInQueue { index: 0, up: true }));
        assert_eq!(order(&app), ["e", "a", "c", "b", "d"]);
    }

    #[test]
    fn a_row_held_by_a_finger_is_lifted_at_once_and_moves_with_it() {
        let mut app = App {
            page: Page::NowPlaying,
            queue: Arc::new(["a", "b", "c", "d"].map(song).to_vec()),
            queue_index: Some(0),
            ..App::default()
        };
        let pitch = app.preferences.ui_density.queue_row_pitch();
        let order =
            |app: &App| -> Vec<String> { app.queue.iter().map(|t| t.video_id.clone()).collect() };
        // Held and let go without moving: nothing moves, and the row is not played.
        drop(app.update(Message::DragHeld {
            list: DragList::Queue,
            index: 2,
        }));
        assert!(matches!(
            app.row_click(DragList::Queue, Message::PlayQueueIndex(2)),
            Message::DragReleased
        ));
        drop(app.update(Message::PointerHeld(false)));
        assert_eq!(order(&app), ["a", "b", "c", "d"]);
        // Held, then moved: the first move already aims the row, with no threshold to pass.
        drop(app.update(Message::DragHeld {
            list: DragList::Queue,
            index: 2,
        }));
        point(&mut app, 0.3 * pitch);
        assert_eq!(app.drag_slot(), Some(0));
        drop(app.update(Message::PointerHeld(false)));
        assert_eq!(order(&app), ["c", "a", "b", "d"]);
        assert_eq!(app.queue_index, Some(1));
    }

    #[test]
    fn a_drag_at_the_edge_scrolls_the_list_and_aims_again() {
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
        point(&mut app, 200.0);
        let before = app.drag_slot();
        point(&mut app, 395.0);
        let tick = app.drag_tick;
        for _ in 0..5 {
            drop(app.update(Message::DragTick(app.drag_tick)));
        }
        assert!(app.library_scroll.y > 0.0);
        assert!(app.drag_slot() > before);
        // A tick from before the pointer left the edge does nothing.
        point(&mut app, 200.0);
        let scrolled = app.library_scroll.y;
        drop(app.update(Message::DragTick(tick)));
        assert_eq!(app.library_scroll.y, scrolled);
    }

    #[test]
    fn a_playlist_is_deleted_from_its_panel_and_its_page_goes() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        assert_eq!(app.page, Page::List(ListId::Playlist(id)));
        // Delete without the panel's question does nothing.
        drop(app.update(Message::DeletePlaylist(id)));
        assert_eq!(app.playlists.len(), 1);
        drop(app.update(Message::AskDeletePlaylist(id)));
        assert_eq!(app.delete_prompt, Some(id));
        assert!(app.overlay_panel().is_some());
        drop(app.update(Message::CancelDeletePlaylist));
        assert!(app.delete_prompt.is_none());
        drop(app.update(Message::DeletePlaylist(id)));
        assert_eq!(app.playlists.len(), 1);
        // A press outside the panel closes it too.
        drop(app.update(Message::AskDeletePlaylist(id)));
        drop(app.update(Message::CloseOverlay));
        assert!(app.delete_prompt.is_none());
        drop(app.update(Message::AskDeletePlaylist(id)));
        drop(app.update(Message::DeletePlaylist(id)));
        assert!(app.playlists.is_empty());
        assert!(app.delete_prompt.is_none());
        assert_eq!(app.page, Page::Library);
        assert!(
            !app.history
                .iter()
                .chain(&app.forward)
                .any(|page| *page == Page::List(ListId::Playlist(id)))
        );
    }

    #[test]
    fn a_menu_opens_as_a_panel_for_its_row_and_closes_when_the_row_goes() {
        let mut app = library();
        let list = ListId::Playlist(make(&mut app, "Mix"));
        for id in ["a", "b"] {
            save(&mut app, list, id);
        }
        drop(app.update(Message::ToggleRowMenu(RowMenu {
            list: "playlist",
            index: 1,
        })));
        assert!(app.overlay_panel().is_some());
        drop(app.update(Message::CloseOverlay));
        assert!(app.row_menu.is_none());
        assert!(app.overlay_panel().is_none());
        // A menu for a row that is not there shows nothing.
        app.row_menu = Some(RowMenu {
            list: "playlist",
            index: 9,
        });
        assert!(app.overlay_panel().is_none());
        // The Library's rows: Favorite songs has no menu, a playlist has one.
        drop(app.update(Message::Navigate(Page::Library)));
        app.row_menu = Some(RowMenu {
            list: "lists",
            index: 1,
        });
        assert!(app.overlay_panel().is_some());
        app.row_menu = Some(RowMenu {
            list: "lists",
            index: 0,
        });
        assert!(app.overlay_panel().is_none());
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
        drop(app.update(Message::PlaylistCoverChosen(CoverChoice::Song(
            "https://covers.test/b.jpg".into(),
        ))));
        drop(app.update(Message::SubmitPlaylistEditor));
        let playlist = app.playlist(id).unwrap();
        assert_eq!(playlist.name, "Night drive");
        assert_eq!(songs_of(&app, list), ["a", "b"]);
        assert_eq!(app.list_cover_urls(list), ["https://covers.test/b.jpg"]);
        // Cancel leaves both as they were.
        drop(app.update(Message::EditPlaylist(id)));
        drop(app.update(Message::PlaylistCoverChosen(CoverChoice::Automatic)));
        drop(app.update(Message::CancelPlaylistEditor));
        assert_eq!(app.list_cover_urls(list), ["https://covers.test/b.jpg"]);
    }

    /// A picture from the computer, made a cover as the Open dialog's answer would be.
    fn imported_cover(app: &mut App, width: u32) -> String {
        let dir = std::env::temp_dir().join(format!("crabytube-ui-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join(format!("{width}.png"));
        ::image::RgbImage::from_pixel(width, width, ::image::Rgb([10, 20, 30]))
            .save(&source)
            .unwrap();
        let name = crate::playlist_covers::import(&source).unwrap();
        drop(app.update(Message::CoverFileImported(Ok(name.clone()))));
        name
    }

    #[test]
    fn a_picture_from_the_computer_becomes_the_cover_and_goes_with_it() {
        let _folder = crate::playlist_covers::lock_test_folder();
        let mut app = library();
        let id = make(&mut app, "Mix");
        drop(app.update(Message::EditPlaylist(id)));
        let first = imported_cover(&mut app, 40);
        assert_eq!(
            app.playlist_editor.as_ref().map(|editor| &editor.cover),
            Some(&CoverChoice::File(first.clone()))
        );
        // Another picture before saving: the first is not kept.
        let second = imported_cover(&mut app, 50);
        assert!(!crate::playlist_covers::exists(&first));
        drop(app.update(Message::SubmitPlaylistEditor));
        assert_eq!(
            app.playlist(id).unwrap().cover_file.as_deref(),
            Some(second.as_str())
        );
        assert!(app.list_cover_urls(ListId::Playlist(id)).is_empty());
        // Back to the automatic cover: the picture goes.
        drop(app.update(Message::EditPlaylist(id)));
        assert_eq!(
            app.playlist_editor.as_ref().map(|editor| &editor.cover),
            Some(&CoverChoice::File(second.clone()))
        );
        drop(app.update(Message::PlaylistCoverChosen(CoverChoice::Automatic)));
        // Not until it is saved: Cancel keeps it.
        drop(app.update(Message::CancelPlaylistEditor));
        assert!(crate::playlist_covers::exists(&second));
        drop(app.update(Message::EditPlaylist(id)));
        drop(app.update(Message::PlaylistCoverChosen(CoverChoice::Automatic)));
        drop(app.update(Message::SubmitPlaylistEditor));
        assert!(app.playlist(id).unwrap().cover_file.is_none());
        assert!(!crate::playlist_covers::exists(&second));
    }

    #[test]
    fn a_picture_chosen_and_cancelled_or_deleted_with_its_playlist_is_removed() {
        let _folder = crate::playlist_covers::lock_test_folder();
        let mut app = library();
        let id = make(&mut app, "Mix");
        drop(app.update(Message::EditPlaylist(id)));
        let unsaved = imported_cover(&mut app, 30);
        drop(app.update(Message::CancelPlaylistEditor));
        assert!(!crate::playlist_covers::exists(&unsaved));
        // A picture that arrives after the fields closed is not kept either.
        let late = imported_cover(&mut app, 31);
        assert!(!crate::playlist_covers::exists(&late));
        drop(app.update(Message::EditPlaylist(id)));
        let kept = imported_cover(&mut app, 32);
        drop(app.update(Message::SubmitPlaylistEditor));
        drop(app.update(Message::AskDeletePlaylist(id)));
        drop(app.update(Message::DeletePlaylist(id)));
        assert!(!crate::playlist_covers::exists(&kept));
    }

    #[test]
    fn a_cover_file_that_is_gone_is_forgotten() {
        let _folder = crate::playlist_covers::lock_test_folder();
        let mut app = library();
        let id = make(&mut app, "Mix");
        Arc::make_mut(&mut app.playlists)[0].cover_file = Some("00000000deadbeef.jpg".into());
        app.tidy_playlist_covers();
        assert!(app.playlist(id).unwrap().cover_file.is_none());
    }

    #[test]
    fn a_failed_open_dialog_says_so_and_a_cancelled_one_changes_nothing() {
        let mut app = library();
        let id = make(&mut app, "Mix");
        drop(app.update(Message::EditPlaylist(id)));
        drop(app.update(Message::CoverFilePicked(Ok(None))));
        let editor = app.playlist_editor.as_ref().unwrap();
        assert!(!editor.cover_failed && editor.cover == CoverChoice::Automatic);
        drop(app.update(Message::CoverFilePicked(Err("no portal".into()))));
        assert!(app.playlist_editor.as_ref().unwrap().cover_failed);
        drop(app.update(Message::CoverFileImported(Err("not a picture".into()))));
        assert!(app.playlist_editor.as_ref().unwrap().cover_failed);
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
