//! The panels over the window: the actions of a ••• menu and the question before a playlist is
//! deleted (Save to is in `playlists.rs`). Each is a card in the middle of the dimmed window, the
//! same at a phone's width as on a wide window.

use super::messages::{ListId, RowMenu};
use super::shell::{display_title, fit};
use super::theme::{
    self, DIM, FADED, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_SECTION, hrule, icon, label,
};
use super::{App, Message};
use crate::music::Track;
use crate::preferences::RecentPlay;
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

/// The widest a panel's card gets, and the room kept around it on a narrow window.
const PANEL_W: f32 = 420.0;
const PANEL_MARGIN: f32 = 16.0;
/// The cover at the top of a ••• panel.
const SHEET_COVER: f32 = 48.0;

/// The list an opened row is in, for the actions that only it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowContext {
    Plain,
    /// A row of Up Next, at this place in the queue.
    Queue(usize),
    /// A song of one of the user's lists, at `index` of its `count`.
    List {
        list: ListId,
        index: usize,
        count: usize,
    },
}

/// What a ••• menu is about: a song, or a playlist on the Library page by its place.
enum SheetTarget<'a> {
    Song(&'a Track, RowContext),
    Playlist(usize),
}

/// One action of a ••• panel: its icon, its label and what it does. Without a message it is off.
type Action = (&'static str, &'static str, Option<Message>);

impl App {
    /// The width of a panel's card on this window.
    pub(super) fn panel_width(&self, widest: f32) -> f32 {
        widest
            .min(self.window_width - 2.0 * PANEL_MARGIN)
            .max(200.0)
    }

    /// The panel over the window, if one is open: Save to, the question to delete, or the actions
    /// of the open ••• menu.
    pub(super) fn overlay_panel(&self) -> Option<Element<'_, Message>> {
        if let Some(track) = &self.save_dialog {
            return Some(self.save_dialog_view(track));
        }
        if let Some(id) = self.delete_prompt {
            return self.delete_prompt_view(id);
        }
        let menu = self.row_menu.as_ref()?;
        match self.sheet_target(menu)? {
            SheetTarget::Song(track, context) => Some(self.song_sheet(track, context)),
            SheetTarget::Playlist(place) => self.playlist_sheet(place),
        }
    }

    /// The song or playlist an open ••• menu belongs to, while its row is still there.
    fn sheet_target(&self, menu: &RowMenu) -> Option<SheetTarget<'_>> {
        let index = menu.index;
        Some(match menu.list {
            "songs" => SheetTarget::Song(
                self.search_songs().get(index).copied()?.track.as_ref()?,
                RowContext::Plain,
            ),
            "live" => SheetTarget::Song(
                self.search_live().get(index).copied()?.track.as_ref()?,
                RowContext::Plain,
            ),
            "jump" => match self.recent.get(index)? {
                RecentPlay::Track(track) => SheetTarget::Song(track, RowContext::Plain),
                RecentPlay::Collection(_) => return None,
            },
            "collection" => SheetTarget::Song(
                self.collection.tracks_context.get(index)?,
                RowContext::Plain,
            ),
            "queue" => {
                let track = self.queue.get(index)?;
                // The track that plays stays in the queue.
                let context = if self.queue_index == Some(index) {
                    RowContext::Plain
                } else {
                    RowContext::Queue(index)
                };
                SheetTarget::Song(track, context)
            }
            "favorites" | "playlist" => {
                let list = self.shown_list()?;
                if App::song_rows(list) != menu.list {
                    return None;
                }
                let songs = self.list_songs(list)?;
                SheetTarget::Song(
                    songs.get(index)?,
                    RowContext::List {
                        list,
                        index,
                        count: songs.len(),
                    },
                )
            }
            "lists" => {
                let place = index.checked_sub(1)?;
                (place < self.playlists.len()).then_some(SheetTarget::Playlist(place))?
            }
            _ => return None,
        })
    }

    /// The card of a panel: its title, what it is about, a rule, then its content.
    fn panel_card<'a>(
        &self,
        head: Element<'a, Message>,
        content: Element<'a, Message>,
        width: f32,
    ) -> Element<'a, Message> {
        let narrow = width < 360.0;
        container(column![head, hrule(RULE), content].spacing(14))
            .width(width)
            .padding(if narrow { 16 } else { 24 })
            .style(theme::boxed)
            .into()
    }

    /// The top of a ••• panel: a cover, two lines about what it is for, and a button that closes it.
    fn sheet_head<'a>(
        &self,
        cover: Element<'a, Message>,
        title: &str,
        subtitle: &str,
        width: f32,
    ) -> Element<'a, Message> {
        let text_width = width - SHEET_COVER - 16.0 - 40.0 - 48.0;
        row![
            cover,
            column![
                label(fit(title, text_width, TEXT_BODY), TEXT_BODY),
                label(fit(subtitle, text_width, TEXT_CAPTION), TEXT_CAPTION).color(DIM),
            ]
            .spacing(4)
            .width(Length::Fill),
            button(icon("close", 14.0, INK))
                .on_press(Message::CloseOverlay)
                .padding(6)
                .style(theme::icon_button),
        ]
        .spacing(16)
        .align_y(Alignment::Center)
        .into()
    }

    /// The actions of a ••• panel, one under the other, each with its icon.
    fn sheet_actions<'a>(&self, actions: Vec<Action>) -> Element<'a, Message> {
        let mut list = column![].spacing(2);
        for (glyph, title, message) in actions {
            let ink = if message.is_some() { INK } else { FADED };
            list = list.push(
                button(
                    row![
                        icon(glyph, 16.0, ink),
                        text(self.tr(title).into_owned())
                            .size(TEXT_BODY)
                            .font(MONO)
                            .color(ink)
                    ]
                    .spacing(16)
                    .align_y(Alignment::Center),
                )
                .on_press_maybe(message)
                .width(Length::Fill)
                .padding([10, 12])
                .style(theme::row_button),
            );
        }
        list.into()
    }

    /// The ••• panel of a song: play it next or later, a radio from it, the lists to save it in,
    /// its artist and album, and what its list allows.
    fn song_sheet<'a>(&'a self, track: &'a Track, context: RowContext) -> Element<'a, Message> {
        let width = self.panel_width(PANEL_W);
        let mut actions: Vec<Action> = vec![
            ("next", "Play next", Some(Message::PlayNext(track.clone()))),
            (
                "queue",
                "Add to queue",
                Some(Message::AddToQueue(track.clone())),
            ),
            (
                "radio",
                "Start radio",
                Some(Message::StartRadio(track.clone())),
            ),
            (
                "plus",
                "Add to playlist",
                Some(Message::OpenSaveDialog(track.clone())),
            ),
        ];
        // Only the artist and album the catalog named; a track without them gets no "Go to".
        for (title, page) in super::track_actions::go_to_destinations(track) {
            let glyph = if title == "Go to artist" {
                "artist"
            } else {
                "album"
            };
            actions.push((glyph, title, Some(Message::OpenCollection(page))));
        }
        match context {
            RowContext::Plain => {}
            RowContext::Queue(index) => {
                actions.push((
                    "close",
                    "Remove from queue",
                    Some(Message::RemoveFromQueue(index)),
                ));
            }
            RowContext::List { list, index, count } => {
                actions.push((
                    "arrow-up",
                    "Move up",
                    (index > 0).then_some(Message::MoveInList {
                        list,
                        index,
                        up: true,
                    }),
                ));
                actions.push((
                    "arrow-down",
                    "Move down",
                    (index + 1 < count).then_some(Message::MoveInList {
                        list,
                        index,
                        up: false,
                    }),
                ));
                actions.push((
                    "trash",
                    "Remove from playlist",
                    Some(Message::RemoveFromList { list, index }),
                ));
            }
        }
        let head = self.sheet_head(
            self.cover(
                track.thumbnail_url.as_deref(),
                SHEET_COVER,
                crate::artwork::THUMB_MAX,
            ),
            display_title(&track.title),
            &track.artist,
            width,
        );
        self.panel_card(head, self.sheet_actions(actions), width)
    }

    /// The ••• panel of a playlist on the Library page: play it, move it, delete it.
    fn playlist_sheet(&self, place: usize) -> Option<Element<'_, Message>> {
        let playlist = self.playlists.get(place)?;
        let id = playlist.id;
        let full = !playlist.tracks.is_empty();
        let width = self.panel_width(PANEL_W);
        let actions: Vec<Action> = vec![
            (
                "play",
                "Play all",
                full.then_some(Message::PlayPlaylist { id, index: 0 }),
            ),
            (
                "shuffle",
                "Shuffle",
                full.then_some(Message::ShufflePlaylist(id)),
            ),
            (
                "arrow-up",
                "Move up",
                (place > 0).then_some(Message::MovePlaylist {
                    index: place,
                    up: true,
                }),
            ),
            (
                "arrow-down",
                "Move down",
                (place + 1 < self.playlists.len()).then_some(Message::MovePlaylist {
                    index: place,
                    up: false,
                }),
            ),
            ("trash", "Delete", Some(Message::AskDeletePlaylist(id))),
        ];
        let head = self.sheet_head(
            self.list_cover(ListId::Playlist(id), SHEET_COVER),
            &playlist.name,
            &self.song_count(playlist.tracks.len(), false),
            width,
        );
        Some(self.panel_card(head, self.sheet_actions(actions), width))
    }

    /// The panel that asks before a playlist is deleted, with Cancel and Delete side by side under
    /// the question.
    fn delete_prompt_view(&self, id: u64) -> Option<Element<'_, Message>> {
        let playlist = self.playlist(id)?;
        let width = self.panel_width(380.0);
        let button_width = ((width - 48.0 - 12.0) / 2.0).min(140.0);
        let choice = |title: &'static str, message: Message, primary: bool| {
            button(
                container(label(self.tr(title).into_owned(), TEXT_BODY))
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            )
            .on_press(message)
            .width(button_width)
            .padding([10, 12])
            .style(if primary {
                theme::selected_outline_button
            } else {
                theme::outline_button
            })
        };
        let content = column![
            text(self.tr("Delete this playlist?"))
                .size(TEXT_SECTION)
                .font(MONO),
            label(fit(&playlist.name, width - 48.0, TEXT_BODY), TEXT_BODY).color(DIM),
            Space::new().height(8),
            row![
                choice("Cancel", Message::CancelDeletePlaylist, false),
                choice("Delete", Message::DeletePlaylist(id), true),
            ]
            .spacing(12),
        ]
        .spacing(12)
        .align_x(Alignment::Center);
        Some(
            container(content)
                .width(width)
                .padding(24)
                .style(theme::boxed)
                .into(),
        )
    }
}
