//! Start at login from inside a Flatpak. The Background portal asks the user and writes the
//! autostart entry on the host: the sandbox's own config folder is not read by the session.

/// What the portal shows the user as the reason for running in the background.
#[cfg(target_os = "linux")]
const REASON: &str = "Play music in the background.";

/// The command the portal starts at login, as the Flatpak installs it.
#[cfg(target_os = "linux")]
const COMMAND: &str = "crabytube";

/// Gives each request its own handle token, so its answer has a path to listen on.
#[cfg(target_os = "linux")]
static NEXT_TOKEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Asks the Background portal to start CrabyTube at login (`enabled`) or to stop it. Resolves to
/// whether the portal grants that, once the user has answered its prompt.
#[cfg(target_os = "linux")]
pub async fn request_autostart(enabled: bool) -> Result<bool, String> {
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use zbus::zvariant::{OwnedValue, Value};

    const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    // The user may take a while to answer the prompt.
    const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

    let connection = zbus::Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    let sender = connection
        .unique_name()
        .map(|name| name.trim_start_matches(':').replace('.', "_"))
        .ok_or_else(|| "the session bus gave no unique name".to_owned())?;
    let token = format!(
        "crabytube_autostart_{}",
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    );
    // The answer comes on a request object named after the sender and the token. Listening before
    // the call means the answer cannot arrive unseen.
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

    let reason = crate::i18n::translate(crate::i18n::current_language(), REASON);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("reason", Value::from(reason.as_ref()));
    options.insert("autostart", Value::from(enabled));
    options.insert("commandline", Value::from(vec![COMMAND]));
    options.insert("dbus-activatable", Value::from(false));
    connection
        .call_method(
            Some(DESTINATION),
            PATH,
            Some("org.freedesktop.portal.Background"),
            "RequestBackground",
            &("", options),
        )
        .await
        .map_err(|error| error.to_string())?;

    let answer = tokio::time::timeout(ANSWER_TIMEOUT, answers.next())
        .await
        .map_err(|_| "the portal did not answer".to_owned())?
        .ok_or_else(|| "the portal closed the request".to_owned())?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = answer
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    let autostart = results
        .get("autostart")
        .and_then(|value| bool::try_from(value.clone()).ok())
        .unwrap_or(false);
    granted(code, autostart)
}

/// Not a Linux service: nothing is started at login from a sandbox there.
#[cfg(not(target_os = "linux"))]
pub async fn request_autostart(_enabled: bool) -> Result<bool, String> {
    Err("The Background portal is available on Linux only.".to_owned())
}

/// The portal's answer: 0 is success and carries the autostart flag it reports, 1 means the user
/// refused, and any other code is an error.
#[cfg(any(target_os = "linux", test))]
fn granted(code: u32, autostart: bool) -> Result<bool, String> {
    match code {
        0 => Ok(autostart),
        1 => Ok(false),
        other => Err(format!("the portal failed with code {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::granted;

    #[test]
    fn a_successful_answer_grants_what_the_portal_reports() {
        assert_eq!(granted(0, true), Ok(true));
        assert_eq!(granted(0, false), Ok(false));
    }

    #[test]
    fn a_refusal_is_not_an_error_and_grants_nothing() {
        assert_eq!(granted(1, false), Ok(false));
    }

    #[test]
    fn any_other_code_is_an_error() {
        assert!(granted(2, false).is_err());
    }

    /// Asks the desktop's real portal to stop autostart, which changes nothing while it is off. Run by
    /// hand with `cargo test -- --ignored portal_answers_a_disable_request`.
    #[cfg(target_os = "linux")]
    #[ignore = "talks to the desktop's Background portal"]
    #[tokio::test]
    async fn portal_answers_a_disable_request() {
        let answer = super::request_autostart(false).await;
        assert!(matches!(answer, Ok(false)), "{answer:?}");
    }
}
