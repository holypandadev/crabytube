//! Track menu actions beyond the queue: "Start radio" and the "Go to" destinations a track names.

use super::toast::Note;
use super::*;

impl App {
    /// Starts the catalog radio of `track` in the background. The queue is replaced when it
    /// arrives, so the track plays first.
    pub(super) fn start_radio(&mut self, track: Track) -> Task<Message> {
        let service = Arc::clone(&self.music_service);
        let video = track.video_id.clone();
        let generation = self.queue_generation;
        // A newer request, or a track picked meanwhile, makes this answer late.
        self.radio_generation = self.radio_generation.wrapping_add(1);
        let request = self.radio_generation;
        let note = self.show_toast(Note::RadioLoading(request), None);
        let radio = Task::perform(
            async move {
                service
                    .radio(&video)
                    .await
                    .map_err(|error| error.user_message())
            },
            move |result| Message::StartRadioLoaded {
                track,
                generation,
                request,
                result,
            },
        );
        Task::batch([note, radio])
    }

    /// Plays the radio that came back for `track`: the track first, then the radio in its order.
    /// When the queue changed while the radio loaded, another track was picked, or another radio was
    /// asked for, the radio is dropped, so a late answer never takes over what is playing.
    pub(super) fn start_radio_loaded(
        &mut self,
        track: Track,
        generation: u64,
        request: u64,
        result: Result<Vec<Track>, String>,
    ) -> Task<Message> {
        if generation != self.queue_generation || request != self.radio_generation {
            self.hide_toast(&Note::RadioLoading(request));
            return Task::none();
        }
        let Ok(radio) = result else {
            return self.show_toast(Note::RadioFailed, None);
        };
        let tracks: Vec<Track> = std::iter::once(track).chain(radio).collect();
        let play = self.update(Message::PlayContext(Arc::from(tracks), 0));
        Task::batch([play, self.show_toast(Note::RadioStarted, None)])
    }
}

/// The "Go to" entries of a track menu, in menu order. Only the destinations the catalog named
/// are offered.
pub(super) fn go_to_destinations(track: &Track) -> Vec<(&'static str, BrowseTarget)> {
    [
        ("Go to artist", track.artist_page.clone()),
        ("Go to album", track.album_page.clone()),
    ]
    .into_iter()
    .filter_map(|(label, page)| page.map(|page| (label, page)))
    .collect()
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

    fn page(id: &str) -> BrowseTarget {
        BrowseTarget {
            browse_id: id.into(),
            params: None,
            page_type: None,
        }
    }

    #[test]
    fn go_to_offers_only_the_destinations_the_catalog_named() {
        let mut known = track("a");
        known.artist_page = Some(page("UCartist"));
        assert_eq!(
            go_to_destinations(&known),
            vec![("Go to artist", page("UCartist"))]
        );
        assert!(go_to_destinations(&track("b")).is_empty());
        known.album_page = Some(page("MPalbum"));
        let labels: Vec<_> = go_to_destinations(&known)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(labels, ["Go to artist", "Go to album"]);
    }

    #[test]
    fn a_radio_plays_the_track_first_and_then_the_radio_in_order() {
        let mut app = App::default();
        let generation = app.queue_generation;
        let _ = app.start_radio_loaded(
            track("seed"),
            generation,
            app.radio_generation,
            Ok(vec![track("two"), track("three")]),
        );
        let queued: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(queued, ["seed", "two", "three"]);
        assert_eq!(app.queue_index, Some(0));
    }

    #[test]
    fn a_failed_radio_leaves_the_queue_alone_and_says_so() {
        let mut app = App::default();
        let generation = app.queue_generation;
        let _ = app.start_radio_loaded(
            track("seed"),
            generation,
            app.radio_generation,
            Err("offline".into()),
        );
        assert!(app.queue.is_empty());
        assert_eq!(
            app.toast.as_ref().map(|toast| &toast.note),
            Some(&Note::RadioFailed)
        );
    }

    #[test]
    fn a_radio_that_arrives_after_the_queue_changed_is_dropped() {
        let mut app = App::default();
        drop(app.start_radio(track("seed")));
        let asked_at = app.queue_generation;
        let request = app.radio_generation;
        // The user plays something else while the radio loads.
        let _ = app.update(Message::PlayContext(Arc::from(vec![track("other")]), 0));
        let _ = app.start_radio_loaded(track("seed"), asked_at, request, Ok(vec![track("two")]));
        let queued: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(queued, ["other"]);
        assert!(
            !matches!(
                app.toast.as_ref().map(|toast| &toast.note),
                Some(Note::RadioLoading(_))
            ),
            "the late radio's note is gone"
        );
    }

    #[test]
    fn a_radio_that_arrives_after_another_was_asked_for_is_dropped() {
        let mut app = App::default();
        drop(app.start_radio(track("a")));
        let first = app.radio_generation;
        drop(app.start_radio(track("b")));
        let generation = app.queue_generation;
        // The first answer comes last: the second request wins.
        let _ = app.start_radio_loaded(track("a"), generation, first, Ok(vec![track("a2")]));
        assert!(app.queue.is_empty());
        let _ = app.start_radio_loaded(
            track("b"),
            generation,
            app.radio_generation,
            Ok(vec![track("b2")]),
        );
        let queued: Vec<&str> = app.queue.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(queued, ["b", "b2"]);
    }
}
