//! Moving through the lists with the keyboard: Up and Down move a cursor over the rows of the page
//! and Enter does what clicking the row does. A key only counts when it was not meant for the
//! search field. Also the mouse wheel over the volume controls.

use super::messages::RowMenu;
use super::*;
use crate::preferences::RecentPlay;
use iced::advanced::widget::{
    self, Operation,
    operation::{Outcome, Scrollable},
};
use iced::keyboard::{Key, Modifiers, key::Named};
use iced::{Rectangle, Vector};

/// How far one notch of the wheel moves the volume.
const VOLUME_STEP: f32 = 0.05;

/// What a key press asks of the row cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowKey {
    Up,
    Down,
    Enter,
}

/// The row key a key press stands for: the arrows and Enter without modifiers.
pub(super) fn row_key(key: &Key, modifiers: Modifiers) -> Option<RowKey> {
    if !modifiers.is_empty() {
        return None;
    }
    match key {
        Key::Named(Named::ArrowUp) => Some(RowKey::Up),
        Key::Named(Named::ArrowDown) => Some(RowKey::Down),
        Key::Named(Named::Enter) => Some(RowKey::Enter),
        _ => None,
    }
}

impl App {
    /// A row key. Where there is a search field, it is asked whether it has the focus before the
    /// key acts, so typing and submitting a search never move the cursor; the other pages have
    /// none to ask, and an operation on a widget that is not there never answers.
    pub(super) fn row_key_pressed(&mut self, key: RowKey) -> Task<Message> {
        if matches!(self.page, Page::Home | Page::Search) {
            operation::is_focused("search-field").map(move |typing| Message::RowKey { key, typing })
        } else {
            self.apply_row_key(key, false)
        }
    }

    pub(super) fn apply_row_key(&mut self, key: RowKey, typing: bool) -> Task<Message> {
        if typing {
            return Task::none();
        }
        match key {
            RowKey::Down => self.move_cursor(true),
            RowKey::Up => self.move_cursor(false),
            RowKey::Enter => match self.cursor_target() {
                Some(target) => self.update(target.play),
                None => Task::none(),
            },
        }
    }

    /// Moves the volume by `delta`. A change of the volume ends the mute, and the state is saved
    /// once a burst of changes settles.
    pub(super) fn step_volume(&mut self, delta: f32) -> Task<Message> {
        let volume = (self.preferences.volume + delta).clamp(0.0, 1.0);
        let volume = (volume * 100.0).round() / 100.0;
        let unmute = if self.preferences.muted {
            self.update(Message::Preference(Preference::Muted(false)))
        } else {
            Task::none()
        };
        let set = self.update(Message::SetVolume(volume));
        self.volume_save_generation = self.volume_save_generation.wrapping_add(1);
        let generation = self.volume_save_generation;
        let save = Task::perform(
            async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                generation
            },
            Message::VolumeSaveDue,
        );
        Task::batch([unmute, set, save])
    }

    /// The wheel over a volume control: a notch up raises the volume.
    pub(super) fn volume_scrolled(&mut self, delta: iced::mouse::ScrollDelta) -> Task<Message> {
        let notches = match delta {
            iced::mouse::ScrollDelta::Lines { y, .. } => y,
            iced::mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
        };
        if !notches.is_finite() || notches == 0.0 {
            return Task::none();
        }
        self.step_volume(VOLUME_STEP * notches.signum() * notches.abs().ceil().min(4.0))
    }
}

/// What the row under the cursor stands for.
pub(super) struct RowTarget {
    /// What pressing Enter on the row does: what clicking it does.
    pub(super) play: Message,
}

impl App {
    /// The rows of the page the cursor can stand on, in the order it moves through them, and how
    /// many places one key press moves it (Home lists its entries in two columns).
    fn cursor_rows(&self) -> (Vec<RowMenu>, usize) {
        let row = |list: &'static str, index: usize| RowMenu { list, index };
        let home = || {
            let layout = self.home_layout(self.area);
            let count = layout.jump.min(self.recent.len());
            let step = if layout.narrow { 1 } else { 2 };
            ((0..count).map(|index| row("recent", index)).collect(), step)
        };
        match &self.page {
            Page::Home => home(),
            // Home's rows only while the search page draws Home: under "Searching…", or results
            // without songs, there is no row to move to.
            Page::Search if self.home_on_screen() => home(),
            Page::Search => (self.search_row_order(), 1),
            Page::Library => (
                (0..self.favorites.len())
                    .map(|index| row("favorites", index))
                    .collect(),
                1,
            ),
            Page::CollectionDetail(_) => (
                (0..self
                    .collection
                    .data
                    .as_ref()
                    .map_or(0, |data| data.tracks.len()))
                    .map(|index| row("collection", index))
                    .collect(),
                1,
            ),
            Page::NowPlaying if self.now_playing_tab == NowPlayingTab::Queue => (
                (0..self.queue.len())
                    .map(|index| row("queue", index))
                    .collect(),
                1,
            ),
            Page::NowPlaying | Page::Settings => (Vec::new(), 1),
        }
    }

    /// Whether `row` is the one the cursor stands on.
    pub(super) fn row_is_cursor(&self, list: &str, index: usize) -> bool {
        self.row_cursor
            .as_ref()
            .is_some_and(|row| row.list == list && row.index == index)
    }

    /// What the row under the cursor stands for, or `None` when there is no cursor or its row is
    /// gone.
    fn cursor_target(&self) -> Option<RowTarget> {
        let row = self.row_cursor.as_ref()?;
        let index = row.index;
        match row.list {
            "recent" => match self.recent.get(index)? {
                RecentPlay::Track(track) => Some(RowTarget {
                    play: Message::PlayContext(Arc::from(vec![track.clone()]), 0),
                }),
                RecentPlay::Collection(collection) => Some(RowTarget {
                    play: Message::OpenCollection(collection.browse.clone()),
                }),
            },
            list @ ("songs" | "live") => {
                let items = if list == "songs" {
                    self.search_songs()
                } else {
                    self.search_live()
                };
                let queue: Arc<[Track]> =
                    items.iter().filter_map(|item| item.track.clone()).collect();
                Some(RowTarget {
                    play: Message::PlayContext(queue, index),
                })
            }
            "collection" => {
                let data = self.collection.data.as_ref()?;
                let Page::CollectionDetail(target) = &self.page else {
                    return None;
                };
                let source = QueueSource {
                    title: data.title.clone(),
                    track_count: data.tracks.len(),
                    browse: Some(target.clone()),
                    thumbnail_url: data.thumbnail_url.clone(),
                    album: target.page_type.as_deref() == Some("MUSIC_PAGE_TYPE_ALBUM"),
                };
                Some(RowTarget {
                    play: Message::PlayCollection(
                        Arc::clone(&self.collection.tracks_context),
                        index,
                        source,
                    ),
                })
            }
            "favorites" => Some(RowTarget {
                play: Message::PlayContext(Arc::clone(&self.favorites_context), index),
            }),
            "queue" => Some(RowTarget {
                play: Message::PlayQueueIndex(index),
            }),
            _ => None,
        }
    }

    /// Moves the cursor a step. The first press puts it on the first row.
    fn move_cursor(&mut self, down: bool) -> Task<Message> {
        let (rows, step) = self.cursor_rows();
        if rows.is_empty() {
            self.row_cursor = None;
            return Task::none();
        }
        let position = self
            .row_cursor
            .as_ref()
            .and_then(|cursor| rows.iter().position(|row| row == cursor));
        let next = match position {
            None => 0,
            Some(at) if down && at + step < rows.len() => at + step,
            Some(at) if down => at,
            Some(at) => at.saturating_sub(step),
        };
        let row = rows[next].clone();
        let reveal = self.reveal_row(&row);
        self.row_cursor = Some(row);
        reveal
    }

    /// Scrolls the list of `row` just far enough that the row, and one row of context on its far
    /// side, are in view. The row is looked up by its widget id; a long list draws only the rows
    /// near the view, so when the row is not drawn the scroll is worked out from the row pitch.
    fn reveal_row(&self, row: &RowMenu) -> Task<Message> {
        let density = self.preferences.ui_density;
        let (scroll_id, pitch) = match row.list {
            "songs" | "live" => ("search-content", None),
            "favorites" => (
                "library-content",
                Some(super::favorites::favorites_row_pitch(density)),
            ),
            "collection" => ("collection-content", Some(density.collection_row_pitch())),
            "queue" => ("queue-content", Some(density.queue_row_pitch())),
            _ => return Task::none(),
        };
        let index = row.index;
        widget::operate(RevealRow {
            row: row_id(row.list, index),
            list: widget::Id::new(scroll_id),
            row_bounds: None,
            viewport: None,
        })
        .then(move |reveal| match reveal {
            Reveal::ScrollTo(y) => operation::scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y }),
            Reveal::Missing if pitch.is_some() => {
                let pitch = pitch.unwrap_or_default();
                let y = (index as f32 * pitch - pitch).max(0.0);
                operation::scroll_to(scroll_id, AbsoluteOffset { x: 0.0, y })
            }
            Reveal::Missing | Reveal::InView => Task::none(),
        })
    }
}

/// The widget id of a row, which lets the list be scrolled to it.
fn row_id(list: &str, index: usize) -> widget::Id {
    widget::Id::from(format!("row-{list}-{index}"))
}

/// Gives a row its widget id. Every row has one, so the cursor can be moved to a row that is only
/// found once the view has been drawn again.
pub(super) fn mark_row<'a>(
    list: &'static str,
    index: usize,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    iced::widget::container(content)
        .id(row_id(list, index))
        .into()
}

/// What looking for a row in its list found.
enum Reveal {
    /// The row is not drawn.
    Missing,
    /// The row is in view with room around it.
    InView,
    /// The offset to scroll the list to.
    ScrollTo(f32),
}

/// Finds a row and the list it is in, and works out where the list has to scroll to show the row.
struct RevealRow {
    row: widget::Id,
    list: widget::Id,
    row_bounds: Option<Rectangle>,
    /// The bounds of the list, those of its content and how far it is scrolled.
    viewport: Option<(Rectangle, Rectangle, f32)>,
}

impl Operation<Reveal> for RevealRow {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Reveal>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if id == Some(&self.row) {
            self.row_bounds = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.list) {
            self.viewport = Some((bounds, content_bounds, translation.y));
        }
    }

    fn finish(&self) -> Outcome<Reveal> {
        let (Some(row), Some((list, content, offset))) = (self.row_bounds, self.viewport) else {
            return Outcome::Some(Reveal::Missing);
        };
        Outcome::Some(reveal_offset(
            row.y - content.y,
            row.height,
            list.height,
            offset,
            (content.height - list.height).max(0.0),
        ))
    }
}

/// Where a list scrolled to `offset` has to scroll to show a row that starts `top` below the
/// start of the list's content, with one row of context on the side it came from.
fn reveal_offset(top: f32, height: f32, viewport: f32, offset: f32, furthest: f32) -> Reveal {
    let bottom = top + height;
    if top - height < offset {
        let target = (top - height).max(0.0);
        if (target - offset).abs() < 0.5 {
            return Reveal::InView;
        }
        return Reveal::ScrollTo(target);
    }
    if bottom + height > offset + viewport {
        let target = (bottom + height - viewport).min(furthest).max(0.0);
        if (target - offset).abs() < 0.5 {
            return Reveal::InView;
        }
        return Reveal::ScrollTo(target);
    }
    Reveal::InView
}

/// Makes the mouse wheel over `content` change the volume.
pub(super) fn wheel_volume<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    iced::widget::mouse_area(content)
        .on_scroll(Message::VolumeScrolled)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_moves_in_steps_and_ends_the_mute() {
        let mut app = App::default();
        app.preferences.volume = 0.5;
        drop(app.step_volume(VOLUME_STEP));
        assert_eq!(app.preferences.volume, 0.55);
        drop(app.step_volume(-VOLUME_STEP));
        drop(app.step_volume(-VOLUME_STEP));
        assert_eq!(app.preferences.volume, 0.45);
        // The ends hold.
        drop(app.step_volume(5.0));
        assert_eq!(app.preferences.volume, 1.0);
        drop(app.step_volume(-5.0));
        assert_eq!(app.preferences.volume, 0.0);
        app.preferences.muted = true;
        drop(app.step_volume(VOLUME_STEP));
        assert!(!app.preferences.muted);
        assert_eq!(app.preferences.volume, 0.05);
    }

    #[test]
    fn the_wheel_raises_and_lowers_the_volume_a_notch_at_a_time() {
        let mut app = App::default();
        app.preferences.volume = 0.5;
        let lines = |y| iced::mouse::ScrollDelta::Lines { x: 0.0, y };
        drop(app.volume_scrolled(lines(1.0)));
        assert_eq!(app.preferences.volume, 0.55);
        drop(app.volume_scrolled(lines(-2.0)));
        assert_eq!(app.preferences.volume, 0.45);
        drop(app.volume_scrolled(lines(0.0)));
        drop(app.volume_scrolled(lines(f32::NAN)));
        assert_eq!(app.preferences.volume, 0.45);
        drop(app.volume_scrolled(iced::mouse::ScrollDelta::Pixels { x: 0.0, y: 40.0 }));
        assert_eq!(app.preferences.volume, 0.5);
    }
    fn song(id: &str) -> Track {
        Track {
            duration_secs: None,
            video_id: id.into(),
            title: format!("Title {id}"),
            artist: "Artist".into(),
            album: None,
            artist_page: None,
            album_page: None,
            thumbnail_url: None,
        }
    }

    fn favorites_page(count: usize) -> App {
        let favorites: Vec<Track> = (0..count).map(|index| song(&format!("f{index}"))).collect();
        App {
            page: Page::Library,
            liked: favorites
                .iter()
                .map(|track| track.video_id.clone())
                .collect(),
            favorites_context: Arc::from(favorites.clone()),
            favorites: Arc::new(favorites),
            ..App::default()
        }
    }

    fn cursor_at(app: &App) -> Option<(&'static str, usize)> {
        app.row_cursor.as_ref().map(|row| (row.list, row.index))
    }

    #[test]
    fn only_the_arrows_and_enter_without_modifiers_are_row_keys() {
        let none = Modifiers::empty();
        assert_eq!(row_key(&Key::Named(Named::ArrowUp), none), Some(RowKey::Up));
        assert_eq!(
            row_key(&Key::Named(Named::ArrowDown), none),
            Some(RowKey::Down)
        );
        assert_eq!(
            row_key(&Key::Named(Named::Enter), none),
            Some(RowKey::Enter)
        );
        assert_eq!(row_key(&Key::Named(Named::ArrowLeft), none), None);
        assert_eq!(row_key(&Key::Character("j".into()), none), None);
        for modifiers in [Modifiers::SHIFT, Modifiers::CTRL, Modifiers::ALT] {
            assert_eq!(row_key(&Key::Named(Named::ArrowDown), modifiers), None);
        }
    }

    #[test]
    fn the_cursor_starts_on_the_first_row_and_stops_at_the_ends() {
        let mut app = favorites_page(3);
        assert_eq!(cursor_at(&app), None);
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), Some(("favorites", 0)));
        drop(app.apply_row_key(RowKey::Down, false));
        drop(app.apply_row_key(RowKey::Down, false));
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), Some(("favorites", 2)));
        drop(app.apply_row_key(RowKey::Up, false));
        assert_eq!(cursor_at(&app), Some(("favorites", 1)));
        drop(app.apply_row_key(RowKey::Up, false));
        drop(app.apply_row_key(RowKey::Up, false));
        assert_eq!(cursor_at(&app), Some(("favorites", 0)));
    }

    #[test]
    fn a_page_without_rows_has_no_cursor() {
        let mut app = App {
            page: Page::Settings,
            ..App::default()
        };
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), None);
        let mut app = favorites_page(0);
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), None);
    }

    #[test]
    fn the_cursor_is_dropped_when_the_page_changes() {
        let mut app = favorites_page(2);
        drop(app.apply_row_key(RowKey::Down, false));
        assert!(app.row_cursor.is_some());
        drop(app.update(Message::Navigate(Page::Settings)));
        assert_eq!(cursor_at(&app), None);
    }

    #[test]
    fn enter_plays_the_row_as_clicking_it_does() {
        let mut app = favorites_page(3);
        assert!(app.cursor_target().is_none());
        drop(app.apply_row_key(RowKey::Down, false));
        drop(app.apply_row_key(RowKey::Down, false));
        let target = app.cursor_target().expect("a row");
        assert!(matches!(target.play, Message::PlayContext(_, 1)));
        drop(app.apply_row_key(RowKey::Enter, false));
        assert_eq!(app.queue_index, Some(1));
    }

    #[test]
    fn a_queue_row_plays_its_place_in_the_queue() {
        let mut app = App {
            page: Page::NowPlaying,
            queue: Arc::new(vec![song("a"), song("b"), song("a")]),
            ..App::default()
        };
        drop(app.apply_row_key(RowKey::Down, false));
        drop(app.apply_row_key(RowKey::Down, false));
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), Some(("queue", 2)));
        let target = app.cursor_target().expect("a row");
        assert!(matches!(target.play, Message::PlayQueueIndex(2)));
        // The lyrics tab has no rows.
        app.now_playing_tab = NowPlayingTab::Lyrics;
        app.row_cursor = None;
        drop(app.apply_row_key(RowKey::Down, false));
        assert_eq!(cursor_at(&app), None);
    }

    #[test]
    fn keys_do_nothing_to_rows_while_the_search_field_is_typed_in() {
        let mut app = favorites_page(2);
        drop(app.apply_row_key(RowKey::Down, true));
        assert_eq!(cursor_at(&app), None);
    }

    #[test]
    fn the_list_scrolls_just_far_enough_to_show_the_row() {
        let furthest = 5000.0;
        // A row in view with room around it needs no scroll.
        assert!(matches!(
            reveal_offset(300.0, 60.0, 500.0, 200.0, furthest),
            Reveal::InView
        ));
        // A row below the view scrolls so it and one row more are in view.
        match reveal_offset(600.0, 60.0, 500.0, 200.0, furthest) {
            Reveal::ScrollTo(y) => assert_eq!(y, 220.0),
            _ => panic!("expected a scroll"),
        }
        // A row above the view scrolls up to it with one row of context.
        match reveal_offset(100.0, 60.0, 500.0, 400.0, furthest) {
            Reveal::ScrollTo(y) => assert_eq!(y, 40.0),
            _ => panic!("expected a scroll"),
        }
        // The first row goes to the very top.
        match reveal_offset(0.0, 60.0, 500.0, 300.0, furthest) {
            Reveal::ScrollTo(y) => assert_eq!(y, 0.0),
            _ => panic!("expected a scroll"),
        }
        // The last rows do not scroll past the end of the content.
        match reveal_offset(4900.0, 60.0, 500.0, 4000.0, 4500.0) {
            Reveal::ScrollTo(y) => assert_eq!(y, 4500.0),
            _ => panic!("expected a scroll"),
        }
    }
}
