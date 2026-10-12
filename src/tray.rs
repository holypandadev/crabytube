//! Linux StatusNotifier tray integration.

use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use futures_util::{StreamExt, stream, stream::BoxStream};
use ksni::{
    Tray, TrayMethods,
    menu::{MenuItem, StandardItem},
};
use tokio::sync::{Mutex, mpsc::UnboundedReceiver, mpsc::UnboundedSender};

const WATCHER_UNKNOWN: u8 = 0;
const WATCHER_ONLINE: u8 = 1;
const WATCHER_OFFLINE: u8 = 2;

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
    sender: UnboundedSender<TrayCommand>,
    handle: Arc<std::sync::Mutex<Option<ksni::Handle<CrabyTray>>>>,
}

impl TrayRemote {
    pub fn start() -> Self {
        static NEXT_COMMAND_STREAM_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            commands: CommandReceiver {
                id: NEXT_COMMAND_STREAM_ID.fetch_add(1, Ordering::Relaxed),
                receiver: Arc::new(Mutex::new(receiver)),
            },
            sender,
            handle: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    pub fn start_service(
        &self,
    ) -> impl std::future::Future<Output = Result<bool, String>> + Send + 'static {
        let sender = self.sender.clone();
        let handle_slot = Arc::clone(&self.handle);
        async move {
            let watcher_state = Arc::new(AtomicU8::new(WATCHER_UNKNOWN));
            let tray = CrabyTray {
                sender: sender.clone(),
                watcher_state: Arc::clone(&watcher_state),
                language: crate::i18n::current_language(),
            };
            let handle = tray
                .assume_sni_available(true)
                .disable_dbus_name(disables_item_name(crate::flatpak::in_flatpak()))
                .spawn()
                .await
                .map_err(|error| error.to_string())?;

            if watcher_state
                .compare_exchange(
                    WATCHER_UNKNOWN,
                    WATCHER_ONLINE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                let _ = sender.send(TrayCommand::AvailabilityChanged(true));
            }
            let available = watcher_state.load(Ordering::Acquire) == WATCHER_ONLINE;
            *handle_slot
                .lock()
                .map_err(|_| "System tray handle lock is unavailable.".to_owned())? = Some(handle);
            Ok(available)
        }
    }
}

/// The app icon as ARGB32 at the sizes tray hosts ask for. Decoded once.
fn logo_icons() -> Vec<ksni::Icon> {
    static LOGO: std::sync::OnceLock<Vec<ksni::Icon>> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        let Ok(logo) = image::load_from_memory(include_bytes!("../assets/app-icon-128.png")) else {
            return Vec::new();
        };
        [22_u32, 32, 48]
            .into_iter()
            .map(|size| {
                let rgba = logo
                    .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                    .to_rgba8();
                // The tray protocol wants network byte order: alpha, red, green, blue.
                let data = rgba
                    .pixels()
                    .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
                    .collect();
                ksni::Icon {
                    width: size as i32,
                    height: size as i32,
                    data,
                }
            })
            .collect()
    })
    .clone()
}

struct CrabyTray {
    sender: UnboundedSender<TrayCommand>,
    watcher_state: Arc<AtomicU8>,
    language: crate::preferences::UiLanguage,
}

impl CrabyTray {
    fn item(&self, label: &str, icon_name: &str, command: TrayCommand) -> MenuItem<Self> {
        let sender = self.sender.clone();
        StandardItem {
            label: label.into(),
            icon_name: icon_name.into(),
            activate: Box::new(move |_| {
                let _ = sender.send(command.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

/// Whether the item leaves out its well-known `org.kde.StatusNotifierItem-*` name and registers
/// under its unique name only. A Flatpak sandbox may not claim that name. Outside one, the item
/// keeps the name it has always had.
fn disables_item_name(in_flatpak: bool) -> bool {
    in_flatpak
}

impl Tray for CrabyTray {
    fn id(&self) -> String {
        "crabytube".into()
    }

    fn title(&self) -> String {
        "CrabyTube".into()
    }

    fn icon_name(&self) -> String {
        // Empty, so the host draws the pixmap below instead of a generic themed icon.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        logo_icons()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCommand::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let labels = [
            "Open CrabyTube",
            "Hide to tray",
            "Play / Pause",
            "Next track",
            "Previous track",
            "Quit",
        ]
        .map(|label| crate::i18n::translate_static(self.language, label));
        vec![
            self.item(labels[0], "window", TrayCommand::ShowWindow),
            self.item(labels[1], "go-down", TrayCommand::HideWindow),
            MenuItem::Separator,
            self.item(
                labels[2],
                "media-playback-start",
                TrayCommand::TogglePlayback,
            ),
            self.item(labels[3], "media-skip-forward", TrayCommand::NextTrack),
            self.item(labels[4], "media-skip-backward", TrayCommand::PreviousTrack),
            MenuItem::Separator,
            self.item(labels[5], "application-exit", TrayCommand::Quit),
        ]
    }

    fn watcher_online(&self) {
        self.watcher_state.store(WATCHER_ONLINE, Ordering::Release);
        let _ = self.sender.send(TrayCommand::AvailabilityChanged(true));
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.watcher_state.store(WATCHER_OFFLINE, Ordering::Release);
        let _ = self.sender.send(TrayCommand::AvailabilityChanged(false));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_item_keeps_its_bus_name_outside_flatpak() {
        assert!(!disables_item_name(false));
        assert!(disables_item_name(true));
    }

    #[tokio::test]
    async fn tray_command_stream_delivers_actions_without_polling() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let receiver = CommandReceiver {
            id: 0,
            receiver: Arc::new(Mutex::new(receiver)),
        };
        let mut events = Box::pin(command_stream(&receiver));
        sender.send(TrayCommand::ShowWindow).unwrap();
        assert_eq!(events.next().await, Some(TrayCommand::ShowWindow));
    }

    #[test]
    fn default_tray_item_exposes_recovery_and_playback_actions() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let tray = CrabyTray {
            sender,
            watcher_state: Arc::new(AtomicU8::new(WATCHER_ONLINE)),
            language: crate::preferences::UiLanguage::English,
        };
        assert_eq!(tray.id(), "crabytube");
        assert!(tray.icon_name().is_empty());
        let pixmaps = tray.icon_pixmap();
        assert_eq!(pixmaps.len(), 3);
        for icon in &pixmaps {
            assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
        }
        let mut menu = tray.menu();
        let mut tray_state = CrabyTray {
            sender: tray.sender.clone(),
            watcher_state: Arc::clone(&tray.watcher_state),
            language: tray.language,
        };
        assert_eq!(menu.len(), 8);
        for (index, expected) in [
            (0, TrayCommand::ShowWindow),
            (1, TrayCommand::HideWindow),
            (3, TrayCommand::TogglePlayback),
            (4, TrayCommand::NextTrack),
            (5, TrayCommand::PreviousTrack),
            (7, TrayCommand::Quit),
        ] {
            let MenuItem::Standard(item) = &mut menu[index] else {
                panic!("tray action should be a standard menu item")
            };
            (item.activate)(&mut tray_state);
            assert_eq!(receiver.try_recv().unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn isolated_dbus_tray_registers_and_activation_reaches_the_app() {
        if std::env::var_os("CRABYTUBE_TEST_TRAY_BUS").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        use std::sync::Mutex as StdMutex;

        #[derive(Clone, Default)]
        struct Watcher {
            registered: Arc<StdMutex<Vec<String>>>,
        }

        #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
        impl Watcher {
            fn register_status_notifier_item(&self, service: &str) -> zbus::fdo::Result<()> {
                self.registered
                    .lock()
                    .map_err(|_| zbus::fdo::Error::Failed("watcher state poisoned".into()))?
                    .push(service.into());
                Ok(())
            }
        }

        let watcher = Watcher::default();
        let registered = Arc::clone(&watcher.registered);
        let _watcher_connection = zbus::connection::Builder::session()
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at("/StatusNotifierWatcher", watcher)
            .unwrap()
            .build()
            .await
            .unwrap();

        let remote = TrayRemote::start();
        let receiver = remote.command_receiver();
        assert!(remote.start_service().await.unwrap());
        assert_eq!(
            receiver.receiver.lock().await.recv().await,
            Some(TrayCommand::AvailabilityChanged(true))
        );
        let service_name = registered.lock().unwrap()[0].clone();
        let connection = zbus::Connection::session().await.unwrap();
        let item = zbus::Proxy::new(
            &connection,
            service_name,
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
        )
        .await
        .unwrap();
        item.call::<_, _, ()>("Activate", &(0i32, 0i32))
            .await
            .unwrap();
        let command = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            receiver.receiver.lock().await.recv(),
        )
        .await
        .unwrap();
        assert_eq!(command, Some(TrayCommand::ShowWindow));
    }
}
