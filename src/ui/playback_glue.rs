//! Playback and queue: the messages that drive the player, the queue and the radio.

use super::*;
use crate::preferences::{MAX_RECENT_PLAYS, RecentCollection, RecentPlay};

/// Bounds of the karaoke wait between two highlight changes.
const KARAOKE_MIN_WAIT: Duration = Duration::from_millis(60);
const KARAOKE_MAX_WAIT: Duration = Duration::from_millis(500);

impl App {
    /// Adds the track that just started playing to "Jump back in", once per start. A track that
    /// plays from an album or playlist records that collection instead.
    pub(super) fn record_playing_track(&mut self) {
        if self.playback_status.state != PlaybackState::Playing {
            return;
        }
        let Some(track) = self.playback_status.track.clone() else {
            return;
        };
        if self.recent_track_id.as_deref() == Some(track.video_id.as_str()) {
            return;
        }
        self.recent_track_id = Some(track.video_id.clone());
        if !self.preferences.remember_history {
            return;
        }
        let play = match &self.queue_source {
            Some(QueueSource {
                title,
                track_count,
                browse: Some(browse),
                thumbnail_url,
                album,
            }) => RecentPlay::Collection(RecentCollection {
                title: title.clone(),
                browse: browse.clone(),
                track_count: *track_count,
                thumbnail_url: thumbnail_url.clone(),
                album: *album,
            }),
            _ => RecentPlay::Track(track),
        };
        let key = play.key();
        self.recent.retain(|past| past.key() != key);
        self.recent.insert(0, play);
        self.recent.truncate(MAX_RECENT_PLAYS);
        self.persist();
    }

    pub(super) fn update_playback_status(&mut self, status: PlaybackStatus) -> Task<Message> {
        self.apply_helper_memory_budget();
        let previous_state = self.playback_status.state;
        let previous_position = self.playback_status.position;
        let previous_track = self.playback_status.track.clone();
        self.playback_status = status;
        self.playback_status_at = std::time::Instant::now();
        self.record_playing_track();
        // A position that moves back within the same track is a seek or a restart, whatever started it.
        let same_track = previous_track.as_ref().map(|track| &track.video_id)
            == self
                .playback_status
                .track
                .as_ref()
                .map(|track| &track.video_id);
        if same_track && self.playback_status.position + Duration::from_secs(1) < previous_position
        {
            self.resync_lyrics();
        }
        self.release_previous_cover(previous_track.as_ref());
        let mut metadata_tasks = self.adopt_resolved_metadata(previous_track);
        metadata_tasks.extend(self.track_notice_task());
        self.request_prefetch_near_end();
        self.sync_discord_presence(false);
        let old_active = self.lyric_active;
        let lyric_position = self.effective_lyrics_position();
        let active = self.lyrics.as_ref().and_then(|l| l.active(lyric_position));
        if active != self.lyric_active {
            self.lyric_active = active;
        }

        // Follow first, so the branches below cannot skip the lyric scroll.
        let resync = self.lyric_active.is_some() && std::mem::take(&mut self.lyrics_resync);
        let mut tasks = vec![self.follow_lyrics_task(old_active, resync)];
        tasks.append(&mut metadata_tasks);

        // Search tracks may not have a duration until the existing playback
        // resolver finishes. Retry synced lyrics once the stream reports its
        // actual duration.
        if self.lyrics_waiting_for_duration
            && !self.lyrics_loading
            && self.playback_status.duration.is_some()
            && let Some(track) = self.playback_status.track.clone()
        {
            tasks.push(self.load_lyrics(track));
        }

        // Auto-advance with repeat support, gated by the Autoplay toggle.
        if self.playback_status.state == PlaybackState::Finished
            && previous_state != PlaybackState::Finished
            && let Some(task) = self.advance_after_finish()
        {
            tasks.push(task);
        }
        // Synced lyrics wake the app at the next word, not on a fixed tick.
        if self.karaoke_active()
            && !self.karaoke_tick_pending
            && let Some(wait) = self.karaoke_wait()
        {
            self.karaoke_tick_pending = true;
            tasks.push(Task::perform(
                async move { tokio::time::sleep(wait).await },
                |()| Message::PlaybackTick,
            ));
        }
        if self.autoplay
            && self
                .queue
                .len()
                .saturating_sub(self.queue_index.unwrap_or(0) + 1)
                <= 2
            && !self.radio_loading
            && self.radio_error.is_none()
            && self.playback_status.state == PlaybackState::Playing
        {
            tasks.push(self.load_radio());
        }
        Task::batch(tasks)
    }

    /// Synced lyrics are on screen and sung word by word while the track plays.
    pub(super) fn karaoke_active(&self) -> bool {
        self.page == Page::NowPlaying
            && self.now_playing_tab == NowPlayingTab::Lyrics
            && self.playback_status.state == PlaybackState::Playing
            && !self.window_hidden_to_tray
            && self
                .lyrics
                .as_ref()
                .is_some_and(|lyrics| lyrics.lines.iter().any(|line| !line.words.is_empty()))
    }

    /// How long until the next word or line of the lyrics starts, counted from the
    /// position the player has reached now. Capped, so the progress bar still moves.
    pub(super) fn karaoke_wait(&self) -> Option<Duration> {
        let lyrics = self.lyrics.as_ref()?;
        let since_status = if self.playback_status.state == PlaybackState::Playing {
            self.playback_status_at.elapsed().as_secs_f64()
        } else {
            0.0
        };
        let now = self.effective_lyrics_position() + since_status;
        let until = lyrics
            .next_boundary(self.lyric_active, now)
            .map_or(KARAOKE_MAX_WAIT, |boundary| {
                Duration::from_secs_f64((boundary - now).max(0.0))
            })
            .clamp(KARAOKE_MIN_WAIT, KARAOKE_MAX_WAIT);
        // A few milliseconds past the boundary, so the wake-up finds the word started.
        Some(until + Duration::from_millis(10))
    }

    /// Seeks the player. The lyrics view follows the new position, even when the user
    /// had scrolled it by hand before.
    pub(super) fn seek_playback(&mut self, target: Duration) {
        // A live broadcast has no position to move to.
        if self.playback_status.live {
            return;
        }
        self.playback.seek(target);
        self.resync_lyrics();
    }

    /// Resumes lyrics following, and scrolls to the active line on the next status update.
    pub(super) fn resync_lyrics(&mut self) {
        self.lyrics_follow_paused = false;
        self.lyrics_resync = true;
    }

    /// The resolver fills in title, artist, artwork and length for a track that was started
    /// from its video ID alone. The queue, the artwork and the lyrics follow once they arrive.
    /// Gives a favorite or queue entry without an artist the artist of its video.
    fn fill_artist_of(&mut self, video_id: &str, artist: &str) {
        let mut favorites_changed = false;
        for favorite in self
            .favorites
            .iter_mut()
            .filter(|track| track.video_id == video_id && track.artist.trim().is_empty())
        {
            favorite.artist = artist.to_owned();
            favorites_changed = true;
        }
        for entry in self
            .queue
            .iter_mut()
            .filter(|track| track.video_id == video_id && track.artist.trim().is_empty())
        {
            entry.artist = artist.to_owned();
        }
        if favorites_changed {
            self.persist();
        }
    }

    pub(super) fn adopt_resolved_metadata(
        &mut self,
        previous: Option<Track>,
    ) -> Vec<Task<Message>> {
        let (Some(previous), Some(current)) = (previous, self.playback_status.track.clone()) else {
            return Vec::new();
        };
        // Search rows often arrive without an artist. Once the stream names it, the
        // favorites and queue entries of that video get it too.
        if previous.video_id == current.video_id
            && previous.artist.trim().is_empty()
            && !current.artist.trim().is_empty()
        {
            self.fill_artist_of(&current.video_id, &current.artist);
        }
        if previous.video_id != current.video_id
            || !metadata_pending(&previous)
            || metadata_pending(&current)
        {
            return Vec::new();
        }
        tracing::info!(video_id = %current.video_id, "track metadata resolved");
        if let Some(index) = self.queue_index
            && let Some(entry) = self.queue.get_mut(index)
            && entry.video_id == current.video_id
        {
            *entry = current.clone();
            self.persist();
        }
        let mut tasks = Vec::new();
        if previous.thumbnail_url != current.thumbnail_url {
            tasks.push(self.load_artwork(current.thumbnail_url.clone()));
        }
        if self.now_playing_tab == NowPlayingTab::Lyrics {
            tasks.push(self.load_lyrics(current));
        }
        tasks
    }

    /// Starts the next queued track after a track finishes, if the repeat mode allows it.
    pub(super) fn advance_after_finish(&mut self) -> Option<Task<Message>> {
        if self.repeat == RepeatMode::One
            && let Some(index) = self.queue_index
            && let Some(track) = self.queue.get(index).cloned()
        {
            return Some(self.play_queued_track(track, index));
        }
        if let Some(next) = self.next_queue_index() {
            let track = self.queue[next].clone();
            return Some(self.play_queued_track(track, next));
        }
        if self.repeat == RepeatMode::All && !self.queue.is_empty() {
            let track = self.queue[0].clone();
            return Some(self.play_queued_track(track, 0));
        }
        None
    }

    /// Inserts `track` after the current song (`next`) or at the end of the queue.
    pub(super) fn enqueue(&mut self, track: Track, next: bool) -> Task<Message> {
        if self.playback_status.track.is_none() && self.queue_index.is_none() {
            return self.update(Message::SelectTrack(track));
        }
        if self.queue.len() >= crate::preferences::MAX_QUEUE_TRACKS {
            self.queue_notice = Some("The queue is full.".into());
            return Task::none();
        }
        let position = match (next, self.queue_index) {
            (true, Some(current)) => (current + 1).min(self.queue.len()),
            _ => self.queue.len(),
        };
        self.queue.insert(position, track);
        if let Some(current) = self.queue_index
            && position <= current
        {
            self.queue_index = Some(current + 1);
        }
        self.queue_source = None;
        self.queue_generation = self.queue_generation.wrapping_add(1);
        self.persist();
        Task::none()
    }

    pub(super) fn remember_duration(&mut self, video_id: String, duration: Duration) {
        if self.known_durations.len() >= 4_096 {
            self.known_durations.clear();
        }
        self.known_durations.insert(video_id, duration);
    }

    /// Video IDs after `index` that the next BotGuard run should cover (see playback::pot).
    pub(super) fn upcoming_video_ids(&self, index: usize) -> Vec<String> {
        self.queue
            .iter()
            .skip(index + 1)
            .take(10)
            .map(|track| track.video_id.clone())
            .collect()
    }

    pub(super) fn play_queued_track(&mut self, track: Track, index: usize) -> Task<Message> {
        // Ignore rapid double-taps on the track that is already resolving:
        // each new play() would cancel the in-flight resolution and restart
        // it, which on a cold start delays playback further.
        if self.queue_index == Some(index)
            && self
                .playback_status
                .track
                .as_ref()
                .is_some_and(|current| current.video_id == track.video_id)
            && matches!(
                self.playback_status.state,
                PlaybackState::Resolving | PlaybackState::Buffering
            )
        {
            return Task::none();
        }
        tracing::info!(title = %track.title, index, queue_len = self.queue.len(), "selected catalog track for existing playback");
        if let Some(token) = self.radio_cancel.take() {
            token.cancel();
        }
        self.radio_generation = self.radio_generation.wrapping_add(1);
        self.radio_loading = false;
        self.radio_error = None;
        self.radio_attempted_for = None;
        self.queue_index = Some(index);
        crate::playback::set_upcoming_video_ids(self.upcoming_video_ids(index));
        self.persist();
        self.seek_preview = None;
        if let Some(token) = self.lyrics_cancel.take() {
            token.cancel();
        }
        self.lyrics_generation = self.lyrics_generation.wrapping_add(1);
        self.lyrics = None;
        self.lyrics_error = None;
        self.lyrics_loading = false;
        self.lyrics_waiting_for_duration = false;
        self.lyric_active = None;
        self.playback.play(track.clone());
        self.playback_status.state = PlaybackState::Resolving;
        self.playback_status.track = Some(track.clone());
        self.playback_status.position = Duration::ZERO;
        self.playback_status.duration = None;
        self.playback_status.error = None;
        let artwork = self.load_artwork(track.thumbnail_url.clone());
        let lyrics = if self.now_playing_tab == NowPlayingTab::Lyrics {
            self.load_lyrics(track)
        } else {
            Task::none()
        };
        Task::batch([artwork, lyrics])
    }

    pub(super) fn move_queue(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let item = self.queue.remove(from);
        self.queue.insert(to, item);
        self.queue_index = self.queue_index.map(|i| {
            if i == from {
                to
            } else if from < i && i <= to {
                i - 1
            } else if to <= i && i < from {
                i + 1
            } else {
                i
            }
        });
        self.persist();
    }

    /// Drops one upcoming or earlier track. The playing track stays in place.
    pub(super) fn remove_from_queue(&mut self, index: usize) {
        if index >= self.queue.len() || self.queue_index == Some(index) {
            return;
        }
        self.queue.remove(index);
        self.queue_index = self.queue_index.map(|current| {
            if index < current {
                current - 1
            } else {
                current
            }
        });
        self.queue_source = None;
        self.queue_generation = self.queue_generation.wrapping_add(1);
        self.persist();
    }

    /// Drops every track after the playing one; the playing track keeps playing.
    pub(super) fn clear_upcoming_queue(&mut self) {
        let keep = self.queue_index.map_or(0, |current| current + 1);
        if keep >= self.queue.len() {
            return;
        }
        self.queue.truncate(keep);
        self.queue_source = None;
        self.queue_generation = self.queue_generation.wrapping_add(1);
        self.persist();
    }

    pub(super) fn shuffle_queue(&mut self) {
        let current = self.queue_index;
        let start = if self.preferences.shuffle_all {
            0
        } else {
            current.map_or(0, |i| i + 1)
        };
        if start < self.queue.len() {
            self.queue[start..].shuffle(&mut rand::thread_rng());
        }
        self.persist();
    }

    pub(super) fn load_radio(&mut self) -> Task<Message> {
        if self.radio_loading || !self.autoplay {
            return Task::none();
        }
        let Some(track) = self.playback_status.track.as_ref() else {
            return Task::none();
        };
        let video = track.video_id.clone();
        if self.radio_attempted_for.as_deref() == Some(&video) {
            return Task::none();
        }
        self.radio_attempted_for = Some(video.clone());
        self.radio_loading = true;
        self.radio_error = None;
        self.radio_generation = self.radio_generation.wrapping_add(1);
        let generation = self.radio_generation;
        let token = CancellationToken::new();
        self.radio_cancel = Some(token.clone());
        let service = Arc::clone(&self.music_service);
        Task::perform(
            async move {
                tokio::select! {_ = token.cancelled()=>Err("Cancelled".into()),r=service.radio(&video)=>r.map_err(|e|e.user_message())}
            },
            move |result| Message::RadioLoaded { generation, result },
        )
    }

    pub(super) fn next_queue_index(&self) -> Option<usize> {
        let current = self.queue_index?;
        if current + 1 < self.queue.len() {
            Some(current + 1)
        } else {
            None
        }
    }

    pub(super) fn prev_queue_index(&self) -> Option<usize> {
        let current = self.queue_index?;
        if current > 0 && !self.queue.is_empty() {
            Some(current - 1)
        } else {
            None
        }
    }

    pub(super) fn mpris_player_state(&self) -> crate::mpris::PlayerState {
        let track = self.playback_status.track.as_ref();
        let position = self.playback_status.position;
        crate::mpris::PlayerState {
            title: track.map(|track| track.title.clone()),
            artist: track.map(|track| track.artist.clone()),
            album: track.and_then(|track| track.album.clone()),
            art_url: track.and_then(|track| track.thumbnail_url.clone()),
            playback_status: match self.playback_status.state {
                PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                    "Playing"
                }
                PlaybackState::Paused => "Paused",
                _ => "Stopped",
            }
            .into(),
            position,
            duration: self
                .playback_status
                .duration
                .or_else(|| track.and_then(|track| self.track_duration(&track.video_id))),
            volume: if self.preferences.muted {
                0.0
            } else {
                self.preferences.volume.clamp(0.0, 1.0) as f64
            },
            shuffle: self.shuffle,
            loop_status: match self.repeat {
                RepeatMode::Off => "None",
                RepeatMode::All => "Playlist",
                RepeatMode::One => "Track",
            }
            .into(),
            can_play: track.is_some() || !self.queue.is_empty(),
            can_pause: track.is_some()
                && matches!(
                    self.playback_status.state,
                    PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Paused
                ),
            can_go_next: self.next_queue_index().is_some() || self.repeat == RepeatMode::All,
            can_go_previous: self.queue_index.is_some_and(|index| index > 0),
        }
    }

    pub(super) fn apply_mpris_command(
        &mut self,
        command: crate::mpris::RemoteCommand,
    ) -> Task<Message> {
        use crate::mpris::RemoteCommand;
        match command {
            RemoteCommand::Next => self.update(Message::NextTrack),
            RemoteCommand::Previous => self.update(Message::PrevTrack),
            RemoteCommand::Raise => self.show_window_from_tray_task(),
            RemoteCommand::Quit => self.update(Message::QuitApp),
            RemoteCommand::PlayPause => self.update(Message::TogglePlayback),
            RemoteCommand::Pause
                if matches!(
                    self.playback_status.state,
                    PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
                ) =>
            {
                self.update(Message::TogglePlayback)
            }
            RemoteCommand::Play if self.playback_status.state == PlaybackState::Paused => {
                self.update(Message::TogglePlayback)
            }
            RemoteCommand::Play
                if matches!(
                    self.playback_status.state,
                    PlaybackState::Idle | PlaybackState::Stopped | PlaybackState::Finished
                ) =>
            {
                if let Some(index) = self
                    .queue_index
                    .or_else(|| (!self.queue.is_empty()).then_some(0))
                    && let Some(track) = self.queue.get(index).cloned()
                {
                    self.play_queued_track(track, index)
                } else {
                    Task::none()
                }
            }
            RemoteCommand::Stop => {
                self.playback.stop();
                Task::none()
            }
            RemoteCommand::SeekTo(position) => {
                let mut target = Duration::from_micros(position.max(0) as u64);
                if let Some(duration) = self.playback_status.duration {
                    target = target.min(duration);
                }
                self.seek_playback(target);
                Task::none()
            }
            RemoteCommand::SeekBy(offset) => {
                let current = self.playback_status.position.as_micros() as i128;
                let mut target = (current + offset as i128).max(0) as u128;
                if let Some(duration) = self.playback_status.duration {
                    target = target.min(duration.as_micros());
                }
                self.seek_playback(Duration::from_micros(target.min(u64::MAX as u128) as u64));
                Task::none()
            }
            RemoteCommand::SetVolume(volume) => {
                let value = if volume.is_finite() {
                    volume.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if value == 0.0 {
                    self.preferences.muted = true;
                    self.playback.set_volume(0.0);
                } else {
                    self.preferences.muted = false;
                    self.preferences.volume = value as f32;
                    self.playback.set_volume(value as f32);
                }
                // Desktop clients send bursts of volume changes; save once they settle.
                self.volume_save_generation = self.volume_save_generation.wrapping_add(1);
                let generation = self.volume_save_generation;
                Task::perform(
                    async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        generation
                    },
                    Message::VolumeSaveDue,
                )
            }
            RemoteCommand::SetShuffle(value) => {
                if self.shuffle != value {
                    self.update(Message::ToggleShuffle)
                } else {
                    Task::none()
                }
            }
            RemoteCommand::SetLoopStatus(value) => {
                self.repeat = match value.as_str() {
                    "Track" => RepeatMode::One,
                    "Playlist" => RepeatMode::All,
                    _ => RepeatMode::Off,
                };
                self.preferences.repeat = self.repeat;
                self.persist();
                Task::none()
            }
            RemoteCommand::Pause | RemoteCommand::Play => Task::none(),
        }
    }

    /// Length to keep with a liked track: the playing duration for the current track, otherwise one seen this session.
    pub(super) fn stored_duration_secs(&self, video_id: &str) -> Option<u32> {
        let playing = self
            .playback_status
            .track
            .as_ref()
            .filter(|track| track.video_id == video_id)
            .and(
                self.playback_status
                    .duration
                    .filter(|duration| !duration.is_zero()),
            );
        playing
            .or_else(|| self.track_duration(video_id))
            .and_then(|duration| u32::try_from(duration.as_secs()).ok())
    }

    pub(super) fn track_duration(&self, video_id: &str) -> Option<Duration> {
        if let Some(duration) = self.known_durations.get(video_id) {
            return Some(*duration);
        }
        self.collection.data.as_ref().and_then(|data| {
            data.tracks.iter().find_map(|entry| {
                if entry.track.video_id == video_id {
                    entry.duration
                } else {
                    None
                }
            })
        })
    }

    /// The playback messages of `update_inner`, which routes them here.
    pub(super) fn update_playback(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ShuffleContext(tracks) => {
                if tracks.is_empty() {
                    return Task::none();
                }
                self.shuffle = true;
                let mut shuffled: Vec<Track> = tracks.to_vec();
                shuffled.shuffle(&mut rand::thread_rng());
                if shuffled.len() > 1
                    && let Some(current) = self.playback_status.track.as_ref()
                    && shuffled[0].video_id == current.video_id
                {
                    let swap_with = rand::thread_rng().gen_range(1..shuffled.len());
                    shuffled.swap(0, swap_with);
                }
                return self.update(Message::PlayContext(Arc::from(shuffled), 0));
            }
            Message::RetryRadio => {
                self.radio_attempted_for = None;
                return self.load_radio();
            }
            Message::RadioLoaded { generation, result } => {
                if generation != self.radio_generation {
                    return Task::none();
                }
                self.radio_loading = false;
                self.radio_cancel = None;
                match result {
                    Ok(tracks) => {
                        let cutoff = self.queue_index.unwrap_or(0).saturating_sub(25);
                        while self.queue.len() >= crate::preferences::MAX_QUEUE_TRACKS {
                            let Some(index) = self.queue[..cutoff.min(self.queue.len())]
                                .iter()
                                .position(|track| self.radio_track_ids.contains(&track.video_id))
                            else {
                                break;
                            };
                            let removed = self.queue.remove(index);
                            self.radio_track_ids.remove(&removed.video_id);
                            if let Some(current) = self.queue_index.as_mut()
                                && index < *current
                            {
                                *current -= 1;
                            }
                        }
                        let mut seen: HashSet<_> =
                            self.queue.iter().map(|t| t.video_id.clone()).collect();
                        let mut added = Vec::new();
                        for track in tracks {
                            if self.queue.len() + added.len()
                                >= crate::preferences::MAX_QUEUE_TRACKS
                            {
                                break;
                            }
                            if seen.insert(track.video_id.clone()) {
                                self.radio_track_ids.insert(track.video_id.clone());
                                added.push(track);
                            }
                        }
                        self.queue.extend(added);
                        self.persist();
                        if self.playback_status.state == PlaybackState::Finished
                            && self.autoplay
                            && let Some(i) = self.next_queue_index()
                        {
                            return self.play_queued_track(self.queue[i].clone(), i);
                        }
                        if self.page == Page::NowPlaying {
                            return self.preload_current_page_artwork();
                        }
                    }
                    Err(error) => self.radio_error = Some(error),
                }
            }
            Message::MoveQueue(from, to) => self.move_queue(from, to),
            Message::RemoveFromQueue(index) => self.remove_from_queue(index),
            Message::ClearUpcomingQueue => self.clear_upcoming_queue(),

            Message::SeekTo(seconds) => {
                let audio_seconds = seconds - self.current_lyrics_offset() as f64 / 1000.0;
                self.seek_playback(Duration::from_secs_f64(audio_seconds.max(0.0)));
            }
            Message::ToggleShuffle => {
                self.shuffle = !self.shuffle;
                if self.shuffle {
                    self.shuffle_queue();
                }
                self.preferences.shuffle = self.shuffle;
                self.persist();
            }
            Message::CycleRepeat => {
                self.repeat = match self.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                self.preferences.repeat = self.repeat;
                self.persist();
            }
            Message::ToggleAutoplay => {
                self.autoplay = !self.autoplay;
                if !self.autoplay {
                    if let Some(token) = self.radio_cancel.take() {
                        token.cancel();
                    }
                    self.radio_generation = self.radio_generation.wrapping_add(1);
                    self.radio_loading = false;
                    self.radio_attempted_for = None;
                }
                self.persist();
            }
            Message::TogglePlayback => match self.playback_status.state {
                PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking => {
                    self.playback.pause()
                }
                PlaybackState::Resolving => self.playback.pause(),
                PlaybackState::Paused => self.playback.resume(),
                PlaybackState::Finished | PlaybackState::Stopped | PlaybackState::Idle => {
                    if let Some(index) = self.queue_index
                        && let Some(track) = self.queue.get(index).cloned()
                    {
                        return self.play_queued_track(track, index);
                    }
                }
                _ => {}
            },
            Message::NextTrack => {
                if let Some(next) = self.next_queue_index() {
                    let track = self.queue[next].clone();
                    return self.play_queued_track(track, next);
                } else if self.repeat == RepeatMode::All && !self.queue.is_empty() {
                    let track = self.queue[0].clone();
                    return self.play_queued_track(track, 0);
                }
            }
            Message::PrevTrack => {
                if let Some(prev) = self.prev_queue_index() {
                    // If more than 3s into the song, restart it instead of going back.
                    if self.playback_status.position.as_secs() > 3 && self.queue_index.is_some() {
                        self.seek_playback(Duration::ZERO);
                        self.seek_preview = None;
                    } else {
                        let track = self.queue[prev].clone();
                        return self.play_queued_track(track, prev);
                    }
                } else if self.playback_status.track.is_some() {
                    self.seek_playback(Duration::ZERO);
                    self.seek_preview = None;
                }
            }
            Message::PlayCollection(tracks, index, source) => {
                // PlayContext clears any previous source, so set this one after it.
                let task = self.update(Message::PlayContext(tracks, index));
                self.queue_source = Some(source);
                return task;
            }
            Message::PlayContext(tracks, index) => {
                if tracks.is_empty() || index >= tracks.len() {
                    return Task::none();
                }
                let track = tracks[index].clone();
                self.radio_track_ids.clear();
                self.queue_source = None;
                self.queue_scroll = AbsoluteOffset::default();
                self.queue_notice =
                    (tracks.len() > crate::preferences::MAX_QUEUE_TRACKS).then(|| {
                        format!(
                            "Queue limited to {} entries; the selected track stays first.",
                            crate::preferences::MAX_QUEUE_TRACKS
                        )
                    });
                // Shuffle mode: keep the selected track first and shuffle the rest.
                if self.shuffle && tracks.len() > 2 {
                    let mut rest: Vec<Track> = tracks
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != index)
                        .map(|(_, t)| t.clone())
                        .collect();
                    rest.shuffle(&mut rand::thread_rng());
                    rest.truncate(crate::preferences::MAX_QUEUE_TRACKS.saturating_sub(1));
                    let mut ordered = vec![track.clone()];
                    ordered.extend(rest);
                    self.queue = ordered;
                    self.queue_generation = self.queue_generation.wrapping_add(1);
                    let play = self.play_queued_track(track, 0);
                    return Task::batch([
                        operation::scroll_to("queue-content", AbsoluteOffset::<f32>::default()),
                        play,
                    ]);
                }
                let (queue, queue_index) =
                    bounded_queue_context(&tracks, index, crate::preferences::MAX_QUEUE_TRACKS);
                self.queue = queue;
                self.queue_generation = self.queue_generation.wrapping_add(1);
                let play = self.play_queued_track(track, queue_index);
                return Task::batch([
                    operation::scroll_to("queue-content", AbsoluteOffset::<f32>::default()),
                    play,
                ]);
            }
            Message::SetVolume(volume) => {
                self.preferences.volume = volume;
                if !self.preferences.muted {
                    self.playback.set_volume(volume);
                }
            }
            Message::SeekChanged(position) => self.seek_preview = Some(position),
            Message::SeekReleased => {
                if let Some(position) = self.seek_preview.take() {
                    self.seek_playback(Duration::from_secs_f32(position.max(0.0)));
                }
            }
            Message::PrewarmPlayback => self.playback.prewarm(),
            Message::PlaybackTick => {
                self.karaoke_tick_pending = false;
                return self.update_playback_status(self.playback.status());
            }
            Message::PlaybackStatusChanged(status) => {
                if let (Some(track), Some(duration)) = (&status.track, status.duration)
                    && !duration.is_zero()
                {
                    self.remember_duration(track.video_id.clone(), duration);
                }
                return self.update_playback_status(status);
            }
            Message::MprisCommand(command) => {
                return self.apply_mpris_command(command);
            }
            Message::SelectTrack(track) => {
                // Preserve single-track behaviour for callers without context,
                // but keep the queue consistent so Prev/Next keep working.
                if let Some(index) = self.queue.iter().position(|queued| {
                    queued.video_id == track.video_id && queued.title == track.title
                }) {
                    return self.play_queued_track(track, index);
                }
                self.radio_track_ids.clear();
                self.queue_scroll = AbsoluteOffset::default();
                self.queue_notice = None;
                self.queue_source = None;
                self.queue = vec![track.clone()];
                self.queue_generation = self.queue_generation.wrapping_add(1);
                let play = self.play_queued_track(track, 0);
                return Task::batch([
                    operation::scroll_to("queue-content", AbsoluteOffset::<f32>::default()),
                    play,
                ]);
            }
            Message::PlayNext(track) => {
                self.row_menu = None;
                return self.enqueue(track, true);
            }
            Message::AddToQueue(track) => {
                self.row_menu = None;
                return self.enqueue(track, false);
            }
            _ => unreachable!("update_inner routes only this group here"),
        }
        Task::none()
    }
}
