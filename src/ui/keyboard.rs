//! Keys that act on the player or the page without a modifier: the pages, the volume, mute and the
//! lyrics offset. A key only counts when it was not meant for a text field:
//! the field captures the keys it types, and the arrows and Enter it leaves over are checked
//! against its focus before they act.

use super::*;
use iced::keyboard::{
    Key, Modifiers,
    key::{Code, Physical},
};

/// How far one key press moves the volume.
const VOLUME_STEP: f32 = 0.05;
/// How far one key press moves the lyrics of the current song, as the Earlier and Later buttons do.
const LYRICS_STEP_MS: i64 = 500;

/// What a key press asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shortcut {
    GoHome,
    GoFavorites,
    GoSettings,
    OpenNowPlaying,
    ToggleMute,
    VolumeUp,
    VolumeDown,
    LyricsEarlier,
    LyricsLater,
}

/// The shortcut a key press stands for, or `None`. Letters and punctuation are matched by what
/// the key types, falling back on its place on the keyboard for layouts that are not Latin, as
/// Ctrl+K is; digits are matched by their place, so they work on layouts that need Shift for them.
pub(super) fn shortcut_for(
    key: &Key,
    physical_key: Physical,
    modifiers: Modifiers,
) -> Option<Shortcut> {
    if modifiers.control() || modifiers.alt() || modifiers.command() || modifiers.logo() {
        return None;
    }
    if !modifiers.shift() {
        match physical_key {
            Physical::Code(Code::Digit1) => return Some(Shortcut::GoHome),
            Physical::Code(Code::Digit2) => return Some(Shortcut::GoFavorites),
            Physical::Code(Code::Digit3) => return Some(Shortcut::GoSettings),
            _ => {}
        }
    }
    if let Key::Character(typed) = key {
        match typed.as_str() {
            "+" | "=" => return Some(Shortcut::VolumeUp),
            "-" | "_" => return Some(Shortcut::VolumeDown),
            "[" | "{" => return Some(Shortcut::LyricsEarlier),
            "]" | "}" => return Some(Shortcut::LyricsLater),
            _ => {}
        }
    }
    match physical_key {
        Physical::Code(Code::Equal | Code::NumpadAdd) => return Some(Shortcut::VolumeUp),
        Physical::Code(Code::Minus | Code::NumpadSubtract) => return Some(Shortcut::VolumeDown),
        Physical::Code(Code::BracketLeft) => return Some(Shortcut::LyricsEarlier),
        Physical::Code(Code::BracketRight) => return Some(Shortcut::LyricsLater),
        _ => {}
    }
    if modifiers.shift() {
        return None;
    }
    match key.to_latin(physical_key)? {
        'p' => Some(Shortcut::OpenNowPlaying),
        'm' => Some(Shortcut::ToggleMute),
        _ => None,
    }
}

impl App {
    /// A key press that stands for a shortcut. Where there is a search field, it is asked whether it
    /// has the focus before the shortcut acts; the other pages have none to ask, and an operation
    /// on a widget that is not there never answers.
    pub(super) fn shortcut_pressed(&mut self, shortcut: Shortcut) -> Task<Message> {
        if matches!(self.page, Page::Home | Page::Search) {
            operation::is_focused("search-field")
                .map(move |typing| Message::Shortcut { shortcut, typing })
        } else {
            self.apply_shortcut(shortcut, false)
        }
    }

    pub(super) fn apply_shortcut(&mut self, shortcut: Shortcut, typing: bool) -> Task<Message> {
        if typing {
            return Task::none();
        }
        match shortcut {
            Shortcut::GoHome => self.update(Message::Navigate(Page::Home)),
            Shortcut::GoFavorites => self.update(Message::Navigate(Page::Library)),
            Shortcut::GoSettings => self.update(Message::Navigate(Page::Settings)),
            Shortcut::OpenNowPlaying => self.update(Message::OpenNowPlaying),
            Shortcut::ToggleMute => {
                let muted = !self.preferences.muted;
                self.update(Message::Preference(Preference::Muted(muted)))
            }
            Shortcut::VolumeUp => self.step_volume(VOLUME_STEP),
            Shortcut::VolumeDown => self.step_volume(-VOLUME_STEP),
            Shortcut::LyricsEarlier => self.step_lyrics(-LYRICS_STEP_MS),
            Shortcut::LyricsLater => self.step_lyrics(LYRICS_STEP_MS),
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

    /// Moves the lyrics of the current song, when they are timed and a song is playing.
    fn step_lyrics(&mut self, delta_ms: i64) -> Task<Message> {
        let timed = self
            .lyrics
            .as_ref()
            .is_some_and(|lyrics| !lyrics.lines.is_empty());
        if self.playback_status.track.is_none() || !timed {
            return Task::none();
        }
        self.update(Message::AdjustLyricsOffset(delta_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(typed: &str, code: Code) -> (Key, Physical) {
        (Key::Character(typed.into()), Physical::Code(code))
    }

    fn pressed(key: (Key, Physical), modifiers: Modifiers) -> Option<Shortcut> {
        shortcut_for(&key.0, key.1, modifiers)
    }

    #[test]
    fn digits_open_the_pages_and_p_opens_now_playing() {
        let none = Modifiers::empty();
        assert_eq!(
            pressed(character("1", Code::Digit1), none),
            Some(Shortcut::GoHome)
        );
        assert_eq!(
            pressed(character("2", Code::Digit2), none),
            Some(Shortcut::GoFavorites)
        );
        assert_eq!(
            pressed(character("3", Code::Digit3), none),
            Some(Shortcut::GoSettings)
        );
        assert_eq!(
            pressed(character("p", Code::KeyP), none),
            Some(Shortcut::OpenNowPlaying)
        );
        assert_eq!(pressed(character("p", Code::KeyP), Modifiers::SHIFT), None);
    }

    #[test]
    fn digits_work_on_a_layout_that_needs_shift_for_them() {
        // On an AZERTY keyboard the key of 1 types "&" without Shift.
        assert_eq!(
            pressed(character("&", Code::Digit1), Modifiers::empty()),
            Some(Shortcut::GoHome)
        );
    }

    #[test]
    fn m_mutes_and_plus_and_minus_move_the_volume() {
        let none = Modifiers::empty();
        assert_eq!(
            pressed(character("m", Code::KeyM), none),
            Some(Shortcut::ToggleMute)
        );
        assert_eq!(
            pressed(character("+", Code::Equal), Modifiers::SHIFT),
            Some(Shortcut::VolumeUp)
        );
        assert_eq!(
            pressed(character("=", Code::Equal), none),
            Some(Shortcut::VolumeUp)
        );
        assert_eq!(
            pressed(character("-", Code::Minus), none),
            Some(Shortcut::VolumeDown)
        );
        assert_eq!(
            pressed((Key::Unidentified, Physical::Code(Code::NumpadAdd)), none),
            Some(Shortcut::VolumeUp)
        );
    }

    #[test]
    fn brackets_move_the_lyrics_earlier_and_later() {
        let none = Modifiers::empty();
        assert_eq!(
            pressed(character("[", Code::BracketLeft), none),
            Some(Shortcut::LyricsEarlier)
        );
        assert_eq!(
            pressed(character("]", Code::BracketRight), none),
            Some(Shortcut::LyricsLater)
        );
        // On a layout where the brackets need AltGr the place of the key still counts.
        assert_eq!(
            pressed(character("ñ", Code::BracketLeft), none),
            Some(Shortcut::LyricsEarlier)
        );
    }

    #[test]
    fn a_key_with_ctrl_alt_or_command_is_not_a_shortcut() {
        for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::LOGO] {
            assert_eq!(pressed(character("m", Code::KeyM), modifiers), None);
            assert_eq!(pressed(character("1", Code::Digit1), modifiers), None);
            assert_eq!(pressed(character("-", Code::Minus), modifiers), None);
        }
    }

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

    fn timed_lyrics() -> crate::lyrics::Lyrics {
        crate::lyrics::Lyrics {
            source: "fixture".into(),
            lines: vec![crate::lyrics::Line {
                start: 10.0,
                end: None,
                text: "a line".into(),
                words: Vec::new(),
                translation: None,
                romanization: None,
                background: None,
            }],
            length_mismatch: false,
            plain: None,
        }
    }

    fn playing_song() -> super::PlaybackStatus {
        super::PlaybackStatus {
            track: Some(crate::music::Track {
                duration_secs: None,
                video_id: "abc".into(),
                title: "Song".into(),
                artist: "Artist".into(),
                album: None,
                artist_page: None,
                album_page: None,
                thumbnail_url: None,
            }),
            ..super::PlaybackStatus::default()
        }
    }

    #[test]
    fn the_brackets_move_the_lyrics_of_the_playing_song_by_half_a_second() {
        let mut app = App {
            playback_status: playing_song(),
            lyrics: Some(timed_lyrics()),
            ..App::default()
        };
        drop(app.apply_shortcut(Shortcut::LyricsLater, false));
        assert_eq!(app.current_lyrics_offset(), 500);
        drop(app.apply_shortcut(Shortcut::LyricsEarlier, false));
        drop(app.apply_shortcut(Shortcut::LyricsEarlier, false));
        assert_eq!(app.current_lyrics_offset(), -500);
    }

    #[test]
    fn the_lyrics_stay_put_without_a_song_or_without_timing() {
        // Timed lyrics but nothing playing.
        let mut app = App {
            lyrics: Some(timed_lyrics()),
            ..App::default()
        };
        drop(app.apply_shortcut(Shortcut::LyricsLater, false));
        assert_eq!(app.current_lyrics_offset(), 0);
        // A song playing but plain text only.
        let mut plain = timed_lyrics();
        plain.lines.clear();
        plain.plain = Some("words".into());
        let mut app = App {
            playback_status: playing_song(),
            lyrics: Some(plain),
            ..App::default()
        };
        drop(app.apply_shortcut(Shortcut::LyricsLater, false));
        assert_eq!(app.current_lyrics_offset(), 0);
    }
    #[test]
    fn nothing_acts_while_the_search_field_is_being_typed_in() {
        let mut app = App::default();
        app.preferences.muted = false;
        drop(app.apply_shortcut(Shortcut::ToggleMute, true));
        assert!(!app.preferences.muted);
        drop(app.apply_shortcut(Shortcut::ToggleMute, false));
        assert!(app.preferences.muted);
    }
}
