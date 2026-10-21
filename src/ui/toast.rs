//! A short note at the foot of the window about what an action did: a song queued, a radio started,
//! a song taken out of a list, a playlist saved. A note about something that can be taken back has
//! Undo, and one about a new playlist opens it.

use super::shell::fit;
use super::theme::{self, INK, TEXT_BODY, TEXT_CAPTION, label};
use super::*;
use iced::widget::{button, container, row};
use iced::{Alignment, Length};
use unicode_width::UnicodeWidthStr;

/// How long a note stays, and a note with a button, which gives time to press it.
const NOTE_TIME: Duration = Duration::from_secs(4);
const ACTION_TIME: Duration = Duration::from_secs(7);
/// The note of a radio that is loading stays until the radio comes, or this long.
const LOADING_TIME: Duration = Duration::from_secs(30);
/// The widest a note gets, and the room kept around it on a narrow window.
const TOAST_W: f32 = 480.0;
const TOAST_MARGIN: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Toast {
    pub(super) note: Note,
    pub(super) action: Option<ToastAction>,
    /// Numbers the note, so the timer of an older one cannot take a newer one away.
    pub(super) count: u64,
}

/// What a note says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Note {
    AddedToQueue,
    PlaysNext,
    QueueFull,
    /// The radio of this request to "Start radio" is loading.
    RadioLoading(u64),
    RadioStarted,
    RadioFailed,
    RemovedFromQueue,
    UpNextCleared,
    /// A song was taken out of this list, named as it was then.
    RemovedFrom {
        list: ListId,
        name: String,
    },
    /// A playlist was made with this name: `saved` songs went in, of the `of` there were.
    PlaylistSaved {
        name: String,
        saved: usize,
        of: usize,
    },
    /// No playlist was made, for this limit.
    Limit(PlaylistNotice),
}

/// What the note's button does.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ToastAction {
    /// Puts a song taken out of a list back at its place.
    RestoreListSong {
        list: ListId,
        index: usize,
        track: Box<Track>,
    },
    /// Puts songs taken out of the queue back at their place, while the queue has not changed
    /// since.
    RestoreQueue {
        index: usize,
        tracks: Vec<Track>,
        generation: u64,
    },
    /// Opens a list.
    Open(ListId),
}

impl App {
    /// Shows a note in place of the one shown, and sets the timer that takes it away.
    pub(super) fn show_toast(&mut self, note: Note, action: Option<ToastAction>) -> Task<Message> {
        self.toast_count = self.toast_count.wrapping_add(1);
        let count = self.toast_count;
        let time = if matches!(note, Note::RadioLoading(_)) {
            LOADING_TIME
        } else if action.is_some() {
            ACTION_TIME
        } else {
            NOTE_TIME
        };
        self.toast = Some(Toast {
            note,
            action,
            count,
        });
        Task::perform(async move { tokio::time::sleep(time).await }, move |()| {
            Message::ToastExpired(count)
        })
    }

    /// Takes the note away when it says this.
    pub(super) fn hide_toast(&mut self, note: &Note) {
        if self.toast.as_ref().is_some_and(|toast| toast.note == *note) {
            self.toast = None;
        }
    }

    pub(super) fn update_toast(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToastExpired(count) => {
                if self
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.count == count)
                {
                    self.toast = None;
                }
            }
            Message::ToastPressed(count) => {
                let Some(toast) = self.toast.take_if(|toast| toast.count == count) else {
                    return Task::none();
                };
                match toast.action {
                    Some(ToastAction::RestoreListSong { list, index, track }) => {
                        self.restore_list_song(list, index, *track);
                    }
                    Some(ToastAction::RestoreQueue {
                        index,
                        tracks,
                        generation,
                    }) => self.restore_queue(index, tracks, generation),
                    Some(ToastAction::Open(list)) if self.list_songs(list).is_some() => {
                        return self.navigate(Page::List(list));
                    }
                    Some(ToastAction::Open(_)) | None => {}
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// Puts the songs taken out of the queue back where they were. A queue that changed since is
    /// left as it is.
    fn restore_queue(&mut self, index: usize, tracks: Vec<Track>, generation: u64) {
        if generation != self.queue_generation
            || index > self.queue.len()
            || self.queue.len() + tracks.len() > crate::preferences::MAX_QUEUE_TRACKS
        {
            return;
        }
        let count = tracks.len();
        Arc::make_mut(&mut self.queue).splice(index..index, tracks);
        if let Some(current) = self.queue_index
            && index <= current
        {
            self.queue_index = Some(current + count);
        }
        self.queue_source = None;
        self.queue_generation = self.queue_generation.wrapping_add(1);
        self.persist();
    }

    /// The note in the interface language.
    fn toast_text(&self, note: &Note) -> String {
        match note {
            Note::AddedToQueue => self.tr("Added to queue").into_owned(),
            Note::PlaysNext => self.tr("Plays next").into_owned(),
            Note::QueueFull => self.tr("The queue is full.").into_owned(),
            Note::RadioLoading(_) => self.tr("Starting radio…").into_owned(),
            Note::RadioStarted => self.tr("Radio started").into_owned(),
            Note::RadioFailed => self.tr("Couldn't start the radio").into_owned(),
            Note::RemovedFromQueue => self.tr("Removed from queue").into_owned(),
            Note::UpNextCleared => self.tr("Up next cleared").into_owned(),
            Note::RemovedFrom { list, name } => {
                let name = match list {
                    ListId::Favorites => self.tr("Favorite songs").into_owned(),
                    ListId::Playlist(_) => name.clone(),
                };
                self.tr_static("Removed from {}").replacen("{}", &name, 1)
            }
            Note::PlaylistSaved { name, saved, of } if saved < of => self
                .tr_static("Saved {} of {} songs to {}")
                .replacen("{}", &saved.to_string(), 1)
                .replacen("{}", &of.to_string(), 1)
                .replacen("{}", name, 1),
            Note::PlaylistSaved { name, .. } => self
                .tr_static("Playlist created: {}")
                .replacen("{}", name, 1),
            Note::Limit(notice) => self.playlist_notice_text(notice),
        }
    }

    /// The note at the foot of the window, if one is shown: a card above the player, with its
    /// button on the right.
    pub(super) fn toast_view(&self) -> Option<Element<'_, Message>> {
        let toast = self.toast.as_ref()?;
        let width = TOAST_W
            .min(self.window_width - 2.0 * TOAST_MARGIN)
            .max(200.0);
        let button_label = match toast.action {
            Some(ToastAction::RestoreListSong { .. } | ToastAction::RestoreQueue { .. }) => {
                Some(self.tr("Undo").into_owned())
            }
            Some(ToastAction::Open(_)) => Some(self.tr("Open").into_owned()),
            None => None,
        };
        let button_width = button_label.as_deref().map_or(0.0, |title| {
            title.width() as f32 * TEXT_CAPTION * 0.6 + 24.0 + 16.0
        });
        let text_width = width - 2.0 * 16.0 - button_width;
        let mut line = row![
            label(
                fit(&self.toast_text(&toast.note), text_width, TEXT_BODY),
                TEXT_BODY
            )
            .color(INK)
            .width(Length::Fill)
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        if let Some(title) = button_label {
            line = line.push(
                button(label(title, TEXT_CAPTION))
                    .on_press(Message::ToastPressed(toast.count))
                    .padding([6, 12])
                    .style(theme::outline_button),
            );
        }
        Some(
            container(line)
                .width(width)
                .padding([12, 16])
                .style(theme::boxed)
                .into(),
        )
    }

    /// The room under the note: the player bar, when it shows, the tabs of a phone, and a margin.
    pub(super) fn toast_lift(&self) -> f32 {
        let bar = if self.player_bar_shown() {
            theme::PLAYER_H + theme::hairline()
        } else {
            0.0
        };
        let tabs = if self.mobile() {
            super::shell::TAB_BAR_H + theme::hairline()
        } else {
            0.0
        };
        bar + tabs + TOAST_MARGIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            artist_page: None,
            album_page: None,
            thumbnail_url: None,
            duration_secs: None,
        }
    }

    #[test]
    fn a_newer_note_is_not_taken_away_by_the_timer_of_an_older_one() {
        let mut app = App::default();
        drop(app.show_toast(Note::AddedToQueue, None));
        let first = app.toast.as_ref().unwrap().count;
        drop(app.show_toast(Note::PlaysNext, None));
        drop(app.update(Message::ToastExpired(first)));
        assert_eq!(
            app.toast.as_ref().map(|toast| &toast.note),
            Some(&Note::PlaysNext)
        );
        let second = app.toast.as_ref().unwrap().count;
        drop(app.update(Message::ToastExpired(second)));
        assert!(app.toast.is_none());
    }

    #[test]
    fn undo_puts_songs_back_in_the_queue_only_while_it_is_unchanged() {
        let mut app = App {
            queue: Arc::new(vec![track("a"), track("b"), track("c")]),
            queue_index: Some(0),
            ..App::default()
        };
        drop(app.update(Message::RemoveFromQueue(1)));
        assert_eq!(app.queue.len(), 2);
        let count = app.toast.as_ref().unwrap().count;
        drop(app.update(Message::ToastPressed(count)));
        let ids: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert!(app.toast.is_none());

        // The queue changed after the removal: Undo leaves it be.
        drop(app.update(Message::RemoveFromQueue(2)));
        let count = app.toast.as_ref().unwrap().count;
        drop(app.update(Message::RemoveFromQueue(1)));
        drop(app.update(Message::ToastPressed(count)));
        let ids: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, ["a"]);
    }

    #[test]
    fn undo_brings_back_up_next_after_it_was_cleared() {
        let mut app = App {
            queue: Arc::new(vec![track("a"), track("b"), track("c")]),
            queue_index: Some(1),
            ..App::default()
        };
        drop(app.update(Message::ClearUpcomingQueue));
        assert_eq!(app.queue.len(), 2);
        assert_eq!(
            app.toast.as_ref().map(|toast| &toast.note),
            Some(&Note::UpNextCleared)
        );
        let count = app.toast.as_ref().unwrap().count;
        drop(app.update(Message::ToastPressed(count)));
        let ids: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(app.queue_index, Some(1));
    }
}
