//! Platform placeholder exposing the same app-facing types without Linux D-Bus.

use futures_util::{stream, stream::BoxStream};
use std::hash::{Hash, Hasher};
use std::time::Duration;

#[derive(Clone, PartialEq, Eq)]
pub struct CommandReceiver;

impl Hash for CommandReceiver {
    fn hash<H: Hasher>(&self, _state: &mut H) {}
}

pub fn command_stream(_receiver: &CommandReceiver) -> BoxStream<'static, RemoteCommand> {
    Box::pin(stream::pending())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    /// The video id of the track, which names it in `mpris:trackid`.
    pub track_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub playback_status: String,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f64,
    pub shuffle: bool,
    pub loop_status: String,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoteCommand {
    Next,
    Previous,
    Pause,
    Play,
    PlayPause,
    Stop,
    SeekBy(i64),
    SeekTo(i64),
    SetVolume(f64),
    SetShuffle(bool),
    SetLoopStatus(String),
    Raise,
    Quit,
}

pub struct MprisRemote {
    commands: CommandReceiver,
}

impl MprisRemote {
    pub fn start() -> Self {
        Self {
            commands: CommandReceiver,
        }
    }
    pub fn update(&self, _state: PlayerState) {}
    pub fn status(&self) -> String {
        "System media controls are unavailable on this platform".into()
    }
    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }
    pub fn drain(&self) -> Vec<RemoteCommand> {
        Vec::new()
    }
}
