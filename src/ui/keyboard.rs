//! The mouse wheel over the volume controls, and the volume steps it moves by.

use super::*;

/// How far one notch of the wheel moves the volume.
const VOLUME_STEP: f32 = 0.05;

impl App {
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
}
