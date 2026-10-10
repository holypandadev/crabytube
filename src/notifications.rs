//! Track notifications. The session's notification service decides how they look; CrabyTube sends
//! the title, the artist, and the cover when the artwork cache already holds it.

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

/// Sends the notice and returns the id the service assigned. Zero means it was not sent, for
/// example when no notification service is running.
#[cfg(target_os = "linux")]
pub async fn show(notice: TrackNotice) -> u32 {
    match send(&notice).await {
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
}
