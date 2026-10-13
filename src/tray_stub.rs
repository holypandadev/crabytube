//! Platform stub for builds without a native system tray adapter.

use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

use futures_util::{StreamExt, stream, stream::BoxStream};
use tokio::sync::{Mutex, mpsc::UnboundedReceiver, mpsc::UnboundedSender};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    ShowWindow,
    HideWindow,
    TogglePlayback,
    NextTrack,
    PreviousTrack,
    Quit,
    AvailabilityChanged(bool),
}

#[derive(Clone)]
pub struct CommandReceiver {
    id: u64,
    receiver: Arc<Mutex<UnboundedReceiver<TrayCommand>>>,
}

impl PartialEq for CommandReceiver {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for CommandReceiver {}
impl Hash for CommandReceiver {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

pub fn command_stream(receiver: &CommandReceiver) -> BoxStream<'static, TrayCommand> {
    stream::unfold(receiver.clone(), |receiver| async move {
        let command = receiver.receiver.lock().await.recv().await?;
        Some((command, receiver))
    })
    .boxed()
}

pub struct TrayRemote {
    commands: CommandReceiver,
    _sender: UnboundedSender<TrayCommand>,
}

impl TrayRemote {
    pub fn start() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            commands: CommandReceiver {
                id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                receiver: Arc::new(Mutex::new(receiver)),
            },
            _sender: sender,
        }
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    /// Same signature as the Linux tray, which has a menu to translate.
    #[allow(clippy::manual_async_fn)]
    pub fn set_language(
        &self,
        _language: crate::preferences::UiLanguage,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        async {}
    }

    // Same signature as the Linux tray: the future must not borrow `self`.
    #[allow(clippy::manual_async_fn)]
    pub fn start_service(
        &self,
    ) -> impl std::future::Future<Output = Result<bool, String>> + Send + 'static {
        async { Err("System tray support is unavailable on this platform.".into()) }
    }
}
