//! Track notifications. The session's notification service decides how they look; CrabyTube sends
//! the title, the artist, and the cover when the artwork cache already holds it. Inside a Flatpak
//! sandbox they go through the notification portal, the one route the sandbox has to the session.

use std::path::PathBuf;

/// What one track notification shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackNotice {
    /// The notification this one replaces, so tracks do not pile up. Zero when there is none.
    pub replaces_id: u32,
    pub title: String,
    pub artist: String,
    /// A cover already on disk. Nothing is downloaded for a notification.
    pub cover: Option<PathBuf>,
}

/// How long a track notification stays on screen, in milliseconds.
#[cfg(target_os = "linux")]
const EXPIRE_MS: i32 = 5_000;

/// The portal replaces a notification that has the same id, so track notices do not pile up.
#[cfg(target_os = "linux")]
const PORTAL_NOTICE_ID: &str = "track";

/// The portal returns no id. Any non-zero number tells the caller that the notice was sent.
#[cfg(target_os = "linux")]
const PORTAL_SENT: u32 = 1;

/// The app icon as a Flatpak installs it (see `packaging/linux/flatpak`).
#[cfg(target_os = "linux")]
const FLATPAK_ICON: &str = "dev.holypanda.CrabyTube";

/// Sends the notice and returns the id the service assigned. Through the portal the id is a
/// placeholder, since the portal assigns none. Zero means it was not sent, for example when no
/// notification service is running.
#[cfg(target_os = "linux")]
pub async fn show(notice: TrackNotice) -> u32 {
    let sent = if in_flatpak() {
        send_portal(&notice).await.map(|()| PORTAL_SENT)
    } else {
        send(&notice).await
    };
    match sent {
        Ok(id) => id,
        Err(error) => {
            tracing::debug!(%error, "track notification was not shown");
            0
        }
    }
}

/// Not available yet on macOS and Windows.
#[cfg(not(target_os = "linux"))]
pub async fn show(_notice: TrackNotice) -> u32 {
    0
}

/// The session bus connection, opened on the first notification and reused for the rest.
#[cfg(target_os = "linux")]
static SESSION: std::sync::Mutex<Option<zbus::Connection>> = std::sync::Mutex::new(None);

#[cfg(target_os = "linux")]
async fn session() -> zbus::Result<zbus::Connection> {
    if let Some(connection) = SESSION.lock().ok().and_then(|slot| slot.clone()) {
        return Ok(connection);
    }
    let connection = zbus::Connection::session().await?;
    if let Ok(mut slot) = SESSION.lock() {
        *slot = Some(connection.clone());
    }
    Ok(connection)
}

/// Drops the cached connection after a failed call, so the next notification connects again.
#[cfg(target_os = "linux")]
fn forget_session() {
    if let Ok(mut slot) = SESSION.lock() {
        *slot = None;
    }
}

/// Flatpak places this file at the root of every sandbox.
#[cfg(target_os = "linux")]
fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

#[cfg(target_os = "linux")]
async fn send(notice: &TrackNotice) -> zbus::Result<u32> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    let connection = session().await?;
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    // Lets the server match the notification to `crabytube.desktop`.
    hints.insert("desktop-entry", Value::from("crabytube"));
    if let Some(cover) = &notice.cover {
        hints.insert(
            "image-path",
            Value::from(cover.to_string_lossy().into_owned()),
        );
    }
    let summary = escape_markup(&notice.title);
    let body = escape_markup(&notice.artist);
    let reply = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "CrabyTube",
                notice.replaces_id,
                // The installed icon name (see `packaging/linux`).
                "crabytube",
                summary.as_str(),
                body.as_str(),
                Vec::<&str>::new(),
                hints,
                EXPIRE_MS,
            ),
        )
        .await
        .inspect_err(|_| forget_session())?;
    reply.body().deserialize::<u32>()
}

/// Sends the notice through the notification portal. The portal shows plain text, so the title and
/// the artist go as written. The cover goes as bytes, because the portal cannot open a path inside
/// the sandbox's cache.
#[cfg(target_os = "linux")]
async fn send_portal(notice: &TrackNotice) -> zbus::Result<()> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    let cover = notice
        .cover
        .as_ref()
        .and_then(|path| std::fs::read(path).ok());
    let connection = session().await?;
    let mut fields: HashMap<&str, Value<'_>> = HashMap::new();
    fields.insert("title", Value::from(notice.title.as_str()));
    fields.insert("body", Value::from(notice.artist.as_str()));
    fields.insert("priority", Value::from("normal"));
    fields.insert("icon", Value::Structure(portal_icon(cover)));
    connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Notification"),
            "AddNotification",
            &(PORTAL_NOTICE_ID, fields),
        )
        .await
        .inspect_err(|_| forget_session())?;
    Ok(())
}

/// The icon the portal shows: the cover when it could be read, otherwise the installed app icon.
#[cfg(target_os = "linux")]
fn portal_icon(cover: Option<Vec<u8>>) -> zbus::zvariant::Structure<'static> {
    use zbus::zvariant::{Structure, Value};

    match cover {
        Some(bytes) => Structure::from(("bytes", Value::from(bytes))),
        None => Structure::from(("themed", Value::from(vec![FLATPAK_ICON]))),
    }
}

/// Notification text may carry a little markup. Escaping keeps titles such as "A & B" as written.
#[cfg(any(target_os = "linux", test))]
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::escape_markup;

    #[test]
    fn titles_keep_their_characters_when_markup_is_escaped() {
        assert_eq!(
            escape_markup("Tom & Jerry <live>"),
            "Tom &amp; Jerry &lt;live&gt;"
        );
        assert_eq!(escape_markup("Plain Song Title"), "Plain Song Title");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_portal_icon_without_a_cover_is_the_installed_app_icon() {
        let icon = super::portal_icon(None);
        assert_eq!(icon.fields()[0], zbus::zvariant::Value::from("themed"));
        // The second field of the (sv) structure is a variant.
        assert_eq!(
            icon.fields()[1],
            zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::from(vec![
                super::FLATPAK_ICON
            ])))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_portal_icon_with_a_cover_carries_the_cover_bytes() {
        let icon = super::portal_icon(Some(vec![0xff, 0xd8, 0xff]));
        assert_eq!(icon.fields()[0], zbus::zvariant::Value::from("bytes"));
        assert_eq!(
            icon.fields()[1],
            zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::from(vec![
                0xff_u8, 0xd8, 0xff
            ])))
        );
    }

    /// Shows a notification on the session bus, so it is run by hand: `cargo test -- --ignored
    /// portal_accepts_a_track_notice`.
    #[cfg(target_os = "linux")]
    #[ignore = "shows a notification on the session bus"]
    #[tokio::test]
    async fn portal_accepts_a_track_notice() {
        let notice = super::TrackNotice {
            replaces_id: 0,
            title: "Portal check & more".into(),
            artist: "CrabyTube".into(),
            cover: None,
        };
        super::send_portal(&notice)
            .await
            .expect("the notification portal accepts the notice");
    }
}
