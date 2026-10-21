//! A picker for one image from the user's computer: the file chooser portal on Linux, inside the
//! Flatpak and out of it, and the system's Open dialog on Windows and macOS.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

/// The file chosen, or `None` when the user cancelled.
pub type Picked = Result<Option<PathBuf>, String>;

/// Opens the picker for a PNG or JPEG image. Called from the thread that runs the window, as macOS
/// shows its panel from there; the answer comes once the user chooses or cancels.
pub fn pick_image(title: &str) -> Pin<Box<dyn Future<Output = Picked> + Send>> {
    let title = title.to_owned();
    #[cfg(target_os = "linux")]
    {
        Box::pin(portal::open(title))
    }
    #[cfg(windows)]
    {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || windows_dialog::open(&title))
                .await
                .map_err(|error| error.to_string())?
        })
    }
    #[cfg(target_os = "macos")]
    {
        match mac_panel::begin(&title) {
            Ok(answer) => Box::pin(async move {
                answer
                    .await
                    .unwrap_or_else(|_| Err("the Open panel closed without an answer".to_owned()))
            }),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = title;
        Box::pin(async { Err("Choosing a file is not available on this system.".to_owned()) })
    }
}

#[cfg(target_os = "linux")]
mod portal {
    use super::Picked;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    /// The user may browse for a long while.
    const ANSWER_TIMEOUT: Duration = Duration::from_secs(30 * 60);

    /// Gives each request its own handle token, so its answer has a path to listen on.
    static NEXT_TOKEN: AtomicU32 = AtomicU32::new(0);

    /// Asks the FileChooser portal for one PNG or JPEG file. In the Flatpak the portal hands back a
    /// path that the sandbox can read.
    pub(super) async fn open(title: String) -> Picked {
        use futures_util::StreamExt;
        use zbus::zvariant::{OwnedValue, Value};

        let connection = zbus::Connection::session()
            .await
            .map_err(|error| error.to_string())?;
        let sender = connection
            .unique_name()
            .map(|name| name.trim_start_matches(':').replace('.', "_"))
            .ok_or_else(|| "the session bus gave no unique name".to_owned())?;
        let token = format!(
            "crabytube_cover_{}",
            NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
        );
        // The answer comes on a request object named after the sender and the token. Listening
        // before the call means the answer cannot arrive unseen.
        let request_path = format!("{PATH}/request/{sender}/{token}");
        let request = zbus::Proxy::new(
            &connection,
            DESTINATION,
            request_path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .await
        .map_err(|error| error.to_string())?;
        let mut answers = request
            .receive_signal("Response")
            .await
            .map_err(|error| error.to_string())?;

        let filters = vec![(
            "PNG, JPEG",
            vec![
                (0_u32, "*.png"),
                (0, "*.jpg"),
                (0, "*.jpeg"),
                (1, "image/png"),
                (1, "image/jpeg"),
            ],
        )];
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        options.insert("modal", Value::from(true));
        options.insert("multiple", Value::from(false));
        options.insert("filters", Value::from(filters));
        connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some("org.freedesktop.portal.FileChooser"),
                "OpenFile",
                &("", title.as_str(), options),
            )
            .await
            .map_err(|error| error.to_string())?;

        let answer = tokio::time::timeout(ANSWER_TIMEOUT, answers.next())
            .await
            .map_err(|_| "the file chooser did not answer".to_owned())?
            .ok_or_else(|| "the file chooser closed the request".to_owned())?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = answer
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        match code {
            0 => {
                let uris = results
                    .get("uris")
                    .and_then(|value| value.try_clone().ok())
                    .and_then(|value| Vec::<String>::try_from(value).ok())
                    .unwrap_or_default();
                Ok(uris.first().and_then(|uri| file_path(uri)))
            }
            1 => Ok(None),
            other => Err(format!("the file chooser failed with code {other}")),
        }
    }

    /// The path of a `file://` URI, with its percent escapes decoded.
    pub(super) fn file_path(uri: &str) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        let rest = uri.strip_prefix("file://")?;
        // A host before the path, as in file://localhost/..., is skipped.
        let path = &rest[rest.find('/')?..];
        let bytes = path.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%'
                && let Some(hex) = path.get(index + 1..index + 3)
                && let Ok(byte) = u8::from_str_radix(hex, 16)
            {
                decoded.push(byte);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
    }
}

#[cfg(windows)]
mod windows_dialog {
    use super::Picked;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    /// Shows the system's Open dialog for one PNG or JPEG file. It runs on a thread of its own and
    /// waits there until the user closes it.
    pub(super) fn open(title: &str) -> Picked {
        let filter: Vec<u16> = "PNG, JPEG\0*.png;*.jpg;*.jpeg\0\0".encode_utf16().collect();
        let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let mut file = vec![0_u16; 4096];
        // SAFETY: an all-zero OPENFILENAMEW is a valid empty value; the fields used are set below.
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = file.as_mut_ptr();
        dialog.nMaxFile = file.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
        // SAFETY: the dialog's shell views want COM on their thread; it is released below when this
        // call set it up.
        let com = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _) };
        // SAFETY: `dialog` points at buffers that outlive the call.
        let chosen = unsafe { GetOpenFileNameW(&mut dialog) } != 0;
        if com >= 0 {
            // SAFETY: balances the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        if !chosen {
            // SAFETY: reads the error of the dialog call above, on the same thread.
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(None)
            } else {
                Err(format!("the Open dialog failed with code {code}"))
            };
        }
        let length = file
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file.len());
        Ok(Some(PathBuf::from(OsString::from_wide(&file[..length]))))
    }
}

#[cfg(target_os = "macos")]
mod mac_panel {
    use super::Picked;
    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::NSString;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    /// Shows the Open panel, which does not block the window, and returns where its answer comes.
    pub(super) fn begin(title: &str) -> Result<oneshot::Receiver<Picked>, String> {
        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| "the Open panel must be shown from the main thread".to_owned())?;
        let (sender, receiver) = oneshot::channel();
        let panel = NSOpenPanel::openPanel(main_thread);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setMessage(Some(&NSString::from_str(title)));
        let sender = Mutex::new(Some(sender));
        let answered = panel.clone();
        let handler = RcBlock::new(move |response: NSModalResponse| {
            let picked = (response == NSModalResponseOK)
                .then(|| answered.URL())
                .flatten()
                .and_then(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()));
            if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
                let _ = sender.send(Ok(picked));
            }
        });
        panel.beginWithCompletionHandler(&handler);
        Ok(receiver)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::portal::file_path;
    use std::path::Path;

    #[test]
    fn a_file_uri_becomes_its_path_with_escapes_decoded() {
        assert_eq!(
            file_path("file:///home/user/My%20Pictures/caf%C3%A9.png").as_deref(),
            Some(Path::new("/home/user/My Pictures/café.png"))
        );
        assert_eq!(
            file_path("file://localhost/tmp/a.jpg").as_deref(),
            Some(Path::new("/tmp/a.jpg"))
        );
        // A stray percent sign stays as it is.
        assert_eq!(
            file_path("file:///tmp/100%.png").as_deref(),
            Some(Path::new("/tmp/100%.png"))
        );
        assert_eq!(file_path("https://example.com/a.png"), None);
    }
}
