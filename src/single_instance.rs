//! Keeps one CrabyTube running per user. A second launch asks the running one to show its window
//! and exits, so two processes never write the same state file or open two audio streams.
//!
//! The first process holds an exclusive lock on a file in the config folder; the operating system
//! releases it when the process ends, however it ends. On Unix the first process also listens on
//! a socket next to the lock, which a later launch connects to as its request. Elsewhere it
//! listens on a loopback port, recorded in a file next to the lock, and the request is the same.

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
///
/// `wait` is how long a running instance is given to end first, as the one that started this
/// process after an update does: it quits once it has started it.
pub fn claim(wait: std::time::Duration) -> bool {
    let Some(folder) = folder() else {
        return true;
    };
    match claim_waiting(&folder, wait) {
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

/// `claim_in`, asked again for as long as `wait` while another instance holds the lock.
fn claim_waiting(folder: &Path, wait: std::time::Duration) -> Claim {
    let started = std::time::Instant::now();
    let mut claim = claim_in(folder);
    while matches!(claim, Claim::Duplicate) && started.elapsed() < wait {
        std::thread::sleep(std::time::Duration::from_millis(200));
        claim = claim_in(folder);
    }
    claim
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
    let requests = listen(folder);
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
fn notify_running(folder: &Path) {
    loopback::notify(folder);
}

/// Platforms without Unix sockets listen on a loopback port instead.
#[cfg(not(unix))]
fn listen(folder: &Path) -> Option<UnboundedReceiver<()>> {
    loopback::listen(folder)
}

/// The transport for platforms without Unix sockets. The first instance listens on the loopback
/// address, on a port the system picks, and writes that port to a file beside the lock. A later
/// launch connects to the port. The connection is the whole request: it is dropped unread, so no
/// data is exchanged. Only processes on this machine can reach the port.
#[cfg_attr(unix, allow(dead_code))]
mod loopback {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::path::Path;
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

    pub(super) const PORT_FILE: &str = "instance.port";
    const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

    /// Starts the thread that turns each connection into one request, and records the port.
    pub(super) fn listen(folder: &Path) -> Option<UnboundedReceiver<()>> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).ok()?;
        let port = listener.local_addr().ok()?.port();
        write_port(folder, port).ok()?;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("crabytube-instance".into())
            .spawn(move || forward_connections(listener, sender))
            .ok()?;
        Some(receiver)
    }

    /// Asks a running instance to show its window. Failures are ignored, as on Unix.
    pub(super) fn notify(folder: &Path) {
        let Some(port) = read_port(folder) else {
            return;
        };
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let _ = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT);
    }

    fn forward_connections(listener: TcpListener, sender: UnboundedSender<()>) {
        for connection in listener.incoming() {
            // The connection is the request. It is dropped here without reading from it.
            if connection.is_ok() && sender.send(()).is_err() {
                break;
            }
        }
    }

    fn write_port(folder: &Path, port: u16) -> std::io::Result<()> {
        let temporary = folder.join(format!("{PORT_FILE}.tmp"));
        std::fs::write(&temporary, port.to_string())?;
        crate::preferences::rename_over(&temporary, &folder.join(PORT_FILE))
    }

    fn read_port(folder: &Path) -> Option<u16> {
        std::fs::read_to_string(folder.join(PORT_FILE))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cti-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_claim_that_waits_gets_the_lock_once_the_first_instance_ends() {
        let dir = scratch("waiting");
        let first = claim_in(&dir);
        assert!(matches!(first, Claim::First(_)));
        // Without waiting, the second launch is a duplicate.
        assert!(matches!(
            claim_waiting(&dir, std::time::Duration::ZERO),
            Claim::Duplicate
        ));
        // The first one ends while the second waits.
        let ending = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            drop(first);
        });
        assert!(matches!(
            claim_waiting(&dir, std::time::Duration::from_secs(10)),
            Claim::First(_)
        ));
        ending.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn the_loopback_listener_records_its_port_beside_the_lock() {
        let dir = scratch("port-file");
        std::fs::create_dir_all(&dir).unwrap();
        let _requests = loopback::listen(&dir).expect("a loopback listener");
        let recorded = std::fs::read_to_string(dir.join(loopback::PORT_FILE)).unwrap();
        assert!(
            recorded.trim().parse::<u16>().is_ok_and(|port| port != 0),
            "the port file holds {recorded:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_later_launch_reaches_the_first_one_over_loopback() {
        let dir = scratch("loopback");
        std::fs::create_dir_all(&dir).unwrap();
        let mut requests = loopback::listen(&dir).expect("a loopback listener");
        loopback::notify(&dir);
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

    #[test]
    fn a_garbled_or_dead_port_file_is_ignored_without_blocking() {
        let dir = scratch("stale-port");
        std::fs::create_dir_all(&dir).unwrap();
        loopback::notify(&dir);
        std::fs::write(dir.join(loopback::PORT_FILE), "not a port").unwrap();
        loopback::notify(&dir);
        // Port 1 has no listener, so the connection is refused at once.
        std::fs::write(dir.join(loopback::PORT_FILE), "1").unwrap();
        loopback::notify(&dir);
        let _ = std::fs::remove_dir_all(dir);
    }
}
