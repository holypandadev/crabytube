//! The App's subscriptions: playback status, ticks, window events and keyboard.

use super::*;

/// Home's greeting changes once a minute at most. The status line has its own stream, which
/// reads the process every few seconds. Both run only while Home is on screen.
const HOME_CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(60);

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
            && !self.karaoke_active()
            && matches!(
                self.playback_status.state,
                PlaybackState::Buffering | PlaybackState::Seeking | PlaybackState::Playing
            ) {
            iced::time::every(PLAYBACK_TICK).map(|_| Message::PlaybackTick)
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
        ])
    }
}
