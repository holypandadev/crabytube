//! The App's subscriptions: playback status, ticks, window events and keyboard.

use super::*;

/// Now Playing redraws the lyrics and the queue on every progress tick, so it ticks once a
/// second when no karaoke wake-up drives it. Elsewhere the progress bar ticks 4 times a second.
const NOW_PLAYING_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// How often a window off screen reads the playback position on macOS, for the media session.
const MEDIA_SESSION_TICK: std::time::Duration = std::time::Duration::from_secs(5);

/// The interval of the progress tick on `page`.
pub(super) fn progress_tick(page: &Page) -> std::time::Duration {
    if *page == Page::NowPlaying {
        NOW_PLAYING_TICK
    } else {
        PLAYBACK_TICK
    }
}

impl App {
    pub(super) fn subscription(&self) -> Subscription<Message> {
        let playback_status = iced::Subscription::run_with(
            self.playback.status_receiver(),
            crate::playback::playback_status_stream,
        )
        .map(Message::PlaybackStatusChanged);
        // Progress is only drawn on screen. State changes, including the end of a track, still
        // arrive through `playback_status`, so the tick can stop while the window is in the tray.
        // Synced lyrics schedule their own wake-ups at each word (see `karaoke_wait`).
        let playing = matches!(
            self.playback_status.state,
            PlaybackState::Buffering | PlaybackState::Seeking | PlaybackState::Playing
        );
        let on_screen = !self.window_hidden_to_tray && !self.window_minimized;
        let playback = if on_screen && !self.karaoke_active() && playing {
            iced::time::every(progress_tick(&self.page)).map(|_| Message::PlaybackTick)
        } else if cfg!(target_os = "macos") && !on_screen && playing {
            // macOS moves the Now Playing position on by the wall clock, which a sleep advances while
            // playback stands still. A slow tick lets the media session notice it and report again.
            iced::time::every(MEDIA_SESSION_TICK).map(|_| Message::PlaybackTick)
        } else {
            Subscription::none()
        };
        let mpris = iced::Subscription::run_with(
            self.mpris.command_receiver(),
            crate::mpris::command_stream,
        )
        .map(Message::MprisCommand);
        let tray =
            iced::Subscription::run_with(self.tray.command_receiver(), crate::tray::command_stream)
                .map(Message::TrayCommand);
        let show_requests = Subscription::run(crate::single_instance::show_requests)
            .map(|()| Message::ShowWindowRequested);
        let display_scale = iced::window::events().filter_map(|(_, event)| match event {
            iced::window::Event::Rescaled(scale_factor) => {
                Some(Message::DisplayScaleChanged(scale_factor))
            }
            _ => None,
        });
        let home_visible =
            self.page == Page::Home && !self.window_minimized && !self.window_hidden_to_tray;
        let clock = if home_visible {
            Subscription::run(super::home_panels::clock_stream)
        } else {
            Subscription::none()
        };
        let status = if home_visible && crate::process_stats::AVAILABLE {
            Subscription::run(super::home_panels::status_stream)
        } else {
            Subscription::none()
        };
        let diagnostic = diagnostic_subscription();
        let carousel = if self.carousel.animating() {
            iced::window::frames().map(Message::CarouselFrame)
        } else {
            Subscription::none()
        };
        let volume_rail = if self.volume_rail.animating() {
            iced::window::frames().map(Message::VolumeRailFrame)
        } else {
            Subscription::none()
        };
        let fades = if self.fade_until.is_some() {
            iced::window::frames().map(Message::FadeFrame)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            diagnostic,
            clock,
            status,
            carousel,
            volume_rail,
            fades,
            playback_status,
            playback,
            mpris,
            tray,
            iced::window::close_requests().map(|_| Message::CloseRequested),
            display_scale,
            show_requests,
            iced::window::events().filter_map(|(_, event)| match event {
                iced::window::Event::Focused => Some(Message::WindowFocusChanged(true)),
                iced::window::Event::Unfocused => Some(Message::WindowFocusChanged(false)),
                _ => None,
            }),
            iced::window::resize_events().map(|(_, size)| Message::Resized {
                width: size.width,
                height: size.height,
            }),
            iced::keyboard::listen().map(Message::KeyPressed),
            iced::event::listen_with(mouse_navigation),
            iced::event::listen_with(search_field_escape),
        ])
    }
}

/// Escape in the search field: the field takes the key to give up its focus, so `keyboard::listen`
/// never sees it. It is the only text field, so a taken Escape is always the search field's.
fn search_field_escape(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match (event, status) {
        (
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                // A held key repeats: only the first press counts, or the repeat that follows the
                // closing of the suggestions would also clear the text.
                repeat: false,
                ..
            }),
            iced::event::Status::Captured,
        ) => Some(Message::SearchFieldEscape),
        _ => None,
    }
}

/// The mouse back and forward buttons navigate, like Alt+← and Alt+→.
fn mouse_navigation(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Back)) => {
            Some(Message::Back)
        }
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Forward)) => {
            Some(Message::Forward)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_escape_the_search_field_took_is_passed_on() {
        use iced::keyboard::{Event, Key, Location, Modifiers, key};
        let window = iced::window::Id::unique();
        let event = |named, repeat| {
            iced::Event::Keyboard(Event::KeyPressed {
                key: Key::Named(named),
                modified_key: Key::Named(named),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                location: Location::Standard,
                modifiers: Modifiers::default(),
                text: None,
                repeat,
            })
        };
        let press = |named| event(named, false);
        let captured = iced::event::Status::Captured;
        assert!(matches!(
            search_field_escape(press(key::Named::Escape), captured, window),
            Some(Message::SearchFieldEscape)
        ));
        // Elsewhere Escape is not taken, and `keyboard::listen` handles it.
        assert!(
            search_field_escape(
                press(key::Named::Escape),
                iced::event::Status::Ignored,
                window
            )
            .is_none()
        );
        assert!(search_field_escape(press(key::Named::Enter), captured, window).is_none());
        // A held Escape acts once.
        assert!(search_field_escape(event(key::Named::Escape, true), captured, window).is_none());
    }

    #[test]
    fn the_mouse_back_and_forward_buttons_navigate_and_other_buttons_do_not() {
        let window = iced::window::Id::unique();
        let press = |button| iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button));
        assert!(matches!(
            mouse_navigation(
                press(iced::mouse::Button::Back),
                iced::event::Status::Ignored,
                window
            ),
            Some(Message::Back)
        ));
        assert!(matches!(
            mouse_navigation(
                press(iced::mouse::Button::Forward),
                iced::event::Status::Ignored,
                window
            ),
            Some(Message::Forward)
        ));
        assert!(
            mouse_navigation(
                press(iced::mouse::Button::Left),
                iced::event::Status::Ignored,
                window
            )
            .is_none()
        );
    }
}
