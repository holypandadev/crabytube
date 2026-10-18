//! The tray icon on Windows (the notification area) and macOS (the menu bar), through `tray-icon`.
//!
//! The icon and its menu belong to the main thread, which runs the window's event loop: they are
//! made there once the first window has opened, kept in a thread-local, and removed before the app
//! exits. Their clicks reach the app as `TrayCommand`s, as from the Linux tray.

use std::{
    cell::RefCell,
    hash::{Hash, Hasher},
    sync::Arc,
};

use futures_util::{StreamExt, stream, stream::BoxStream};
use tokio::sync::{Mutex, mpsc::UnboundedReceiver, mpsc::UnboundedSender};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};

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

/// The menu, in order, with the command each entry sends. `None` is a separator.
const ENTRIES: [Option<(&str, TrayCommand)>; 8] = [
    Some(("Open CrabyTube", TrayCommand::ShowWindow)),
    Some(("Hide to tray", TrayCommand::HideWindow)),
    None,
    Some(("Play / Pause", TrayCommand::TogglePlayback)),
    Some(("Next track", TrayCommand::NextTrack)),
    Some(("Previous track", TrayCommand::PreviousTrack)),
    None,
    Some(("Quit", TrayCommand::Quit)),
];

/// What lives on the main thread while the icon is shown.
struct Native {
    icon: TrayIcon,
    /// Each entry with the English text it translates.
    items: Vec<(MenuItem, &'static str)>,
}

thread_local! {
    static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) };
}

pub struct TrayRemote {
    commands: CommandReceiver,
    sender: UnboundedSender<TrayCommand>,
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
            sender,
        }
    }

    pub fn command_receiver(&self) -> CommandReceiver {
        self.commands.clone()
    }

    /// Writes the menu in `language`. Called from the interface, on the main thread, so the
    /// entries change at once; the future is there for the same call as on Linux.
    pub fn set_language(
        &self,
        language: crate::preferences::UiLanguage,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        NATIVE.with(|native| {
            if let Some(native) = native.borrow().as_ref() {
                for (item, label) in &native.items {
                    item.set_text(crate::i18n::translate_static(language, label));
                }
            }
        });
        async {}
    }

    /// Shows the icon, once. Must run on the main thread with its event loop running, which is
    /// the case when the first window has opened. `true` when the icon is shown.
    pub fn attach(&self) -> Result<bool, String> {
        // Tests drive the interface from their own threads, where macOS forbids making a menu.
        if cfg!(test) {
            return Err("the tray icon is not made in tests".into());
        }
        if NATIVE.with(|native| native.borrow().is_some()) {
            return Ok(true);
        }
        #[cfg(target_os = "macos")]
        answer_reopen(self.sender.clone());
        let language = crate::i18n::current_language();
        let menu = Menu::new();
        let mut items = Vec::new();
        let mut commands: Vec<(MenuId, TrayCommand)> = Vec::new();
        for entry in &ENTRIES {
            match entry {
                Some((label, command)) => {
                    let item =
                        MenuItem::new(crate::i18n::translate_static(language, label), true, None);
                    menu.append(&item).map_err(|error| error.to_string())?;
                    commands.push((item.id().clone(), command.clone()));
                    items.push((item, *label));
                }
                None => menu
                    .append(&PredefinedMenuItem::separator())
                    .map_err(|error| error.to_string())?,
            }
        }
        let sender = self.sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, command)) = commands.iter().find(|(id, _)| *id == event.id) {
                let _ = sender.send(command.clone());
            }
        }));
        // On Windows a click on the icon opens the window and the right button shows the menu. On
        // macOS a click shows the menu, as for every icon in the menu bar.
        let sender = self.sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if cfg!(windows)
                && let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
            {
                let _ = sender.send(TrayCommand::ShowWindow);
            }
        }));
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("CrabyTube")
            .with_menu_on_left_click(cfg!(target_os = "macos"));
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(menu_bar_icon()?);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(notification_area_icon()?);
        let icon = builder.build().map_err(|error| error.to_string())?;
        NATIVE.with(|native| *native.borrow_mut() = Some(Native { icon, items }));
        Ok(true)
    }

    /// Removes the icon. Called before the app exits: Windows leaves the icon of a process that
    /// ended in the notification area until the pointer passes over it.
    pub fn detach(&self) {
        if let Some(native) = NATIVE.with(|native| native.borrow_mut().take()) {
            let _ = native.icon.set_visible(false);
        }
    }
}

/// Puts the app in the Dock and the app switcher, or takes it out while its window is hidden in
/// the menu bar, as a window closed to the tray leaves the taskbar on Linux and Windows. Must run
/// on the main thread.
#[cfg(target_os = "macos")]
pub fn show_in_dock(shown: bool) {
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Bool},
    };
    /// `NSApplicationActivationPolicyRegular` and `NSApplicationActivationPolicyAccessory`.
    const REGULAR: isize = 0;
    const ACCESSORY: isize = 1;
    // Tests drive the interface from their own threads, where AppKit must not be touched.
    if cfg!(test) {
        return;
    }
    // SAFETY: called from the interface, which runs on the main thread once the application has
    // started; both messages take and return plain values.
    unsafe {
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: Bool =
            msg_send![application, setActivationPolicy: if shown { REGULAR } else { ACCESSORY }];
        if shown {
            // Back in the Dock, the app comes forward with its window.
            let _: () = msg_send![application, activateIgnoringOtherApps: Bool::YES];
        }
    }
}

/// A click on the Dock icon, or opening the app again from the Finder or Spotlight, asks the app
/// delegate to reopen. The delegate winit sets does not answer, so a window hidden to the menu bar
/// would stay hidden: the answer is added to its class, and shows the window.
#[cfg(target_os = "macos")]
fn answer_reopen(sender: UnboundedSender<TrayCommand>) {
    use objc2::{
        Encode, class, ffi, msg_send,
        runtime::{AnyObject, Bool, Imp, Sel},
        sel,
    };
    static REOPEN: std::sync::OnceLock<UnboundedSender<TrayCommand>> = std::sync::OnceLock::new();
    extern "C-unwind" fn reopen(
        _delegate: *mut AnyObject,
        _command: Sel,
        _application: *mut AnyObject,
        _has_visible_windows: Bool,
    ) -> Bool {
        // Whatever AppKit counts as visible, the menu bar icon included: showing a window that is
        // already on screen only brings it to the front, as the click does anyway.
        if let Some(sender) = REOPEN.get() {
            let _ = sender.send(TrayCommand::ShowWindow);
        }
        // AppKit then does what it does by default, such as bringing back a minimized window.
        Bool::YES
    }
    if REOPEN.set(sender).is_err() {
        return;
    }
    let Ok(types) = std::ffi::CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING))
    else {
        return;
    };
    // SAFETY: this runs on the main thread once the application has started. The function has the
    // signature `types` names, which is the one AppKit calls
    // `applicationShouldHandleReopen:hasVisibleWindows:` with.
    unsafe {
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![application, delegate];
        if delegate.is_null() {
            return;
        }
        let reopen: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool =
            reopen;
        let imp = std::mem::transmute::<
            extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Bool) -> Bool,
            Imp,
        >(reopen);
        let added = ffi::class_addMethod(
            ffi::object_getClass(delegate).cast_mut(),
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            imp,
            types.as_ptr(),
        );
        if added.as_bool() {
            // AppKit notes which questions a delegate answers when the delegate is set.
            let _: () = msg_send![application, setDelegate: delegate];
        }
    }
}

/// The app icon, as in the Linux tray, at the size the notification area draws.
#[cfg(not(target_os = "macos"))]
fn notification_area_icon() -> Result<Icon, String> {
    let logo = image::load_from_memory(include_bytes!("../assets/app-icon-128.png"))
        .map_err(|error| error.to_string())?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (width, height) = logo.dimensions();
    Icon::from_rgba(logo.into_raw(), width, height).map_err(|error| error.to_string())
}

/// The crab's outline, which macOS draws in the colour of the menu bar's text, light or dark.
#[cfg(target_os = "macos")]
fn menu_bar_icon() -> Result<Icon, String> {
    let outline = image::load_from_memory(include_bytes!("../assets/tray-template.png"))
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = outline.dimensions();
    Icon::from_rgba(outline.into_raw(), width, height).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_offers_the_same_actions_as_on_linux() {
        let commands: Vec<_> = ENTRIES
            .iter()
            .flatten()
            .map(|(_, command)| command.clone())
            .collect();
        assert_eq!(
            commands,
            [
                TrayCommand::ShowWindow,
                TrayCommand::HideWindow,
                TrayCommand::TogglePlayback,
                TrayCommand::NextTrack,
                TrayCommand::PreviousTrack,
                TrayCommand::Quit,
            ]
        );
        for (label, _) in ENTRIES.iter().flatten() {
            // Every entry has its Spanish text, as the test of the catalogue checks for all.
            assert_ne!(
                crate::i18n::translate_static(crate::preferences::UiLanguage::Spanish, label),
                *label
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_notification_area_icon_decodes() {
        assert!(notification_area_icon().is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_menu_bar_icon_decodes() {
        assert!(menu_bar_icon().is_ok());
    }
}
