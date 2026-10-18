//! Helper processes join one job object on Windows. The job ends them when the app exits, even
//! when the app is killed before its own cleanup runs. Elsewhere the helpers rely on their own
//! deadline and on the parent-death signal (see the `helper-guard` crate). What a helper prints is
//! read here too, up to a limit.

#[cfg(windows)]
mod windows {
    use std::io;
    use std::os::windows::io::RawHandle;
    use std::sync::OnceLock;

    use tracing::warn;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// The job every helper joins. It lives as long as the app runs.
    static HELPERS: OnceLock<Option<KillOnCloseJob>> = OnceLock::new();

    /// A job object whose processes end when its last handle closes.
    pub struct KillOnCloseJob {
        /// The handle as an integer, so that the job can be shared between threads.
        handle: usize,
    }

    impl KillOnCloseJob {
        pub fn new() -> io::Result<Self> {
            // SAFETY: null arguments create an unnamed job with the default security attributes.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // From here on the job closes its handle when it drops, including on the error path below.
            let job = KillOnCloseJob {
                handle: handle as usize,
            };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the information class is the extended limit structure, and `limits` is that
            // structure with its size passed alongside it.
            let set = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if set == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        /// Puts a running process under the job.
        pub fn assign(&self, process: RawHandle) -> io::Result<()> {
            // SAFETY: the job handle stays open while `self` exists, and the caller holds `process`.
            let assigned = unsafe { AssignProcessToJobObject(self.handle as HANDLE, process) };
            if assigned == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    impl Drop for KillOnCloseJob {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed only here.
            unsafe { CloseHandle(self.handle as HANDLE) };
        }
    }

    /// Puts a helper that was just started under the job. A helper that cannot join is still used:
    /// `kill_on_drop` and the helper's own deadline cover its usual ends.
    pub fn join(child: &tokio::process::Child) {
        let job = HELPERS.get_or_init(|| match KillOnCloseJob::new() {
            Ok(job) => Some(job),
            Err(error) => {
                warn!(%error, "helper job could not be created");
                None
            }
        });
        let (Some(job), Some(process)) = (job, child.raw_handle()) else {
            return;
        };
        if let Err(error) = job.assign(process) {
            warn!(%error, "helper process did not join its job");
        }
    }

    #[cfg(test)]
    mod tests {
        use std::os::windows::io::AsRawHandle;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        use super::KillOnCloseJob;

        #[test]
        fn closing_the_job_ends_the_processes_assigned_to_it() {
            let job = KillOnCloseJob::new().expect("the job is created");
            let mut child = Command::new("cmd")
                .args(["/C", "ping -n 60 127.0.0.1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("the process starts");
            job.assign(child.as_raw_handle())
                .expect("the process joins the job");
            // The last handle to the job closes here, so the process ends long before its 60 seconds.
            drop(job);
            let started = Instant::now();
            while child.try_wait().expect("the status is readable").is_none() {
                assert!(
                    started.elapsed() < Duration::from_secs(30),
                    "the process outlived its job"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Puts a helper that was just started under the Windows job. Elsewhere it does nothing.
#[cfg(windows)]
pub(super) fn join_helpers(child: &tokio::process::Child) {
    windows::join(child);
}

#[cfg(not(windows))]
pub(super) fn join_helpers(_child: &tokio::process::Child) {}

/// Stops a helper from opening a console window of its own. A release build on Windows has no
/// console, so Windows would open one for each console program the app starts. Elsewhere it does
/// nothing.
#[cfg(windows)]
pub(super) fn hide_console_window(command: &mut tokio::process::Command) {
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(super) fn hide_console_window(_command: &mut tokio::process::Command) {}

/// The largest answer read from a helper; a real one is a few kilobytes.
const MAX_HELPER_OUTPUT_BYTES: u64 = 1024 * 1024;

/// What a helper printed and how it ended. The scripts the helpers run come from the network, so
/// their output is read up to `MAX_HELPER_OUTPUT_BYTES`; a helper that prints more is left to the
/// caller's timeout.
pub(super) async fn bounded_output(
    child: &mut tokio::process::Child,
) -> std::io::Result<std::process::Output> {
    use tokio::io::AsyncReadExt;
    let mut stdout = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        pipe.take(MAX_HELPER_OUTPUT_BYTES)
            .read_to_end(&mut stdout)
            .await?;
    }
    let status = child.wait().await?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn a_helper_that_prints_too_much_is_read_up_to_the_limit() {
        // Twice the limit. The pipe is closed once the limit is read, so the process ends either
        // way: it finishes writing first or it is stopped by the closed pipe, depending on the
        // system and the timing. How it ended is not what is tested.
        let mut child = tokio::process::Command::new("head")
            .args([
                "-c",
                &(super::MAX_HELPER_OUTPUT_BYTES * 2).to_string(),
                "/dev/zero",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = super::bounded_output(&mut child).await.unwrap();
        assert_eq!(output.stdout.len() as u64, super::MAX_HELPER_OUTPUT_BYTES);
    }
}
