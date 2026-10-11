//! Keeps one CrabyTube running per user. A second launch asks the running one to show its window
//! and exits, so two processes never write the same state file or open two audio streams.
//!
//! The first process holds an exclusive lock on a file in the config folder; the operating system
//! releases it when the process ends, however it ends. On Unix the first process also listens on
//! a socket next to the lock, which a later launch connects to as its request.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use futures_util::{StreamExt, stream, stream::BoxStream};
use tokio::sync::mpsc::UnboundedReceiver;

const LOCK_FILE: &str = "instance.lock";
#[cfg(unix)]
const SOCKET_FILE: &str = "instance.sock";

/// What the first process keeps for as long as it runs.
struct Held {
    _lock: File,
    requests: Option<UnboundedReceiver<()>>,
}

static HELD: OnceLock<Mutex<Held>> = OnceLock::new();

/// Returns whether this process should go on starting. `false` means another instance is running
/// and has been asked to show its window. Any problem with the lock lets the app start: a failure
/// here must never keep the app from opening.
pub fn claim() -> bool {
    let Some(folder) = folder() else {
        return true;
    };
    match claim_in(&folder) {
        Claim::Duplicate => {
            notify_running(&folder);
            false
        }
        Claim::First(held) => {
            let _ = HELD.set(Mutex::new(held));
            true
        }
        Claim::Unavailable => true,
    }
}

/// Requests from later launches to show the window, one item each. Takes the receiver, so only
/// the first call yields anything.
pub fn show_requests() -> BoxStream<'static, ()> {
    let receiver = HELD
        .get()
        .and_then(|held| held.lock().ok())
        .and_then(|mut held| held.requests.take());
    match receiver {
        Some(receiver) => stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|()| ((), receiver))
        })
        .boxed(),
        None => stream::pending().boxed(),
    }
}

enum Claim {
    First(Held),
    Duplicate,
    Unavailable,
}

fn folder() -> Option<PathBuf> {
    crate::preferences::state_path()
        .parent()
        .map(Path::to_path_buf)
}

fn claim_in(folder: &Path) -> Claim {
    if crate::storage::create_private_dir_all(folder).is_err() {
        return Claim::Unavailable;
    }
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(folder.join(LOCK_FILE))
    else {
        return Claim::Unavailable;
    };
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Claim::Duplicate,
        Err(std::fs::TryLockError::Error(_)) => return Claim::Unavailable,
    }
    #[cfg(unix)]
    let requests = listen(folder);
    #[cfg(not(unix))]
    let requests = None;
    Claim::First(Held {
        _lock: lock,
        requests,
    })
}

/// Starts the thread that turns each connection to the socket into one request. The lock is held,
/// so a socket file that is already there belongs to a process that has ended.
#[cfg(unix)]
fn listen(folder: &Path) -> Option<UnboundedReceiver<()>> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let path = folder.join(SOCKET_FILE);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("crabytube-instance".into())
        .spawn(move || forward_connections(listener, sender))
        .ok()?;
    Some(receiver)
}

#[cfg(unix)]
fn forward_connections(
    listener: std::os::unix::net::UnixListener,
    sender: tokio::sync::mpsc::UnboundedSender<()>,
) {
    for connection in listener.incoming() {
        if connection.is_ok() && sender.send(()).is_err() {
            break;
        }
    }
}

#[cfg(unix)]
fn notify_running(folder: &Path) {
    let _ = std::os::unix::net::UnixStream::connect(folder.join(SOCKET_FILE));
}

#[cfg(not(unix))]
fn notify_running(_folder: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cti-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_second_claim_while_the_first_is_held_is_a_duplicate() {
        let dir = scratch("dup");
        let first = claim_in(&dir);
        assert!(matches!(first, Claim::First(_)));
        assert!(matches!(claim_in(&dir), Claim::Duplicate));
        // The lock goes with the process that held it.
        drop(first);
        assert!(matches!(claim_in(&dir), Claim::First(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_later_launch_reaches_the_first_one() {
        let dir = scratch("wake");
        let Claim::First(mut held) = claim_in(&dir) else {
            panic!("the first claim must succeed");
        };
        let mut requests = held.requests.take().expect("a listener");
        notify_running(&dir);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let received = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv()).await
        });
        assert_eq!(received, Ok(Some(())));
        let _ = std::fs::remove_dir_all(dir);
    }
}
