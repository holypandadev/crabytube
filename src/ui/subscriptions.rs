//! The App's subscriptions: playback status, ticks, window events and keyboard.

use super::*;

/// Home's greeting changes once a minute at most. The status line has its own stream, which
/// reads the process every few seconds. Both run only while Home is on screen.
const HOME_CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(60);
/// Now Playing redraws the lyrics and the queue on every progress tick, so it ticks once a
/// second when no karaoke wake-up drives it. Elsewhere the progress bar ticks 4 times a second.
const NOW_PLAYING_TICK: std::time::Duration = std::time::Duration::from_secs(1);

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
        let playback = if !self.window_hidden_to_tray
            && !self.window_minimized
            && !self.karaoke_active()
            && matches!(
                self.playback_status.state,
                PlaybackState::Buffering | PlaybackState::Seeking | PlaybackState::Playing
            ) {
            iced::time::every(progress_tick(&self.page)).map(|_| Message::PlaybackTick)
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
        let display_scale = iced::window::events().filter_map(|(_, event)| match event {
            iced::window::Event::Rescaled(scale_factor) => {
                Some(Message::DisplayScaleChanged(scale_factor))
            }
            _ => None,
        });
        let home_visible =
            self.page == Page::Home && !self.window_minimized && !self.window_hidden_to_tray;
        let clock = if home_visible {
            iced::time::every(HOME_CLOCK_TICK).map(|_| Message::ClockTick)
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
            fades,
            playback_status,
            playback,
            mpris,
            tray,
            iced::window::close_requests().map(|_| Message::CloseRequested),
            display_scale,
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
        ])
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
